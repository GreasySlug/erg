use std::fmt::Write as _;
use std::fs::File;
use std::io::Write;
use std::sync::OnceLock;

use erg_common::error::MultiErrorDisplay;
use erg_common::log;
use erg_common::set::Set as HashSet;
use erg_common::traits::{ExitStatus, Locational, New, Runnable, Stream};
use erg_common::Str;
use erg_common::{config::ErgConfig, dict};
use erg_common::{config::TranspileTarget, dict::Dict as HashMap};

use erg_parser::ast::{ParamPattern, TypeSpec, VarName, AST};
use erg_parser::token::TokenKind;

use crate::artifact::{
    BuildRunnable, Buildable, CompleteArtifact, ErrorArtifact, IncompleteArtifact,
};
use crate::build_package::PackageBuilder;
use crate::codegen::PyCodeGenerator;
use crate::context::{Context, ContextProvider, ModuleContext};
use crate::desugar_hir::HIRDesugarer;
use crate::error::{CompileError, CompileErrors, CompileResult};
use crate::hir::{
    Accessor, Args, BinOp, Block, Call, ClassDef, Def, Dict, Expr, Identifier, Lambda, List,
    Literal, Params, PatchDef, ReDef, Record, Set, Signature, Tuple, UnaryOp, HIR,
};
use crate::link_hir::HIRLinker;
use crate::module::SharedCompilerResource;
use crate::ty::typaram::OpKind;
use crate::ty::value::ValueObj;
use crate::ty::{Field, HasType, Type, VisibilityModifier};
use crate::varinfo::{AbsLocation, VarInfo};

/// patch method -> function
/// patch attr -> variable
fn debind(ident: &Identifier) -> Option<Str> {
    match ident.vi.py_name.as_ref().map(|s| &s[..]) {
        Some(name) if name.starts_with("Function::") => {
            Some(Str::from(name.replace("Function::", "")))
        }
        Some(patch_method) if patch_method.contains("::") || patch_method.contains('.') => {
            if ident.vis().is_private() {
                Some(Str::from(format!("{patch_method}__")))
            } else {
                Some(Str::rc(patch_method))
            }
        }
        _ => None,
    }
}

fn demangle(name: &str) -> String {
    name.trim_start_matches("::<module>")
        .replace("::", "__")
        .replace('.', "_")
}

// TODO:
fn replace_non_symbolic(name: &str) -> String {
    let mut replaced = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '\'' => replaced.push_str("__single_quote__"),
            ' ' => replaced.push_str("__space__"),
            '+' => replaced.push_str("__plus__"),
            '-' => replaced.push_str("__minus__"),
            '*' => replaced.push_str("__star__"),
            '/' => replaced.push_str("__slash__"),
            '%' => replaced.push_str("__percent__"),
            '!' => replaced.push_str("__erg_proc__"),
            '$' => replaced.push_str("erg_shared__"),
            _ => replaced.push(c),
        }
    }
    replaced
}

fn push_indent(code: &mut String, level: usize) {
    for _ in 0..level {
        code.push_str("    ");
    }
}

/// Core runtime modules, in definition order: a module must appear after any
/// module whose classes its own class definitions inherit from
/// (e.g. `_erg_bool` defines `class Bool(Nat)`, so it comes after `_erg_nat`).
const CORE_MODULES: &[(&str, &str)] = &[
    ("_erg_result", include_str!("lib/core/_erg_result.py")),
    ("_erg_control", include_str!("lib/core/_erg_control.py")),
    ("_erg_type", include_str!("lib/core/_erg_type.py")),
    ("_erg_int", include_str!("lib/core/_erg_int.py")),
    ("_erg_nat", include_str!("lib/core/_erg_nat.py")),
    ("_erg_bool", include_str!("lib/core/_erg_bool.py")),
    ("_erg_str", include_str!("lib/core/_erg_str.py")),
    ("_erg_float", include_str!("lib/core/_erg_float.py")),
    ("_erg_range", include_str!("lib/core/_erg_range.py")),
    (
        "_erg_contains_operator",
        include_str!("lib/core/_erg_contains_operator.py"),
    ),
    (
        "_erg_mutate_operator",
        include_str!("lib/core/_erg_mutate_operator.py"),
    ),
    ("_erg_list", include_str!("lib/core/_erg_list.py")),
    ("_erg_dict", include_str!("lib/core/_erg_dict.py")),
    ("_erg_set", include_str!("lib/core/_erg_set.py")),
    ("_erg_bytes", include_str!("lib/core/_erg_bytes.py")),
    (
        "_erg_convertors",
        include_str!("lib/core/_erg_convertors.py"),
    ),
];

/// Modules required by each feature (transitive closure, in `CORE_MODULES` order)
const RANGE_OPS_MODULES: &[&str] = &[
    "_erg_result",
    "_erg_control",
    "_erg_type",
    "_erg_int",
    "_erg_nat",
    "_erg_str",
    "_erg_range",
];
const CONTAINS_OP_MODULES: &[&str] = &[
    "_erg_result",
    "_erg_type",
    "_erg_range",
    "_erg_contains_operator",
];
const BUILTIN_TYPES_MODULES: &[&str] = &[
    "_erg_result",
    "_erg_control",
    "_erg_type",
    "_erg_int",
    "_erg_nat",
    "_erg_bool",
    "_erg_str",
    "_erg_float",
    "_erg_range",
    "_erg_contains_operator",
    "_erg_list",
    "_erg_dict",
    "_erg_set",
    "_erg_bytes",
];
const CONVERTORS_MODULES: &[&str] = &[
    "_erg_result",
    "_erg_control",
    "_erg_type",
    "_erg_int",
    "_erg_nat",
    "_erg_str",
    "_erg_float",
    "_erg_convertors",
];

/// The core modules are inlined into a single prelude, so their cross-imports
/// must be removed.
fn strip_erg_imports(src: &str) -> String {
    let mut stripped = String::with_capacity(src.len());
    for line in src.lines() {
        if !line.trim_start().starts_with("from _erg") {
            stripped.push_str(line);
            stripped.push('\n');
        }
    }
    stripped
}

fn stripped_module_src(name: &str) -> &'static str {
    static STRIPPED: [OnceLock<String>; CORE_MODULES.len()] =
        [const { OnceLock::new() }; CORE_MODULES.len()];
    let idx = CORE_MODULES
        .iter()
        .position(|(mod_name, _)| *mod_name == name)
        .unwrap_or_else(|| unreachable!("unknown core module: {name}"));
    STRIPPED[idx].get_or_init(|| strip_erg_imports(CORE_MODULES[idx].1))
}

pub enum Enclosure {
    /// ()
    Paren,
    /// []
    Bracket,
    /// {}
    Brace,
    None,
}

impl Enclosure {
    pub const fn open(&self) -> char {
        match self {
            Enclosure::Paren => '(',
            Enclosure::Bracket => '[',
            Enclosure::Brace => '{',
            Enclosure::None => ' ',
        }
    }

    pub const fn close(&self) -> char {
        match self {
            Enclosure::Paren => ')',
            Enclosure::Bracket => ']',
            Enclosure::Brace => '}',
            Enclosure::None => ' ',
        }
    }
}

#[derive(Debug)]
pub enum LastLineOperation {
    Discard,
    Return,
    StoreTmp(Str),
}

use LastLineOperation::*;

impl LastLineOperation {
    pub const fn is_return(&self) -> bool {
        matches!(self, LastLineOperation::Return)
    }

    pub const fn is_store_tmp(&self) -> bool {
        matches!(self, LastLineOperation::StoreTmp(_))
    }
}

#[derive(Debug)]
pub enum TranspiledFile {
    PyScript(PyScript),
    Json(Json),
}

impl TranspiledFile {
    pub fn code(&self) -> &str {
        match self {
            Self::PyScript(script) => &script.code,
            Self::Json(json) => &json.code,
        }
    }

    pub fn into_code(self) -> String {
        match self {
            Self::PyScript(script) => script.code,
            Self::Json(json) => json.code,
        }
    }

    pub fn filename(&self) -> &str {
        match self {
            Self::PyScript(script) => &script.filename,
            Self::Json(json) => &json.filename,
        }
    }

    pub fn extension(&self) -> &str {
        match self {
            Self::PyScript(_) => "py",
            Self::Json(_) => "json",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PyScript {
    pub filename: Str,
    pub code: String,
}

#[derive(Debug, Clone)]
pub struct Json {
    pub filename: Str,
    pub code: String,
}

/// Generates a `PyScript` from an String or other File inputs.
#[derive(Debug)]
pub struct Transpiler {
    pub cfg: ErgConfig,
    builder: PackageBuilder,
    shared: SharedCompilerResource,
    script_generator: PyScriptGenerator,
}

impl Default for Transpiler {
    fn default() -> Self {
        Self::new(ErgConfig::default())
    }
}

impl New for Transpiler {
    fn new(cfg: ErgConfig) -> Self {
        let shared = SharedCompilerResource::new(cfg.copy());
        Self {
            shared: shared.clone(),
            builder: PackageBuilder::new_with_cache(cfg.copy(), "<module>".into(), shared),
            script_generator: PyScriptGenerator::new(),
            cfg,
        }
    }
}

impl Runnable for Transpiler {
    type Err = CompileError;
    type Errs = CompileErrors;
    const NAME: &'static str = "Erg transpiler";

    #[inline]
    fn cfg(&self) -> &ErgConfig {
        &self.cfg
    }
    #[inline]
    fn cfg_mut(&mut self) -> &mut ErgConfig {
        &mut self.cfg
    }

    #[inline]
    fn finish(&mut self) {}

    fn initialize(&mut self) {
        self.builder.initialize();
        // mod_cache will be cleared by the builder
        // self.mod_cache.initialize();
    }

    fn clear(&mut self) {
        self.builder.clear();
    }

    fn exec(&mut self) -> Result<ExitStatus, Self::Errs> {
        let mut path = self.cfg.dump_path();
        let src = self.cfg.input.read();
        let artifact = self.transpile(src, "exec").map_err(|eart| {
            eart.warns.write_all_stderr();
            eart.errors
        })?;
        artifact.warns.write_all_stderr();
        path.set_extension(artifact.object.extension());
        let mut f = File::create(path).unwrap();
        f.write_all(artifact.object.code().as_bytes()).unwrap();
        Ok(ExitStatus::compile_passed(artifact.warns.len()))
    }

    fn eval(&mut self, src: String) -> Result<String, CompileErrors> {
        let artifact = self.transpile(src, "eval").map_err(|eart| {
            eart.warns.write_all_stderr();
            eart.errors
        })?;
        artifact.warns.write_all_stderr();
        Ok(artifact.object.into_code())
    }

    fn completeness_checker(&self) -> Option<erg_common::stdin::CompletenessChecker> {
        Some(Box::new(erg_parser::parse::check_code_completeness))
    }
}

impl ContextProvider for Transpiler {
    fn dir(&self) -> HashMap<&VarName, &VarInfo> {
        self.builder.dir()
    }

    fn get_receiver_ctx(&self, receiver_name: &str) -> Option<&Context> {
        self.builder.get_receiver_ctx(receiver_name)
    }

    fn get_var_info(&self, name: &str) -> Option<(&VarName, &VarInfo)> {
        self.builder.get_var_info(name)
    }
}

impl Buildable<TranspiledFile> for Transpiler {
    fn inherit(cfg: ErgConfig, shared: SharedCompilerResource) -> Self {
        let mod_name = Str::from(cfg.input.file_stem());
        Self::new_with_cache(cfg, mod_name, shared)
    }
    fn inherit_with_name(cfg: ErgConfig, mod_name: Str, shared: SharedCompilerResource) -> Self {
        Self::new_with_cache(cfg, mod_name, shared)
    }
    fn build(
        &mut self,
        src: String,
        mode: &str,
    ) -> Result<CompleteArtifact<TranspiledFile>, IncompleteArtifact<TranspiledFile>> {
        self.transpile(src, mode)
            .map_err(|err| IncompleteArtifact::new(None, err.errors, err.warns))
    }
    fn build_from_ast(
        &mut self,
        ast: AST,
        mode: &str,
    ) -> Result<CompleteArtifact<TranspiledFile>, IncompleteArtifact<TranspiledFile>> {
        self.transpile_from_ast(ast, mode)
            .map_err(|err| IncompleteArtifact::new(None, err.errors, err.warns))
    }
    fn pop_context(&mut self) -> Option<ModuleContext> {
        self.builder.pop_context()
    }
    fn get_context(&self) -> Option<&ModuleContext> {
        self.builder.get_context()
    }
}

impl BuildRunnable<TranspiledFile> for Transpiler {}

impl Transpiler {
    pub fn new(cfg: ErgConfig) -> Self {
        New::new(cfg)
    }

    pub fn new_with_cache(cfg: ErgConfig, mod_name: Str, shared: SharedCompilerResource) -> Self {
        Self {
            shared: shared.clone(),
            builder: PackageBuilder::new_with_cache(cfg.copy(), mod_name, shared),
            script_generator: PyScriptGenerator::new(),
            cfg,
        }
    }

    pub fn transpile(
        &mut self,
        src: String,
        mode: &str,
    ) -> Result<CompleteArtifact<TranspiledFile>, ErrorArtifact> {
        log!(info "the transpiling process has started.");
        let artifact = self.build_link_desugar(src, mode)?;
        let file = self.lower(artifact.object)?;
        log!(info "code:\n{}", file.code());
        log!(info "the transpiling process has completed");
        Ok(CompleteArtifact::new(file, artifact.warns))
    }

    pub fn transpile_from_ast(
        &mut self,
        ast: AST,
        mode: &str,
    ) -> Result<CompleteArtifact<TranspiledFile>, ErrorArtifact> {
        log!(info "the transpiling process has started.");
        let artifact = self.builder.build_from_ast(ast, mode)?;
        let file = self.lower(artifact.object)?;
        log!(info "code:\n{}", file.code());
        log!(info "the transpiling process has completed");
        Ok(CompleteArtifact::new(file, artifact.warns))
    }

    fn lower(&mut self, hir: HIR) -> CompileResult<TranspiledFile> {
        match self.cfg.transpile_target {
            Some(TranspileTarget::Json) => {
                let mut gen = JsonGenerator::new(self.cfg.copy());
                Ok(TranspiledFile::Json(gen.transpile(hir)?))
            }
            _ => Ok(TranspiledFile::PyScript(
                self.script_generator.transpile(hir),
            )),
        }
    }

    pub fn transpile_module(&mut self) -> Result<CompleteArtifact<TranspiledFile>, ErrorArtifact> {
        let src = self.cfg.input.read();
        self.transpile(src, "exec")
    }

    fn build_link_desugar(
        &mut self,
        src: String,
        mode: &str,
    ) -> Result<CompleteArtifact, ErrorArtifact> {
        let artifact = self.builder.build(src, mode)?;
        self.link_desugar(artifact)
    }

    fn link_desugar(
        &mut self,
        artifact: CompleteArtifact,
    ) -> Result<CompleteArtifact, ErrorArtifact> {
        let linker = HIRLinker::new(&self.cfg, &self.shared.mod_cache);
        let hir = linker.link(artifact.object);
        let desugared = HIRDesugarer::desugar(hir);
        Ok(CompleteArtifact::new(desugared, artifact.warns))
    }

    pub fn pop_mod_ctx(&mut self) -> Option<ModuleContext> {
        self.builder.pop_context()
    }

    pub fn dir(&mut self) -> HashMap<&VarName, &VarInfo> {
        ContextProvider::dir(self)
    }

    pub fn get_receiver_ctx(&self, receiver_name: &str) -> Option<&Context> {
        ContextProvider::get_receiver_ctx(self, receiver_name)
    }

    pub fn get_var_info(&self, name: &str) -> Option<(&VarName, &VarInfo)> {
        ContextProvider::get_var_info(self, name)
    }
}

#[derive(Debug, Default)]
pub struct PyScriptGenerator {
    globals: HashSet<String>,
    loaded_mods: HashSet<&'static str>,
    level: usize,
    fresh_var_n: usize,
    namedtuple_loaded: bool,
    prelude: String,
}

impl PyScriptGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn transpile(&mut self, hir: HIR) -> PyScript {
        let mut code = String::new();
        for chunk in hir.module.into_iter() {
            let start = code.len();
            self.write_expr(chunk, &mut code);
            if code.len() > start {
                code.push('\n');
            }
        }
        code = std::mem::take(&mut self.prelude) + &code;
        PyScript {
            filename: hir.name,
            code,
        }
    }

    fn load_namedtuple_if_not(&mut self) {
        if !self.namedtuple_loaded {
            self.prelude += "from collections import namedtuple as NamedTuple__\n";
            self.namedtuple_loaded = true;
        }
    }

    fn load_module_if_not(&mut self, name: &'static str) {
        if self.loaded_mods.insert(name) {
            self.prelude += stripped_module_src(name);
        }
    }

    fn load_modules_if_not(&mut self, names: &[&'static str]) {
        for name in names {
            self.load_module_if_not(name);
        }
    }

    // TODO: name escaping
    fn load_range_ops_if_not(&mut self) {
        self.load_modules_if_not(RANGE_OPS_MODULES);
    }

    fn load_contains_op_if_not(&mut self) {
        self.load_modules_if_not(CONTAINS_OP_MODULES);
    }

    fn load_mutate_op_if_not(&mut self) {
        self.load_module_if_not("_erg_mutate_operator");
    }

    fn load_builtin_types_if_not(&mut self) {
        self.load_modules_if_not(BUILTIN_TYPES_MODULES);
    }

    fn load_builtin_controls_if_not(&mut self) {
        self.load_module_if_not("_erg_control");
    }

    fn load_convertors_if_not(&mut self) {
        self.load_modules_if_not(CONVERTORS_MODULES);
    }

    fn write_escaped_str(s: &str, out: &mut String) {
        for c in s.chars() {
            match c {
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                // '\'' => out.push_str("\\'"),
                '\0' => out.push_str("\\0"),
                _ => out.push(c),
            }
        }
    }

    /// Builds the transpiled `expr` as a standalone string.
    /// Only for contexts that need a `String` value (e.g. `join`);
    /// prefer writing through [`Self::write_expr`] directly.
    fn expr_to_string(&mut self, expr: Expr) -> String {
        let mut code = String::new();
        self.write_expr(expr, &mut code);
        code
    }

    /// Writes the transpiled `expr` to `out`.
    /// Auxiliary definitions (tmp functions, imports, etc.) go to `self.prelude`.
    fn write_expr(&mut self, expr: Expr, out: &mut String) {
        match expr {
            Expr::Literal(lit) => self.write_lit(lit, out),
            Expr::Call(call) => self.write_call(call, out),
            Expr::BinOp(bin) => self.write_binop(bin, out),
            Expr::UnaryOp(unary) => self.write_unaryop(unary, out),
            Expr::List(list) => match list {
                List::Normal(lis) => {
                    self.load_builtin_types_if_not();
                    out.push_str("List([");
                    for elem in lis.elems.pos_args {
                        self.write_expr(elem.expr, out);
                        out.push(',');
                    }
                    out.push_str("])");
                }
                other => todo!("transpiling {other}"),
            },
            Expr::Set(set) => match set {
                Set::Normal(st) => {
                    self.load_builtin_types_if_not();
                    out.push_str("Set({");
                    for elem in st.elems.pos_args {
                        self.write_expr(elem.expr, out);
                        out.push(',');
                    }
                    out.push_str("})");
                }
                other => todo!("transpiling {other}"),
            },
            Expr::Record(rec) => self.write_record(rec, out),
            Expr::Tuple(tuple) => match tuple {
                Tuple::Normal(tup) => {
                    out.push('(');
                    for elem in tup.elems.pos_args {
                        self.write_expr(elem.expr, out);
                        out.push(',');
                    }
                    out.push(')');
                }
            },
            Expr::Dict(dict) => match dict {
                Dict::Normal(dic) => {
                    self.load_builtin_types_if_not();
                    out.push_str("Dict({");
                    for kv in dic.kvs {
                        out.push('(');
                        self.write_expr(kv.key, out);
                        out.push_str("): (");
                        self.write_expr(kv.value, out);
                        out.push_str("),");
                    }
                    out.push_str("})");
                }
                other => todo!("transpiling {other}"),
            },
            Expr::Accessor(acc) => self.write_acc(acc, out),
            Expr::Def(def) => self.write_def(def, out),
            Expr::Lambda(lambda) => self.write_lambda(lambda, out),
            Expr::ClassDef(classdef) => self.write_classdef(classdef, out),
            Expr::PatchDef(patchdef) => self.write_patchdef(patchdef, out),
            Expr::ReDef(redef) => self.write_attrdef(redef, out),
            // TODO:
            Expr::Compound(comp) => {
                for expr in comp.into_iter() {
                    let start = out.len();
                    self.write_expr(expr, out);
                    if out.len() > start {
                        out.push('\n');
                        push_indent(out, self.level);
                    }
                }
            }
            Expr::Import(acc) => {
                let full_name = acc
                    .qual_name()
                    .map_or(acc.show(), |s| s.replace(".__init__", ""));
                let root = PyCodeGenerator::get_root(&acc);
                self.prelude += &format!(
                    "{} = __import__(\"{full_name}\")\n",
                    Self::transpile_ident(root)
                );
            }
            Expr::TypeAsc(tasc) => self.write_expr(*tasc.expr, out),
            Expr::Code(_) => todo!("transpiling importing user-defined code"),
            Expr::Dummy(_) => {}
        }
    }

    fn write_lit(&mut self, lit: Literal, out: &mut String) {
        if matches!(
            &lit.value,
            ValueObj::Bool(_)
                | ValueObj::Int(_)
                | ValueObj::Nat(_)
                | ValueObj::Str(_)
                | ValueObj::Float(_)
        ) {
            self.load_builtin_types_if_not();
            write!(out, "{}(", lit.value.class()).unwrap();
            Self::write_escaped_str(&lit.token.content, out);
            out.push(')');
        } else {
            Self::write_escaped_str(&lit.token.content, out);
        }
    }

    fn write_record(&mut self, rec: Record, out: &mut String) {
        self.load_namedtuple_if_not();
        let mut attrs = "[".to_string();
        let mut values = "(".to_string();
        for mut attr in rec.attrs.into_iter() {
            attrs.push('\'');
            attrs += &Self::transpile_ident(attr.sig.into_ident());
            attrs += "',";
            if attr.body.block.len() > 1 {
                let name = format!("instant_block_{}__", self.fresh_var_n);
                self.fresh_var_n += 1;
                let mut instant = format!("def {name}():\n");
                self.write_block(attr.body.block, Return, &mut instant);
                self.prelude += &instant;
                values += &name;
                values += "(),";
            } else {
                let expr = attr.body.block.remove(0);
                self.write_expr(expr, &mut values);
                values.push(',');
            }
        }
        attrs += "]";
        values += ")";
        out.push_str("NamedTuple__('Record', ");
        out.push_str(&attrs);
        out.push(')');
        out.push_str(&values);
    }

    fn write_binop(&mut self, bin: BinOp, out: &mut String) {
        match bin.op.kind {
            TokenKind::Closed | TokenKind::LeftOpen | TokenKind::RightOpen | TokenKind::Open => {
                self.load_range_ops_if_not();
                out.push_str(match bin.op.kind {
                    TokenKind::Closed => "ClosedRange(",
                    TokenKind::LeftOpen => "LeftOpenRange(",
                    TokenKind::RightOpen => "RightOpenRange(",
                    TokenKind::Open => "OpenRange(",
                    _ => unreachable!(),
                });
                self.write_expr(*bin.lhs, out);
                out.push(',');
                self.write_expr(*bin.rhs, out);
                out.push(')');
            }
            TokenKind::ContainsOp => {
                self.load_contains_op_if_not();
                out.push_str("contains_operator(");
                self.write_expr(*bin.lhs, out);
                out.push(',');
                self.write_expr(*bin.rhs, out);
                out.push(')');
            }
            _ => {
                out.push('(');
                self.write_expr(*bin.lhs, out);
                out.push(' ');
                out.push_str(&bin.op.content);
                out.push(' ');
                self.write_expr(*bin.rhs, out);
                out.push(')');
            }
        }
    }

    fn write_unaryop(&mut self, unary: UnaryOp, out: &mut String) {
        if unary.op.kind == TokenKind::Try {
            // an early `return` cannot be expressed inside a Python expression
            todo!("transpiling the `?` operator")
        }
        if unary.op.kind == TokenKind::Mutate {
            self.load_mutate_op_if_not();
            out.push_str("mutate_operator(");
        } else {
            out.push('(');
            out.push_str(&unary.op.content);
        }
        self.write_expr(*unary.expr, out);
        out.push(')');
    }

    fn write_acc(&mut self, acc: Accessor, out: &mut String) {
        // class wrapper (e.g. `Nat(x)`) for accessors of builtin types
        let class = match acc.ref_t().derefine() {
            v @ (Type::Bool | Type::Nat | Type::Int | Type::Float | Type::Str) => {
                self.load_builtin_types_if_not();
                Some(v.qual_name())
            }
            other => {
                let name = other.qual_name();
                if matches!(&name[..], "Bytes" | "List" | "Dict" | "Set") {
                    self.load_builtin_types_if_not();
                    Some(name)
                } else {
                    None
                }
            }
        };
        match acc {
            Accessor::Ident(ident) => {
                match &ident.inspect()[..] {
                    "Str" | "Bytes" | "Bool" | "Nat" | "Int" | "Float" | "List" | "Dict"
                    | "Set" | "Str!" | "Bytes!" | "Bool!" | "Nat!" | "Int!" | "Float!"
                    | "List!" => {
                        self.load_builtin_types_if_not();
                    }
                    "if" | "if!" | "for!" | "while" | "discard" => {
                        self.load_builtin_controls_if_not();
                    }
                    "int" | "nat" | "float" | "str" => {
                        self.load_convertors_if_not();
                    }
                    _ => {}
                }
                if let Some(class) = &class {
                    out.push_str(class);
                    out.push('(');
                }
                out.push_str(&Self::transpile_ident(ident));
                if class.is_some() {
                    out.push(')');
                }
            }
            Accessor::Attr(attr) => {
                // the class wrapper is not applied to debinded patch methods
                if let Some(name) = debind(&attr.ident) {
                    out.push_str(&demangle(&name));
                } else {
                    if let Some(class) = &class {
                        out.push_str(class);
                        out.push('(');
                    }
                    out.push('(');
                    self.write_expr(*attr.obj, out);
                    out.push_str(").");
                    out.push_str(&Self::transpile_ident(attr.ident));
                    if class.is_some() {
                        out.push(')');
                    }
                }
            }
        }
    }

    fn write_call(&mut self, mut call: Call, out: &mut String) {
        match call.obj.local_name() {
            Some("assert") => {
                out.push_str("assert ");
                self.write_expr(call.args.remove(0), out);
                if let Some(msg) = call.args.try_remove(0) {
                    out.push_str(", ");
                    self.write_expr(msg, out);
                }
            }
            Some("not") => {
                out.push_str("(not (");
                self.write_expr(call.args.remove(0), out);
                out.push_str("))");
            }
            Some("if" | "if!") => self.write_if(call, out),
            Some("for" | "for!") => {
                out.push_str("for ");
                let iter = call.args.remove(0);
                let Expr::Lambda(block) = call.args.remove(0) else {
                    todo!()
                };
                let non_default = block.params.non_defaults.first().unwrap();
                let param_token = match &non_default.raw.pat {
                    ParamPattern::VarName(name) => name.token(),
                    ParamPattern::Discard(token) => token,
                    _ => unreachable!(),
                };
                out.push_str(&Self::transpile_name(
                    &VisibilityModifier::Private,
                    param_token.inspect(),
                    &non_default.vi,
                ));
                out.push_str(" in ");
                self.write_expr(iter, out);
                out.push_str(":\n");
                self.write_block(block.body, Discard, out);
            }
            Some("while" | "while!") => {
                out.push_str("while ");
                let Expr::Lambda(mut cond) = call.args.remove(0) else {
                    todo!()
                };
                let Expr::Lambda(block) = call.args.remove(0) else {
                    todo!()
                };
                self.write_expr(cond.body.remove(0), out);
                out.push_str(":\n");
                self.write_block(block.body, Discard, out);
            }
            Some("match" | "match!") => self.write_match(call, out),
            _ => self.write_simple_call(call, out),
        }
    }

    fn write_if(&mut self, mut call: Call, out: &mut String) {
        // the condition is written after the then-clause, but must be
        // transpiled first to keep the evaluation (side effect) order
        let cond = self.expr_to_string(call.args.remove(0));
        let Expr::Lambda(mut then_block) = call.args.remove(0) else {
            todo!()
        };
        let else_block = call.args.try_remove(0).map(|ex| {
            if let Expr::Lambda(blk) = ex {
                blk
            } else {
                todo!()
            }
        });
        if then_block.body.len() == 1
            && else_block
                .as_ref()
                .map(|blk| blk.body.len() == 1)
                .unwrap_or(true)
        {
            self.write_expr(then_block.body.remove(0), out);
            out.push_str(" if ");
            out.push_str(&cond);
            out.push_str(" else ");
            if let Some(mut else_block) = else_block {
                self.write_expr(else_block.body.remove(0), out);
            } else {
                out.push_str("None");
            }
            return;
        }
        let tmp = Str::from(format!("if_tmp_{}__", self.fresh_var_n));
        self.fresh_var_n += 1;
        let tmp_func = Str::from(format!("if_tmp_func_{}__", self.fresh_var_n));
        self.fresh_var_n += 1;
        let mut code = format!("def {tmp_func}():\n    if {cond}:\n");
        let level = self.level;
        self.level = 1;
        self.write_block(then_block.body, StoreTmp(tmp.clone()), &mut code);
        self.level = level;
        if let Some(else_block) = else_block {
            code += "    else:\n";
            let level = self.level;
            self.level = 1;
            self.write_block(else_block.body, StoreTmp(tmp.clone()), &mut code);
            self.level = level;
        } else {
            code += "    else:\n";
            code += &format!("        {tmp} = None\n");
        }
        code += &format!("    return {tmp}\n");
        self.prelude += &code;
        // ~~ NOTE: In Python, the variable environment of a function is determined at call time
        // This is a very bad design, but can be used for this code ~~
        // FIXME: this trick only works in the global namespace
        out.push_str(&tmp_func);
        out.push_str("()");
    }

    fn write_match(&mut self, mut call: Call, out: &mut String) {
        let tmp = Str::from(format!("match_tmp_{}__", self.fresh_var_n));
        self.fresh_var_n += 1;
        let tmp_func = Str::from(format!("match_tmp_func_{}__", self.fresh_var_n));
        self.fresh_var_n += 1;
        let mut code = format!("def {tmp_func}():\n");
        self.level += 1;
        push_indent(&mut code, self.level);
        code += "match ";
        let cond = call.args.remove(0);
        self.write_expr(cond, &mut code);
        code += ":\n";
        while let Some(Expr::Lambda(arm)) = call.args.try_remove(0) {
            self.level += 1;
            push_indent(&mut code, self.level);
            let target = arm.params.non_defaults.first().unwrap();
            match &target.raw.pat {
                ParamPattern::VarName(param) => {
                    let param = Self::transpile_name(
                        &VisibilityModifier::Private,
                        param.inspect(),
                        &target.vi,
                    );
                    match target.raw.t_spec.as_ref().map(|t| &t.t_spec) {
                        Some(TypeSpec::Enum(enum_t)) => {
                            let values = ValueObj::vec_from_const_args(enum_t.clone());
                            let patterns = values
                                .iter()
                                .map(|v| v.to_string())
                                .collect::<Vec<_>>()
                                .join(" | ");
                            code += &format!("case ({patterns}) as {param}:\n");
                        }
                        Some(other) => {
                            if let Some(Expr::Set(Set::Normal(set))) = &target.t_spec_as_expr {
                                let patterns = set
                                    .elems
                                    .pos_args
                                    .iter()
                                    .map(|elem| self.expr_to_string(elem.expr.clone()))
                                    .collect::<Vec<_>>()
                                    .join(" | ");
                                code += &format!("case ({patterns}) as {param}:\n");
                            } else {
                                todo!("{other}")
                            }
                        }
                        None => {
                            code += &format!("case {param}:\n");
                        }
                    }
                    self.write_block(arm.body, StoreTmp(tmp.clone()), &mut code);
                    self.level -= 1;
                }
                ParamPattern::Discard(_) => {
                    match target.raw.t_spec.as_ref().map(|t| &t.t_spec) {
                        Some(TypeSpec::Enum(enum_t)) => {
                            let values = ValueObj::vec_from_const_args(enum_t.clone());
                            let patterns = values
                                .iter()
                                .map(|v| v.to_string())
                                .collect::<Vec<_>>()
                                .join(" | ");
                            code += &format!("case {patterns}:\n");
                        }
                        Some(_) => todo!(),
                        None => {
                            code += "case _:\n";
                        }
                    }
                    self.write_block(arm.body, StoreTmp(tmp.clone()), &mut code);
                    self.level -= 1;
                }
                _ => todo!(),
            }
        }
        push_indent(&mut code, self.level);
        code += &format!("return {tmp}\n");
        self.prelude += &code;
        self.level -= 1;
        out.push_str(&tmp_func);
        out.push_str("()");
    }

    fn write_simple_call(&mut self, call: Call, out: &mut String) {
        let enc = if call.obj.ref_t().is_poly_meta_type() {
            Enclosure::Bracket
        } else {
            Enclosure::Paren
        };
        let is_py_api = if let Some(attr) = &call.attr_name {
            let is_py_api = attr.is_py_api();
            if let Some(name) = debind(attr) {
                out.push_str(&demangle(&name));
                out.push('(');
                self.write_expr(*call.obj, out);
                out.push_str(", ");
                self.write_args(call.args, is_py_api, enc, out);
                out.push(')');
                return;
            }
            is_py_api
        } else {
            call.obj.is_py_api()
        };
        out.push('(');
        self.write_expr(*call.obj, out);
        out.push(')');
        if let Some(attr) = call.attr_name {
            out.push('.');
            out.push_str(&Self::transpile_ident(attr));
        }
        self.write_args(call.args, is_py_api, enc, out);
    }

    fn write_args(&mut self, mut args: Args, is_py_api: bool, enc: Enclosure, out: &mut String) {
        out.push(enc.open());
        while let Some(arg) = args.try_remove_pos(0) {
            self.write_expr(arg.expr, out);
            out.push(',');
        }
        while let Some(arg) = args.try_remove_kw(0) {
            let escape = if is_py_api { "" } else { "__" };
            out.push_str(&arg.keyword.content);
            out.push_str(escape);
            out.push('=');
            self.write_expr(arg.expr, out);
            out.push(',');
        }
        out.push(enc.close());
    }

    fn transpile_ident(ident: Identifier) -> String {
        Self::transpile_name(ident.vis(), ident.inspect(), &ident.vi)
    }

    fn transpile_name(vis: &VisibilityModifier, name: &Str, vi: &VarInfo) -> String {
        if let Some(py_name) = &vi.py_name {
            return demangle(py_name);
        }
        let name = replace_non_symbolic(name);
        if vis.is_public() || &name == "_" {
            name.to_string()
        } else {
            let def_line = vi.def_loc.loc.ln_begin().unwrap_or(0);
            let def_col = vi.def_loc.loc.col_begin().unwrap_or(0);
            let line_mangling = match (def_line, def_col) {
                (0, 0) => "".to_string(),
                (0, _) => format!("_C{def_col}"),
                (_, 0) => format!("_L{def_line}"),
                (_, _) => format!("_L{def_line}_C{def_col}"),
            };
            format!("{name}{line_mangling}")
        }
    }

    fn write_params(&mut self, params: Params, out: &mut String) {
        for non_default in params.non_defaults {
            match non_default.raw.pat {
                ParamPattern::VarName(param) => {
                    out.push_str(&Self::transpile_name(
                        &VisibilityModifier::Private,
                        param.inspect(),
                        &non_default.vi,
                    ));
                    out.push(',');
                }
                ParamPattern::Discard(_) => {
                    write!(out, "_{},", self.fresh_var_n).unwrap();
                    self.fresh_var_n += 1;
                }
                _ => unreachable!(),
            }
        }
        for default in params.defaults {
            match default.sig.raw.pat {
                ParamPattern::VarName(param) => {
                    out.push_str(&Self::transpile_name(
                        &VisibilityModifier::Private,
                        param.inspect(),
                        &default.sig.vi,
                    ));
                    out.push_str(" = ");
                    self.write_expr(default.default_val, out);
                    out.push(',');
                }
                ParamPattern::Discard(_) => {
                    write!(out, "_{} = ", self.fresh_var_n).unwrap();
                    self.fresh_var_n += 1;
                    self.write_expr(default.default_val, out);
                    out.push(',');
                }
                _ => unreachable!(),
            }
        }
    }

    fn write_block(&mut self, block: Block, last_op: LastLineOperation, out: &mut String) {
        self.level += 1;
        let last = block.len().saturating_sub(1);
        for (i, chunk) in block.into_iter().enumerate() {
            push_indent(out, self.level);
            if i == last {
                match last_op {
                    Return => {
                        out.push_str("return ");
                    }
                    Discard => {}
                    StoreTmp(ref tmp) => {
                        out.push_str(tmp);
                        out.push_str(" = ");
                    }
                }
            }
            let start = out.len();
            self.write_expr(chunk, out);
            if out.len() > start {
                out.push('\n');
            }
        }
        self.level -= 1;
    }

    fn write_lambda(&mut self, lambda: Lambda, out: &mut String) {
        if lambda.body.len() > 1 {
            let name = format!("lambda_{}__", self.fresh_var_n);
            self.fresh_var_n += 1;
            let mut code = format!("def {name}(");
            self.write_params(lambda.params, &mut code);
            code += "):\n";
            self.write_block(lambda.body, Return, &mut code);
            self.prelude += &code;
            out.push_str(&name);
        } else {
            out.push_str("(lambda ");
            self.write_params(lambda.params, out);
            out.push(':');
            self.write_block(lambda.body, Discard, out);
            out.pop(); // \n
            out.push(')');
        }
    }

    // TODO: trait definition
    fn write_def(&mut self, mut def: Def, out: &mut String) {
        // HACK: allow reference to local variables in tmp functions
        if self.level > 0 {
            let ident = def.sig.ident();
            let name = Self::transpile_name(ident.vis(), ident.inspect(), &ident.vi);
            if !self.globals.contains(&name) {
                out.push_str("global ");
                out.push_str(&name);
                out.push('\n');
                push_indent(out, self.level);
                self.globals.insert(name);
            }
        }
        match def.sig {
            Signature::Var(var) => {
                out.push_str(&Self::transpile_ident(var.ident));
                out.push_str(" = ");
                if def.body.block.len() > 1 {
                    let name = format!("instant_block_{}__", self.fresh_var_n);
                    self.fresh_var_n += 1;
                    let mut instant = format!("def {name}():\n");
                    self.write_block(def.body.block, Return, &mut instant);
                    self.prelude += &instant;
                    out.push_str(&name);
                    out.push_str("()");
                } else {
                    let expr = def.body.block.remove(0);
                    self.write_expr(expr, out);
                }
            }
            Signature::Subr(subr) => {
                out.push_str("def ");
                out.push_str(&Self::transpile_ident(subr.ident));
                out.push('(');
                self.write_params(subr.params, out);
                out.push_str("):\n");
                self.write_block(def.body.block, Return, out);
            }
            Signature::Glob(_) => todo!(),
        }
    }

    fn write_classdef(&mut self, classdef: ClassDef, out: &mut String) {
        let class_name = Self::transpile_ident(classdef.sig.into_ident());
        writeln!(out, "class {class_name}():").unwrap();
        push_indent(out, self.level + 1);
        out.push_str("def __init__(self, param__):\n");
        match classdef.constructor.non_default_params().unwrap()[0].typ() {
            Type::Record(rec) => {
                for field in rec.keys() {
                    let vis = if field.vis.is_private() { "__" } else { "" };
                    push_indent(out, self.level + 2);
                    writeln!(
                        out,
                        "self.{}{vis} = param__.{}{vis}",
                        field.symbol, field.symbol
                    )
                    .unwrap();
                }
            }
            other => todo!("{other}"),
        }
        if classdef.need_to_gen_new {
            push_indent(out, self.level + 1);
            writeln!(out, "def new(x): return {class_name}.__call__(x)").unwrap();
        }
        let methods = ClassDef::take_all_methods(classdef.methods_list);
        self.write_block(methods, Discard, out);
    }

    fn write_patchdef(&mut self, patch_def: PatchDef, out: &mut String) {
        for chunk in patch_def.methods.into_iter() {
            let Expr::Def(mut def) = chunk else { todo!() };
            let name = format!(
                "{}{}",
                demangle(&patch_def.sig.ident().to_string_notype()),
                demangle(&def.sig.ident().to_string_notype()),
            );
            def.sig.ident_mut().raw.name = VarName::from_str(Str::from(name));
            push_indent(out, self.level);
            self.write_def(def, out);
            out.push('\n');
        }
    }

    fn write_attrdef(&mut self, mut redef: ReDef, out: &mut String) {
        self.write_expr(Expr::Accessor(redef.attr), out);
        out.push_str(" = ");
        if redef.block.len() > 1 {
            let name = format!("instant_block_{}__", self.fresh_var_n);
            self.fresh_var_n += 1;
            let mut instant = format!("def {name}():\n");
            self.write_block(redef.block, Return, &mut instant);
            self.prelude += &instant;
            out.push_str(&name);
            out.push_str("()");
        } else {
            let expr = redef.block.remove(0);
            self.write_expr(expr, out);
        }
    }
}

#[derive(Debug, Default)]
pub struct JsonGenerator {
    cfg: ErgConfig,
    binds: HashMap<AbsLocation, ValueObj>,
    errors: CompileErrors,
}

impl JsonGenerator {
    pub fn new(cfg: ErgConfig) -> Self {
        Self {
            cfg,
            binds: HashMap::new(),
            errors: CompileErrors::empty(),
        }
    }

    pub fn transpile(&mut self, hir: HIR) -> CompileResult<Json> {
        let mut code = "".to_string();
        let mut len = 0;
        for (i, chunk) in hir.module.into_iter().enumerate() {
            if i > 0 && len > 0 {
                code += ",\n";
            }
            let expr = self.transpile_expr(chunk);
            len = expr.len();
            code += &expr;
        }
        if self.errors.is_empty() {
            Ok(Json {
                filename: hir.name,
                code: format!("{{\n{code}\n}}"),
            })
        } else {
            Err(self.errors.take_all().into())
        }
    }

    fn expr_into_value(&self, expr: Expr) -> Option<ValueObj> {
        match expr {
            Expr::List(List::Normal(lis)) => {
                let mut vals = vec![];
                for elem in lis.elems.pos_args {
                    {
                        let val = self.expr_into_value(elem.expr)?;
                        vals.push(val);
                    }
                }
                Some(ValueObj::List(vals.into()))
            }
            Expr::List(List::WithLength(lis)) => {
                let len = lis
                    .len
                    .and_then(|len| self.expr_into_value(*len))
                    .and_then(|v| usize::try_from(&v).ok())?;
                let vals = vec![self.expr_into_value(*lis.elem)?; len];
                Some(ValueObj::List(vals.into()))
            }
            Expr::Tuple(Tuple::Normal(tup)) => {
                let mut vals = vec![];
                for elem in tup.elems.pos_args {
                    {
                        let val = self.expr_into_value(elem.expr)?;
                        vals.push(val);
                    }
                }
                Some(ValueObj::Tuple(vals.into()))
            }
            Expr::Dict(Dict::Normal(dic)) => {
                let mut kvs = dict! {};
                for kv in dic.kvs {
                    let key = self.expr_into_value(kv.key)?;
                    let val = self.expr_into_value(kv.value)?;
                    kvs.insert(key, val);
                }
                Some(ValueObj::Dict(kvs))
            }
            Expr::Record(rec) => {
                let mut attrs = dict! {};
                for mut attr in rec.attrs {
                    let field = Field::from(attr.sig.ident());
                    let val = self.expr_into_value(attr.body.block.remove(0))?;
                    attrs.insert(field, val);
                }
                Some(ValueObj::Record(attrs))
            }
            Expr::Literal(lit) => Some(lit.value),
            Expr::Accessor(acc) => self.binds.get(&acc.var_info().def_loc).cloned(),
            Expr::BinOp(bin) => {
                let lhs = self.expr_into_value(*bin.lhs)?;
                let rhs = self.expr_into_value(*bin.rhs)?;
                lhs.try_binary(rhs, OpKind::try_from(bin.op.kind).ok()?)
            }
            _ => None,
        }
    }

    fn transpile_def(&mut self, mut def: Def) -> String {
        self.register_def(&def);
        if def.sig.vis().is_public() {
            let expr = self.transpile_expr(def.body.block.remove(0));
            format!("\"{}\": {expr}", def.sig.inspect())
        } else {
            "".to_string()
        }
    }

    fn register_def(&mut self, def: &Def) {
        if let Some(val) = def
            .body
            .block
            .first()
            .cloned()
            .and_then(|expr| self.expr_into_value(expr))
        {
            self.binds.insert(def.sig.ident().vi.def_loc.clone(), val);
        }
    }

    fn transpile_expr(&mut self, expr: Expr) -> String {
        match expr {
            Expr::Literal(lit) => lit.token.content.to_string(),
            Expr::Accessor(acc) => {
                if let Some(val) = self.binds.get(&acc.var_info().def_loc) {
                    val.to_string()
                } else {
                    replace_non_symbolic(&acc.to_string())
                }
            }
            Expr::List(list) => match list {
                List::Normal(lis) => {
                    let mut code = "[".to_string();
                    for (i, elem) in lis.elems.pos_args.into_iter().enumerate() {
                        if i > 0 {
                            code += ", ";
                        }
                        code += &self.transpile_expr(elem.expr);
                    }
                    code += "]";
                    code
                }
                other => todo!("{other}"),
            },
            Expr::Tuple(tup) => match tup {
                Tuple::Normal(tup) => {
                    let mut code = "[".to_string();
                    for (i, elem) in tup.elems.pos_args.into_iter().enumerate() {
                        if i > 0 {
                            code += ", ";
                        }
                        code += &self.transpile_expr(elem.expr);
                    }
                    code += "]";
                    code
                }
            },
            Expr::Record(rec) => {
                let mut code = "".to_string();
                for (i, mut attr) in rec.attrs.into_iter().enumerate() {
                    if i > 0 {
                        code += ", ";
                    }
                    let expr = self.transpile_expr(attr.body.block.remove(0));
                    code += &format!("\"{}\": {expr}", attr.sig.inspect());
                }
                format!("{{{code}}}")
            }
            Expr::Dict(dic) => match dic {
                Dict::Normal(dic) => {
                    let mut code = "".to_string();
                    for (i, kv) in dic.kvs.into_iter().enumerate() {
                        if i > 0 {
                            code += ", ";
                        }
                        code += &format!(
                            "{}: {}",
                            self.transpile_expr(kv.key),
                            self.transpile_expr(kv.value)
                        );
                    }
                    format!("{{{code}}}")
                }
                Dict::Comprehension(other) => todo!("{other}"),
            },
            Expr::Def(def) => self.transpile_def(def),
            other => {
                let loc = other.loc();
                if let Some(val) = self.expr_into_value(other) {
                    val.to_string()
                } else {
                    self.errors.push(CompileError::not_const_expr(
                        self.cfg.input.clone(),
                        line!() as usize,
                        loc,
                        "".into(),
                    ));
                    "".to_string()
                }
            }
        }
    }
}
