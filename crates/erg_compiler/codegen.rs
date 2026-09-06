//! Generates `CodeObj` (equivalent to PyCodeObject of CPython) from `AST`.
//!
//! ASTからPythonバイトコード(コードオブジェクト)を生成する
//!
//! Version-specific opcodes are selected via [`erg_common::opcode_set::OpcodeSetVersion`].
//! Use `self.opcode_set` instead of branching on `self.py_version.minor` so that adding
//! new CPython versions (3.12, 3.13, 3.14, …) only requires updating `opcode_set.rs` and
//! adding `opcodeNNN.rs` in `erg_common`.
use std::collections::HashSet;
use std::fmt;
use std::process;

use erg_common::cache::CacheSet;
use erg_common::config::ErgConfig;
use erg_common::env::erg_core_path;
use erg_common::error::{ErrorDisplay, Location};
use erg_common::fresh::SharedFreshNameGenerator;
use erg_common::io::Input;
use erg_common::opcode::{CommonOpcode, CompareOp};
use erg_common::opcode_set::OpcodeSetVersion;
use erg_common::option_enum_unwrap;
use erg_common::python_util::{env_python_version, PythonVersion};
use erg_common::traits::{Locational, Stream};
use erg_common::Str;
use erg_common::{
    debug_power_assert, fmt_option, fn_name, fn_name_full, impl_stream, log, set,
    switch_unreachable,
};
use erg_parser::ast::VisModifierSpec;
use erg_parser::ast::{DefId, DefKind};
use CommonOpcode::*;

use erg_parser::ast::{ParamPattern, TypeBoundSpecs, VarName};
use erg_parser::token::DOT;
use erg_parser::token::EQUAL;
use erg_parser::token::{Token, TokenKind};

use crate::compile::{AccessKind, Name, StoreLoadKind};
use crate::context::ControlKind;
use crate::error::CompileError;
use crate::hir::DefaultParamSignature;
use crate::hir::GlobSignature;
use crate::hir::ListWithLength;
use crate::hir::Module;
use crate::hir::{
    Accessor, Args, Attribute, BinOp, Block, Call, ClassDef, Def, DefBody, Dict, Expr, GenNew,
    GuardClause, Identifier, Lambda, List, Literal, NonDefaultParamSignature, Params, PatchDef,
    PosArg, ReDef, Record, Set, Signature, SubrSignature, Tuple, UnaryOp, VarSignature, HIR,
};
use crate::ty::codeobj::{CodeObj, CodeObjFlags, MakeFunctionFlags};
use crate::ty::value::{GenTypeObj, ValueObj};
use crate::ty::SubrType;
use crate::ty::{HasType, Type, TypeCode, TypePair, VisibilityModifier};
use crate::varinfo::{AbsLocation, VarInfo};
use AccessKind::*;
use Type::*;

#[derive(Debug, Copy, Clone)]
pub enum RegisterNameKind {
    Import,
    Fast,
    NonFast,
}

use RegisterNameKind::*;

impl RegisterNameKind {
    pub const fn is_fast(&self) -> bool {
        matches!(self, Fast)
    }

    pub fn from_ident(ident: &Identifier) -> Self {
        if ident.vi.is_fast_value() {
            Fast
        } else {
            NonFast
        }
    }
}

/// patch method -> function
/// patch attr -> variable
fn debind(ident: &Identifier) -> Option<Str> {
    match ident.vi.py_name.as_ref().map(|s| &s[..]) {
        Some(name) if name.starts_with("Function::") => {
            Some(Str::from(name.replace("Function::", "")))
        }
        Some(patch_method) if patch_method.contains("::") || patch_method.contains('.') => {
            Some(Str::rc(patch_method))
        }
        _ => None,
    }
}

fn escape_name(
    name: &str,
    vis: &VisibilityModifier,
    def_line: u32,
    def_col: u32,
    is_attr: bool,
) -> Str {
    // Raw identifiers ('name' or 'name'!): use exact Python name without mangling
    if name.starts_with('\'') {
        let inner = name
            .trim_start_matches('\'')
            .trim_end_matches('!')
            .trim_end_matches('\'');
        return Str::rc(inner);
    }
    let name = name.replace('!', "__erg_proc__");
    let name = name.replace('$', "__erg_shared__");
    // For public APIs, mangling is not performed because `hasattr`, etc. cannot be used.
    // For automatically generated variables, there is no possibility of conflict.
    if vis.is_private() && !name.starts_with('%') {
        if is_attr {
            return Str::from(format!("::{name}"));
        }
        let line_mangling = match (def_line, def_col) {
            (0, 0) => "".to_string(),
            (0, _) => format!("_C{def_col}"),
            (_, 0) => format!("_L{def_line}"),
            (_, _) => format!("_L{def_line}_C{def_col}"),
        };
        Str::from(format!("::{name}{line_mangling}"))
    } else {
        Str::from(name)
    }
}

/// The `VarInfo` of each parameter, in [`PyCodeGenerator::gen_param_names`] order.
fn param_vis(params: &Params) -> impl Iterator<Item = &VarInfo> {
    params
        .non_defaults
        .iter()
        .map(|p| &p.vi)
        .chain(params.defaults.iter().map(|p| &p.sig.vi))
        .chain(params.var_params.iter().map(|p| &p.vi))
        .chain(params.kw_var_params.iter().map(|p| &p.vi))
}

/// Each parameter's name and `VarInfo`, in the same order.
fn param_names_vis(params: &Params) -> impl Iterator<Item = (Option<&Str>, &VarInfo)> {
    params
        .non_defaults
        .iter()
        .map(|p| (p.inspect(), &p.vi))
        .chain(params.defaults.iter().map(|p| (p.sig.inspect(), &p.sig.vi)))
        .chain(params.var_params.iter().map(|p| (p.inspect(), &p.vi)))
        .chain(params.kw_var_params.iter().map(|p| (p.inspect(), &p.vi)))
}

/// What the code generator has to know about closures before it emits
/// anything: the definitions some nested function closes over.
///
/// A variable a nested function reads lives in a cell, and the cell is the
/// frame's: made when the frame starts, held by every closure made in it, read
/// and written through by the frame itself. Lowering records what each
/// function captures on that function; this turns it around, into what the
/// scope binding the variable has in hand -- a frame finds what it has to
/// hold by looking its own definitions up here ([`HostFrame::cells`]).
#[derive(Debug, Default)]
struct Captures {
    defs: HashSet<AbsLocation>,
}

fn collect_captures(module: &Module) -> Captures {
    let mut caps = Captures::default();
    let mut walk = CaptureWalk {
        caps: &mut caps,
        into_functions: true,
    };
    // nothing read at module level is captured, so what its inlined blocks
    // read is not looked at
    let mut module_frame = HostFrame::default();
    for chunk in module.iter() {
        walk.expr(chunk, &mut module_frame);
    }
    caps
}

/// The frame of the function whose body is being walked.
///
/// A lambda codegen inlines (a branch of `if`, the body of `for!`, an arm of
/// `match`, ...) is no function at run time: its parameters and locals are
/// this frame's, what it reads from this frame is a plain local, and only
/// what it reads from further out is captured -- by this function. Lowering
/// records the reads on the inlined lambda without telling them apart, and
/// which is which is settled once the whole body is seen, as a nested
/// function is visible before its definition.
#[derive(Default)]
struct HostFrame {
    /// the frame's own definitions, with the name each has as a slot
    own: Vec<(AbsLocation, Str)>,
    read_inline: Vec<AbsLocation>,
}

impl HostFrame {
    fn owns(&self, loc: &AbsLocation) -> bool {
        self.own.iter().any(|(own, _)| own == loc)
    }

    fn define(&mut self, loc: &AbsLocation, name: Str) {
        if loc.loc != Location::Unknown && !self.owns(loc) {
            self.own.push((loc.clone(), name));
        }
    }

    fn define_params(&mut self, params: &Params) {
        for (name, vi) in param_names_vis(params) {
            // a parameter's slot is its bare name (see `escape_ident`)
            if let Some(name) = name.filter(|n| &n[..] != "_") {
                self.define(&vi.def_loc, name.clone());
            }
        }
    }

    fn read_inline(&mut self, captured: &[Identifier]) {
        for ident in captured {
            if ident.vi.def_loc.loc != Location::Unknown {
                self.read_inline.push(ident.vi.def_loc.clone());
            }
        }
    }

    /// What the frame's inlined blocks read from further out is captured --
    /// by this function.
    fn finish(self, caps: &mut Captures) {
        let Self { own, read_inline } = self;
        for loc in read_inline {
            if !own.iter().any(|(o, _)| o == &loc) {
                caps.defs.insert(loc);
            }
        }
    }

    /// The frame's own definitions that some nested function closes over:
    /// what it has to hold in cells, in definition order.
    fn cells(&self, caps: &Captures) -> Vec<Str> {
        self.own
            .iter()
            .filter(|(loc, _)| caps.defs.contains(loc))
            .map(|(_, name)| name.clone())
            .collect()
    }
}

/// Whether the body of a loop, written as a lambda, has to be a function of
/// its own at run time.
///
/// The body is called once per element, so a binding of its own -- its
/// parameter, a local -- is fresh on every iteration, and a function nested
/// in the body that closes over one keeps that iteration's value. Spliced
/// into the enclosing frame instead, the binding would be one slot which the
/// closures of every iteration share, each reading the last iteration's
/// value. (What the body reads from outside is not affected: that is the
/// enclosing frame's, whether the body is spliced in or called.)
pub(crate) fn loop_body_needs_frame(lambda: &Lambda) -> bool {
    let mut caps = Captures::default();
    let mut walk = CaptureWalk {
        caps: &mut caps,
        into_functions: true,
    };
    let frame = walk.frame(&lambda.params, &lambda.body);
    !frame.cells(&caps).is_empty()
}

/// The walk over the module, or over one function's body.
///
/// `into_functions`: whether a function met on the way is walked as a frame
/// of its own (the pass over the module) or only noted as a definition of
/// the frame being walked (a frame asking what it has to hold).
struct CaptureWalk<'c> {
    caps: &'c mut Captures,
    into_functions: bool,
}

impl CaptureWalk<'_> {
    /// A function of its own: what it reads from outside is captured, and
    /// its body is a frame.
    fn function(&mut self, params: &Params, body: &Block, captured: &[Identifier]) {
        if !self.into_functions {
            return;
        }
        for ident in captured {
            if ident.vi.def_loc.loc != Location::Unknown {
                self.caps.defs.insert(ident.vi.def_loc.clone());
            }
        }
        let frame = self.frame(params, body);
        frame.finish(self.caps);
    }

    /// A function's frame: its parameters and locals, those its inlined
    /// blocks bring in included.
    fn frame(&mut self, params: &Params, body: &Block) -> HostFrame {
        let mut frame = HostFrame::default();
        frame.define_params(params);
        self.params(params, &mut frame);
        for chunk in body.iter() {
            self.expr(chunk, &mut frame);
        }
        frame
    }

    fn params(&mut self, params: &Params, frame: &mut HostFrame) {
        for param in params.defaults.iter() {
            self.expr(&param.default_val, frame);
        }
        for guard in params.guards.iter() {
            match guard {
                GuardClause::Condition(cond) => self.expr(cond, frame),
                GuardClause::Bind(bind) => self.def(bind, frame),
            }
        }
    }

    /// `inlined`: codegen splices the body into the enclosing function
    fn lambda(&mut self, lambda: &Lambda, inlined: bool, frame: &mut HostFrame) {
        if inlined {
            frame.define_params(&lambda.params);
            frame.read_inline(&lambda.captured_names);
            self.params(&lambda.params, frame);
            for chunk in lambda.body.iter() {
                self.expr(chunk, frame);
            }
        } else {
            self.function(&lambda.params, &lambda.body, &lambda.captured_names);
        }
    }

    fn def(&mut self, def: &Def, frame: &mut HostFrame) {
        let ident = def.sig.ident();
        frame.define(&ident.vi.def_loc, escape_ident(ident.clone()));
        match &def.sig {
            Signature::Subr(sig) => {
                self.function(&sig.params, &def.body.block, &sig.captured_names)
            }
            _ => {
                for chunk in def.body.block.iter() {
                    self.expr(chunk, frame);
                }
            }
        }
    }

    fn call(&mut self, call: &Call, frame: &mut HostFrame) {
        // the same decision `emit_call_local` makes
        let inlined = match (call.obj.as_ref(), call.attr_name.as_ref()) {
            (Expr::Accessor(Accessor::Ident(ident)), None) if ident.vis().is_private() => {
                match &ident.inspect()[..] {
                    "if" | "if!" | "match" | "match!" | "with!" => true,
                    // a loop is inlined only when its body is written as a
                    // lambda, and the body does not need a frame of its own
                    "for" | "for!" | "while!" => matches!(
                        call.args.pos_args.get(1),
                        Some(arg) if matches!(&arg.expr, Expr::Lambda(l) if !loop_body_needs_frame(l))
                    ),
                    _ => false,
                }
            }
            _ => false,
        };
        self.expr(&call.obj, frame);
        for arg in call.args.iter() {
            match arg {
                Expr::Lambda(lambda) => self.lambda(lambda, inlined, frame),
                other => self.expr(other, frame),
            }
        }
    }

    fn acc(&mut self, acc: &Accessor, frame: &mut HostFrame) {
        if let Accessor::Attr(attr) = acc {
            self.expr(&attr.obj, frame);
        }
    }

    fn expr(&mut self, expr: &Expr, frame: &mut HostFrame) {
        match expr {
            Expr::Literal(_) | Expr::Import(_) => {}
            Expr::Accessor(acc) => self.acc(acc, frame),
            Expr::List(List::Normal(lis)) => {
                for elem in lis.elems.iter() {
                    self.expr(elem, frame);
                }
            }
            Expr::List(List::Comprehension(lis)) => {
                self.expr(&lis.elem, frame);
                self.expr(&lis.guard, frame);
            }
            Expr::List(List::WithLength(lis)) => {
                self.expr(&lis.elem, frame);
                if let Some(len) = lis.len.as_ref() {
                    self.expr(len, frame);
                }
            }
            Expr::Tuple(Tuple::Normal(tup)) => {
                for elem in tup.elems.iter() {
                    self.expr(elem, frame);
                }
            }
            Expr::Set(Set::Normal(set)) => {
                for elem in set.elems.iter() {
                    self.expr(elem, frame);
                }
            }
            Expr::Set(Set::WithLength(set)) => {
                self.expr(&set.elem, frame);
                self.expr(&set.len, frame);
            }
            Expr::Dict(Dict::Normal(dict)) => {
                for kv in dict.kvs.iter() {
                    self.expr(&kv.key, frame);
                    self.expr(&kv.value, frame);
                }
            }
            Expr::Dict(Dict::Comprehension(dict)) => {
                self.expr(&dict.key, frame);
                self.expr(&dict.value, frame);
                self.expr(&dict.guard, frame);
            }
            Expr::Record(record) => {
                for attr in record.attrs.iter() {
                    self.def(attr, frame);
                }
            }
            Expr::BinOp(bin) => {
                self.expr(&bin.lhs, frame);
                self.expr(&bin.rhs, frame);
            }
            Expr::UnaryOp(unary) => self.expr(&unary.expr, frame),
            Expr::Call(call) => self.call(call, frame),
            Expr::Lambda(lambda) => self.lambda(lambda, false, frame),
            Expr::Def(def) => self.def(def, frame),
            Expr::ClassDef(class) => {
                if let Some(sup) = class.require_or_sup.as_ref() {
                    self.expr(sup, frame);
                }
                for methods in class.methods_list.iter() {
                    for chunk in methods.defs.iter() {
                        self.expr(chunk, frame);
                    }
                }
            }
            Expr::PatchDef(patch) => {
                self.expr(&patch.base, frame);
                for chunk in patch.methods.iter() {
                    self.expr(chunk, frame);
                }
            }
            Expr::ReDef(redef) => {
                self.acc(&redef.attr, frame);
                for chunk in redef.block.iter() {
                    self.expr(chunk, frame);
                }
            }
            Expr::TypeAsc(asc) => self.expr(&asc.expr, frame),
            Expr::Code(block) | Expr::Compound(block) => {
                for chunk in block.iter() {
                    self.expr(chunk, frame);
                }
            }
            Expr::Dummy(dummy) => {
                for chunk in dummy.iter() {
                    self.expr(chunk, frame);
                }
            }
        }
    }
}

fn escape_ident(ident: Identifier) -> Str {
    let vis = ident.vis();
    if &ident.inspect()[..] == "Self" {
        // reference the self type or the self type constructor
        let ty = ident
            .vi
            .t
            .singleton_value()
            .and_then(|tp| <&Type>::try_from(tp).ok())
            .or_else(|| ident.vi.t.return_t())
            .unwrap();
        escape_name(
            &ty.local_name(),
            &ident.vi.vis.modifier,
            ident.vi.def_loc.loc.ln_begin().unwrap_or(0),
            ident.vi.def_loc.loc.col_begin().unwrap_or(0),
            ident.vi.kind.is_instance_attr(),
        )
    } else if let Some(py_name) = ident.vi.py_name {
        py_name
    } else if ident.vi.is_parameter() || ident.inspect() == "self" {
        // the name `gen_param_names` registered the parameter under: a `p!` or
        // `x$` is spelled `p__erg_proc__` / `x__erg_shared__` there, and a
        // reference spelled `p!` would not be found among the locals and would
        // become a `LOAD_NAME` of a global that does not exist
        escape_name(ident.inspect(), &VisibilityModifier::Public, 0, 0, false)
    } else {
        escape_name(
            ident.inspect(),
            vis,
            ident.vi.def_loc.loc.ln_begin().unwrap_or(0),
            ident.vi.def_loc.loc.col_begin().unwrap_or(0),
            ident.vi.kind.is_instance_attr(),
        )
    }
}

/// What a code unit is the body of. A store to a name binds a slot in a
/// function frame, and a name in a module or class body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitKind {
    Module,
    ClassBody,
    Function,
}

#[derive(Debug, Clone)]
pub struct PyCodeGenUnit {
    pub(crate) id: usize,
    pub(crate) py_version: PythonVersion,
    pub(crate) kind: UnitKind,
    pub(crate) codeobj: CodeObj,
    pub(crate) captured_vars: Vec<Str>,
    /// The locals whose slot holds a cell (3.11+): a local some nested function
    /// closes over is made a cell where it is bound, and is read and written
    /// through the cell from then on.
    pub(crate) cells: Vec<Str>,
    pub(crate) stack_len: u32, // the maximum stack size
    pub(crate) prev_lineno: u32,
    pub(crate) lasti: usize,
    pub(crate) prev_lasti: usize,
    pub(crate) _refs: Vec<ValueObj>, // ref-counted objects
}

impl PartialEq for PyCodeGenUnit {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl fmt::Display for PyCodeGenUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CompilerUnit{{\nid: {}\ncode:\n{}\n}}",
            self.id,
            self.codeobj.code_info(Some(self.py_version))
        )
    }
}

impl PyCodeGenUnit {
    #[allow(clippy::too_many_arguments)]
    pub fn new<S: Into<Str>, T: Into<Str>>(
        id: usize,
        py_version: PythonVersion,
        params: Vec<Str>,
        kwonlyargcount: u32,
        filename: S,
        name: T,
        firstlineno: u32,
        flags: u32,
    ) -> Self {
        Self {
            id,
            py_version,
            kind: UnitKind::Function,
            codeobj: CodeObj::empty(params, kwonlyargcount, filename, name, firstlineno, flags),
            captured_vars: vec![],
            cells: vec![],
            stack_len: 0,
            prev_lineno: firstlineno,
            lasti: 0,
            prev_lasti: 0,
            _refs: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PyCodeGenStack(Vec<PyCodeGenUnit>);

impl_stream!(PyCodeGenStack, PyCodeGenUnit);

#[derive(Debug, Default)]
pub struct PyCodeGenerator {
    pub(crate) cfg: ErgConfig,
    pub(crate) py_version: PythonVersion,
    pub(crate) opcode_set: OpcodeSetVersion,
    str_cache: CacheSet<str>,
    prelude_loaded: bool,
    mutate_op_loaded: bool,
    contains_op_loaded: bool,
    subtype_op_loaded: bool,
    record_type_loaded: bool,
    module_type_loaded: bool,
    control_loaded: bool,
    convertors_loaded: bool,
    traits_loaded: bool,
    operators_loaded: bool,
    union_loaded: bool,
    fake_generic_loaded: bool,
    abc_loaded: bool,
    builtins_loaded: bool,
    true_div_loaded: bool,
    err_ops_loaded: bool,
    unit_size: usize,
    units: PyCodeGenStack,
    /// What is closed over, collected from the whole module before anything
    /// is emitted. See [`collect_captures`].
    captures: Captures,
    pub(crate) fresh_gen: SharedFreshNameGenerator,
}

impl PyCodeGenerator {
    pub fn new(cfg: ErgConfig) -> Self {
        let py_version = cfg.target_version.unwrap_or_else(|| {
            let Some(version) = env_python_version() else {
                panic!("Failed to get python version");
            };
            version
        });
        let opcode_set = OpcodeSetVersion::from_python_version(py_version);
        Self {
            py_version,
            opcode_set,
            cfg,
            str_cache: CacheSet::new(),
            prelude_loaded: false,
            mutate_op_loaded: false,
            contains_op_loaded: false,
            subtype_op_loaded: false,
            record_type_loaded: false,
            module_type_loaded: false,
            control_loaded: false,
            convertors_loaded: false,
            traits_loaded: false,
            operators_loaded: false,
            union_loaded: false,
            fake_generic_loaded: false,
            abc_loaded: false,
            builtins_loaded: false,
            true_div_loaded: false,
            err_ops_loaded: false,
            unit_size: 0,
            units: PyCodeGenStack::empty(),
            captures: Captures::default(),
            fresh_gen: SharedFreshNameGenerator::new("codegen"),
        }
    }

    pub fn inherit(&self) -> Self {
        Self {
            cfg: self.cfg.clone(),
            py_version: self.py_version,
            opcode_set: self.opcode_set,
            str_cache: self.str_cache.clone(),
            prelude_loaded: false,
            mutate_op_loaded: false,
            contains_op_loaded: false,
            subtype_op_loaded: false,
            record_type_loaded: false,
            module_type_loaded: false,
            control_loaded: false,
            convertors_loaded: false,
            traits_loaded: false,
            operators_loaded: false,
            union_loaded: false,
            fake_generic_loaded: false,
            abc_loaded: false,
            builtins_loaded: false,
            true_div_loaded: false,
            err_ops_loaded: false,
            unit_size: 0,
            units: PyCodeGenStack::empty(),
            captures: Captures::default(),
            fresh_gen: self.fresh_gen.clone(),
        }
    }

    pub fn clear(&mut self) {
        self.units.clear();
    }

    pub fn set_input(&mut self, input: Input) {
        self.cfg.input = input;
    }

    pub fn initialize(&mut self) {
        self.prelude_loaded = false;
        self.mutate_op_loaded = false;
        self.contains_op_loaded = false;
        self.subtype_op_loaded = false;
        self.record_type_loaded = false;
        self.module_type_loaded = false;
        self.control_loaded = false;
        self.convertors_loaded = false;
        self.traits_loaded = false;
        self.operators_loaded = false;
        self.union_loaded = false;
        self.fake_generic_loaded = false;
        self.abc_loaded = false;
        self.builtins_loaded = false;
        self.true_div_loaded = false;
        self.err_ops_loaded = false;
    }

    #[inline]
    fn input(&self) -> &Input {
        &self.cfg.input
    }

    fn get_cached(&self, s: &str) -> Str {
        self.str_cache.get(s)
    }

    #[inline]
    fn toplevel_block(&self) -> &PyCodeGenUnit {
        self.units.first().unwrap()
    }

    #[inline]
    fn cur_block(&self) -> &PyCodeGenUnit {
        self.units.last().unwrap()
    }

    fn is_toplevel(&self) -> bool {
        self.cur_block() == self.toplevel_block()
    }

    fn captured_vars(&self) -> Vec<&str> {
        let mut caps = vec![];
        for unit in self.units.iter() {
            caps.extend(unit.captured_vars.iter().map(|s| &**s));
        }
        caps
    }

    #[inline]
    fn mut_cur_block(&mut self) -> &mut PyCodeGenUnit {
        self.units.last_mut().unwrap()
    }

    #[inline]
    fn cur_block_codeobj(&self) -> &CodeObj {
        &self.cur_block().codeobj
    }

    #[inline]
    fn mut_cur_block_codeobj(&mut self) -> &mut CodeObj {
        &mut self.mut_cur_block().codeobj
    }

    #[inline]
    fn toplevel_block_codeobj(&self) -> &CodeObj {
        &self.toplevel_block().codeobj
    }

    #[inline]
    fn stack_len(&self) -> u32 {
        self.cur_block().stack_len
    }

    #[inline]
    pub(crate) fn lasti(&self) -> usize {
        self.cur_block().lasti
    }

    #[inline]
    #[allow(dead_code)]
    fn debug_print(&mut self, value: impl Into<ValueObj>) {
        self.emit_load_const(value);
        self.emit_print_expr();
    }

    #[inline]
    #[allow(dead_code)]
    fn emit_print_expr(&mut self) {
        if self.opcode_set.is_3_12_plus() {
            self.write_instr(self.opcode_set.call_intrinsic_1());
            self.write_arg(1); // Intrinsic1::PrintExpr
        } else {
            self.write_instr(self.opcode_set.print_expr());
            self.write_arg(0);
        }
        self.stack_dec();
    }

    fn _emit_compare_op(&mut self, op: CompareOp) {
        self.write_instr(self.opcode_set.compare_op());
        self.write_arg(self.opcode_set.encode_compare_arg(op as usize));
        self.stack_dec();
        if self.opcode_set.is_3_11_plus() {
            let cache = self.opcode_set.cache_entries_compare_op() * 2;
            self.write_bytes(&vec![0; cache]); // CACHE
        }
    }

    /// shut down the interpreter
    #[allow(unused)]
    fn terminate(&mut self) {
        self.emit_push_null();
        self.emit_load_name_instr(Identifier::static_public("exit"));
        self.fixup_push_null_order();
        self.emit_load_const(1);
        self.emit_call_instr(1, Name);
        self.stack_dec();
    }

    /// swap TOS and TOS1
    #[allow(unused)]
    fn rot2(&mut self) {
        if self.opcode_set.is_3_11_plus() {
            self.write_instr(self.opcode_set.swap());
            self.write_arg(2);
        } else {
            self.write_instr(self.opcode_set.rot_two());
            self.write_arg(0);
        }
    }

    #[allow(unused)]
    fn dup_top(&mut self) {
        if self.opcode_set.is_3_11_plus() {
            self.write_instr(self.opcode_set.copy());
            self.write_arg(1);
        } else {
            self.write_instr(self.opcode_set.dup_top());
            self.write_arg(0);
        }
        self.stack_inc();
    }

    /// COPY(1) == DUP_TOP
    fn copy(&mut self, i: usize) {
        debug_power_assert!(i, >, 0);
        if self.opcode_set.is_3_11_plus() {
            self.write_instr(self.opcode_set.copy());
            self.write_arg(i);
        } else if i == 1 {
            self.write_instr(self.opcode_set.dup_top());
            self.write_arg(0);
        } else {
            todo!()
        }
        self.stack_inc();
    }

    /// 0 origin
    #[allow(dead_code)]
    fn peek_stack(&mut self, i: usize) {
        self.copy(i + 1);
        self.emit_print_expr();
    }

    pub(crate) fn fill_jump(&mut self, idx: usize, jump_to: usize) {
        let arg = jump_to / self.opcode_set.jump_unit_size();
        let bytes = u16::try_from(arg).unwrap().to_be_bytes();
        *self.mut_cur_block_codeobj().code.get_mut(idx).unwrap() = bytes[0];
        *self.mut_cur_block_codeobj().code.get_mut(idx + 2).unwrap() = bytes[1];
    }

    /// returns: shift bytes
    pub(crate) fn calc_edit_jump(&mut self, idx: usize, jump_to: usize) -> usize {
        let arg = jump_to / self.opcode_set.jump_unit_size();
        if idx == 0
            || !self
                .opcode_set
                .is_jump_op(*self.cur_block_codeobj().code.get(idx - 1).unwrap())
        {
            self.crash(&format!("calc_edit_jump: not jump op: {idx} {jump_to}"));
        }
        self.edit_code(idx, arg)
    }

    /// returns: shift bytes
    #[inline]
    pub(crate) fn edit_code(&mut self, idx: usize, arg: usize) -> usize {
        log!(err "editing: {idx} {arg}");
        match u8::try_from(arg) {
            Ok(u8code) => {
                *self.mut_cur_block_codeobj().code.get_mut(idx).unwrap() = u8code;
                0
            }
            Err(_e) => {
                // TODO: use u16 as long as possible
                // see write_arg's comment
                let bytes = u32::try_from(arg).unwrap().to_be_bytes();
                let before_instr = idx.saturating_sub(1);
                *self.mut_cur_block_codeobj().code.get_mut(idx).unwrap() = bytes[3];
                self.extend_arg(before_instr, &bytes)
            }
        }
    }

    // e.g. JUMP_ABSOLUTE 264, lasti: 100
    // 6 more instructions will be added after this, so 264 + 6 => 270
    // this is greater than u8::MAX, so we need to extend the arg
    // first, split `code + delta` into 4 u8s (as __Big__ endian)
    // 270.to_be_bytes() == [0, 0, 1, 14]
    // then, write the bytes in reverse order
    // [..., EXTENDED_ARG 0, EXTENDED_ARG 0, EXTENDED_ARG 1, JUMP_ABSOLUTE 14]
    /// returns: shift bytes
    #[inline]
    fn extend_arg(&mut self, before_instr: usize, bytes: &[u8]) -> usize {
        let mut shift_bytes = 0;
        let ext_arg = self.common_byte(CommonOpcode::EXTENDED_ARG);
        for byte in bytes.iter().rev().skip(1) {
            self.mut_cur_block_codeobj()
                .code
                .insert(before_instr, *byte);
            self.mut_cur_block_codeobj()
                .code
                .insert(before_instr, ext_arg);
            self.mut_cur_block().lasti += 2;
            shift_bytes += 2;
        }
        shift_bytes
    }

    pub(crate) fn write_instr<C: Into<u8>>(&mut self, code: C) {
        self.mut_cur_block_codeobj().code.push(code.into());
        self.mut_cur_block().lasti += 1;
    }

    /// Write a CommonOpcode, translating it to the target Python version's value.
    /// For 3.12 and below, this is identity. For 3.13+, opcodes are renumbered.
    pub(crate) fn write_opcode(&mut self, code: CommonOpcode) {
        let byte = self.opcode_set.translate_common(code as u8);
        self.mut_cur_block_codeobj().code.push(byte);
        self.mut_cur_block().lasti += 1;
    }

    /// Translate a CommonOpcode value to the target version's byte.
    /// Use this when comparing against bytecode or storing for later emission.
    fn common_byte(&self, code: CommonOpcode) -> u8 {
        self.opcode_set.translate_common(code as u8)
    }

    /// returns: shift bytes
    pub(crate) fn write_arg(&mut self, code: usize) -> usize {
        match u8::try_from(code) {
            Ok(u8code) => {
                self.mut_cur_block_codeobj().code.push(u8code);
                self.mut_cur_block().lasti += 1;
                1
            }
            Err(_) => {
                // The EXTENDED_ARGs about to be inserted push this instruction
                // further from where a *relative* jump was measured, so a jump
                // argument has to grow by as many bytes as they take. Only a
                // jump: adding it to, say, a name index would read the wrong
                // name, which is what `LOAD_NAME` did from 3.14 on -- its new
                // value, 93, is `JUMP_IF_FALSE_OR_POP` in the numbering
                // `CommonOpcode::is_jump_op` reads.
                let is_jump = self
                    .opcode_set
                    .is_jump_op(*self.cur_block_codeobj().code.last().unwrap());
                match u16::try_from(code) {
                    Ok(_) => {
                        let arg = code + if is_jump { 2 } else { 0 };
                        let bytes = u16::try_from(arg).unwrap().to_be_bytes(); // [u8; 2]
                        let before_instr = self.lasti().saturating_sub(1);
                        self.mut_cur_block_codeobj().code.push(bytes[1]);
                        self.mut_cur_block().lasti += 1;
                        self.extend_arg(before_instr, &bytes) + 1
                    }
                    Err(_) => {
                        let arg = code + if is_jump { 6 } else { 0 };
                        let bytes = u32::try_from(arg).unwrap().to_be_bytes(); // [u8; 4]
                        let before_instr = self.lasti().saturating_sub(1);
                        self.mut_cur_block_codeobj().code.push(bytes[3]);
                        self.mut_cur_block().lasti += 1;
                        self.extend_arg(before_instr, &bytes) + 1
                    }
                }
            }
        }
    }

    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        self.mut_cur_block_codeobj().code.extend_from_slice(bytes);
        self.mut_cur_block().lasti += bytes.len();
    }

    pub(crate) fn stack_inc(&mut self) {
        self.mut_cur_block().stack_len += 1;
        if self.stack_len() > self.cur_block_codeobj().stacksize {
            self.mut_cur_block_codeobj().stacksize = self.stack_len();
        }
    }

    pub(crate) fn stack_dec(&mut self) {
        if self.stack_len() == 0 {
            let lasti = self.lasti();
            let last = self.cur_block_codeobj().code.last().unwrap();
            self.crash(&format!(
                "the stack size becomes -1\nlasti: {lasti}\nlast code: {last}"
            ));
        } else {
            self.mut_cur_block().stack_len -= 1;
        }
    }

    /// NOTE: For example, an operation that increases the stack by 2 and decreases it by 1 should be `stack_inc_n(2); stack_dec();` not `stack_inc(1);`.
    /// This is because the stack size will not increase correctly.
    pub(crate) fn stack_inc_n(&mut self, n: usize) {
        self.mut_cur_block().stack_len += n as u32;
        if self.stack_len() > self.cur_block_codeobj().stacksize {
            self.mut_cur_block_codeobj().stacksize = self.stack_len();
        }
    }

    pub(crate) fn stack_dec_n(&mut self, n: usize) {
        if n as u32 > self.stack_len() {
            let lasti = self.lasti();
            let last = self.cur_block_codeobj().code.last().unwrap();
            self.crash(&format!(
                "the stack size becomes -1\nlasti: {lasti}\nlast code: {last}"
            ));
        } else {
            self.mut_cur_block().stack_len -= n as u32;
        }
    }

    pub(crate) fn emit_load_const<C: Into<ValueObj>>(&mut self, cons: C) {
        let value: ValueObj = cons.into();
        let idx = self
            .mut_cur_block_codeobj()
            .consts
            .iter()
            .position(|c| c == &value)
            .unwrap_or_else(|| {
                self.mut_cur_block_codeobj().consts.push(value);
                self.mut_cur_block_codeobj().consts.len() - 1
            });
        self.write_opcode(LOAD_CONST);
        self.write_arg(idx);
        self.stack_inc();
    }

    fn register_const<C: Into<ValueObj>>(&mut self, cons: C) -> usize {
        let value = cons.into();
        self.mut_cur_block_codeobj()
            .consts
            .iter()
            .position(|c| c == &value)
            .unwrap_or_else(|| {
                self.mut_cur_block_codeobj().consts.push(value);
                self.mut_cur_block_codeobj().consts.len() - 1
            })
    }

    fn local_search(&self, name: &str, acc_kind: AccessKind) -> Option<Name> {
        if !self.opcode_set.is_3_11_plus() {
            if let Some(idx) = self
                .cur_block_codeobj()
                .cellvars
                .iter()
                .position(|v| &**v == name)
            {
                return Some(Name::deref(idx));
            }
        }
        match acc_kind {
            AccessKind::Name => {
                if let Some(idx) = self
                    .cur_block_codeobj()
                    .freevars
                    .iter()
                    .position(|f| &**f == name)
                {
                    // in 3.11+ deref args index into the unified varnames (localsplus);
                    // before, into cellvars then freevars
                    let idx = if self.opcode_set.is_3_11_plus() {
                        self.cur_block_codeobj()
                            .varnames
                            .iter()
                            .position(|v| &**v == name)
                            .unwrap_or(idx)
                    } else {
                        self.cur_block_codeobj().cellvars.len() + idx
                    };
                    Some(Name::deref(idx))
                } else if self.opcode_set.is_3_11_plus()
                    && self.cur_block().cells.iter().any(|c| &**c == name)
                {
                    // a local that was made a cell where it was bound
                    let idx = self
                        .cur_block_codeobj()
                        .varnames
                        .iter()
                        .position(|v| &**v == name)
                        .expect("a cell is always a local");
                    Some(Name::deref(idx))
                } else if let Some(idx) = self
                    .cur_block_codeobj()
                    .varnames
                    .iter()
                    .position(|v| &**v == name)
                {
                    if self.captured_vars().contains(&name) {
                        None
                    } else {
                        Some(Name::fast(idx))
                    }
                } else if let Some(idx) = self
                    .cur_block_codeobj()
                    .names
                    .iter()
                    .position(|n| &**n == name)
                {
                    if !self.is_toplevel() {
                        None
                    } else {
                        Some(Name::local(idx))
                    }
                } else {
                    None
                }
            }
            _ => self
                .cur_block_codeobj()
                .names
                .iter()
                .position(|n| &**n == name)
                .map(Name::local),
        }
    }

    // local_searchで見つからなかった変数を探索する
    fn rec_search(&mut self, name: &str) -> Option<StoreLoadKind> {
        // search_name()を実行した後なのでcur_blockはskipする
        let mut found: Option<usize> = None;
        for (nth_from_toplevel, block) in self.units.iter().enumerate().rev().skip(1) {
            let block_is_toplevel = nth_from_toplevel == 0;
            if block.codeobj.cellvars.iter().any(|c| &**c == name) {
                found = Some(nth_from_toplevel);
                break;
            }
            // a variable that is passed through this block (already a freevar here)
            if !block_is_toplevel && block.codeobj.freevars.iter().any(|f| &**f == name) {
                found = Some(nth_from_toplevel);
                break;
            }
            if block.codeobj.varnames.iter().any(|v| &**v == name) {
                if block_is_toplevel {
                    return Some(StoreLoadKind::Global);
                } else {
                    // the outer scope variable
                    found = Some(nth_from_toplevel);
                    break;
                }
            }
            if block_is_toplevel && block.codeobj.names.iter().any(|n| &**n == name) {
                return Some(StoreLoadKind::Global);
            }
        }
        let Some(def_idx) = found else {
            // 見つからなかった変数(前方参照変数など)はグローバル
            return Some(StoreLoadKind::Global);
        };
        let cur_idx = self.units.len() - 1;
        let is_3_11_plus = self.opcode_set.is_3_11_plus();
        for (nth, block) in self.units.iter_mut().enumerate() {
            if nth == def_idx {
                if !block.codeobj.cellvars.iter().any(|c| &**c == name)
                    && !block.codeobj.freevars.iter().any(|f| &**f == name)
                {
                    block.codeobj.cellvars.push(Str::rc(name));
                }
            } else if def_idx < nth && nth < cur_idx {
                // A variable captured from a scope further out than the
                // immediate parent passes through every function in between as
                // a freevar of its own: that is where the function in between
                // takes the cell from when it builds the closure.
                if !block.codeobj.freevars.iter().any(|f| &**f == name)
                    && !block.codeobj.cellvars.iter().any(|c| &**c == name)
                {
                    block.codeobj.freevars.push(Str::rc(name));
                    if is_3_11_plus && !block.codeobj.varnames.iter().any(|v| &**v == name) {
                        // in 3.11 freevars are unified with varnames
                        block.codeobj.varnames.push(Str::rc(name));
                    }
                }
            }
        }
        Some(StoreLoadKind::Deref)
    }

    fn register_name(&mut self, name: Str, kind: RegisterNameKind) -> Name {
        let current_is_toplevel = self.is_toplevel();
        match self.rec_search(&name) {
            Some(st @ (StoreLoadKind::Local | StoreLoadKind::Global)) => {
                if kind.is_fast() {
                    self.mut_cur_block_codeobj().varnames.push(name);
                    Name::fast(self.cur_block_codeobj().varnames.len() - 1)
                } else {
                    let st = if current_is_toplevel {
                        StoreLoadKind::Local
                    } else {
                        st
                    };
                    self.mut_cur_block_codeobj().names.push(name);
                    Name::new(st, self.cur_block_codeobj().names.len() - 1)
                }
            }
            Some(StoreLoadKind::Deref) => {
                self.mut_cur_block_codeobj().freevars.push(name.clone());
                if self.opcode_set.is_3_11_plus() {
                    // in 3.11 freevars are unified with varnames
                    self.mut_cur_block_codeobj().varnames.push(name);
                    Name::deref(self.cur_block_codeobj().varnames.len() - 1)
                } else {
                    // cellvarsのpushはrec_search()で行われる
                    // a deref indexes cellvars then freevars
                    let codeobj = self.cur_block_codeobj();
                    Name::deref(codeobj.cellvars.len() + codeobj.freevars.len() - 1)
                }
            }
            None => {
                // new variable
                if current_is_toplevel {
                    self.mut_cur_block_codeobj().names.push(name);
                    Name::local(self.cur_block_codeobj().names.len() - 1)
                } else {
                    self.mut_cur_block_codeobj().varnames.push(name);
                    Name::fast(self.cur_block_codeobj().varnames.len() - 1)
                }
            }
            Some(_) => {
                switch_unreachable!()
            }
        }
    }

    fn register_attr(&mut self, name: Str) -> Name {
        self.mut_cur_block_codeobj().names.push(name);
        Name::local(self.cur_block_codeobj().names.len() - 1)
    }

    fn register_method(&mut self, name: Str) -> Name {
        self.mut_cur_block_codeobj().names.push(name);
        Name::local(self.cur_block_codeobj().names.len() - 1)
    }

    fn select_load_instr(&self, kind: StoreLoadKind, acc_kind: AccessKind) -> u8 {
        match kind {
            StoreLoadKind::Fast | StoreLoadKind::FastConst => self.common_byte(LOAD_FAST),
            StoreLoadKind::Global | StoreLoadKind::GlobalConst => self.common_byte(LOAD_NAME),
            StoreLoadKind::Deref | StoreLoadKind::DerefConst => self.opcode_set.load_deref(),
            StoreLoadKind::Local | StoreLoadKind::LocalConst => match acc_kind {
                Name => self.common_byte(LOAD_NAME),
                UnboundAttr => self.common_byte(LOAD_ATTR),
                // 3.12+: LOAD_METHOD merged into LOAD_ATTR (namei << 1 | 1)
                BoundAttr if self.opcode_set.has_load_method() => self.common_byte(LOAD_METHOD),
                BoundAttr => self.common_byte(LOAD_ATTR),
            },
        }
    }

    fn select_store_instr(&self, kind: StoreLoadKind, acc_kind: AccessKind) -> u8 {
        match kind {
            StoreLoadKind::Fast => self.common_byte(STORE_FAST),
            StoreLoadKind::FastConst => self.common_byte(STORE_FAST),
            // NOTE: First-time variables are treated as GLOBAL, but they are always first-time variables when assigned, so they are just NAME
            // NOTE: 初見の変数はGLOBAL扱いになるが、代入時は必ず初見であるので単なるNAME
            StoreLoadKind::Global | StoreLoadKind::GlobalConst => self.common_byte(STORE_NAME),
            StoreLoadKind::Deref | StoreLoadKind::DerefConst => self.opcode_set.store_deref(),
            StoreLoadKind::Local | StoreLoadKind::LocalConst => {
                match acc_kind {
                    Name => self.common_byte(STORE_NAME),
                    UnboundAttr => self.common_byte(STORE_ATTR),
                    // cannot overwrite methods directly
                    BoundAttr => self.common_byte(STORE_ATTR),
                }
            }
        }
    }

    pub(crate) fn emit_load_name_instr(&mut self, ident: Identifier) {
        log!(info "entered {}({ident})", fn_name!());
        if &ident.inspect()[..] == "#ModuleType" && !self.module_type_loaded {
            self.load_module_type();
            self.module_type_loaded = true;
        }
        let kind = RegisterNameKind::from_ident(&ident);
        let escaped = escape_ident(ident);
        match &escaped[..] {
            "if__" | "for__" | "while__" | "with__" | "discard__" | "assert__" => {
                self.load_control();
            }
            "int__" | "nat__" | "str__" | "float__" | "bool__" => {
                self.load_convertors();
            }
            "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow" | "eq" | "ne" | "lt"
            | "le" | "gt" | "ge" | "and_" | "or_" | "xor" | "lshift" | "rshift" | "pos" | "neg"
            | "invert" | "is_" | "is_not" | "call" => {
                self.load_operators();
            }
            "Eq" | "Ord" | "Hash" | "Add" | "Sub" | "Mul" | "Div" | "Pos" | "Neg" => {
                self.load_traits();
            }
            "CodeType" => {
                self.emit_global_import_items(
                    Identifier::static_public("types"),
                    vec![(Identifier::static_public("CodeType"), None)],
                );
            }
            // NoneType is not defined in the global scope, use `type(None)` instead
            "NoneType" => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::static_public("type"));
                self.fixup_push_null_order();
                let none = Expr::Literal(Literal::new(ValueObj::None, Token::DUMMY));
                let args = Args::single(PosArg::new(none));
                self.emit_args_311(args, AccessKind::Name);
                return;
            }
            "list_iterator" => {
                let list = Expr::Literal(Literal::new(ValueObj::List(vec![].into()), Token::DUMMY));
                let iter = Identifier::static_public("iter");
                let iter_call = iter.call(Args::single(PosArg::new(list)));
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(iter_call.into())));
                self.emit_call(typ_call);
                return;
            }
            "set_iterator" => {
                let set = Expr::Set(Set::empty());
                let iter = Identifier::static_public("iter");
                let iter_call = iter.call(Args::single(PosArg::new(set)));
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(iter_call.into())));
                self.emit_call(typ_call);
                return;
            }
            "dict_items" => {
                let dict = Expr::Dict(Dict::empty());
                let items = Identifier::static_public("iter");
                let items_call = items.call(Args::single(PosArg::new(dict)));
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(items_call.into())));
                self.emit_call(typ_call);
                return;
            }
            "dict_keys" => {
                let dict = Expr::Dict(Dict::empty());
                let keys = Identifier::static_public("keys");
                let keys_call = dict.method_call(keys, Args::empty());
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(keys_call.into())));
                self.emit_call(typ_call);
                return;
            }
            "dict_values" => {
                let dict = Expr::Dict(Dict::empty());
                let values = Identifier::static_public("values");
                let values_call = dict.method_call(values, Args::empty());
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(values_call.into())));
                self.emit_call(typ_call);
                return;
            }
            _ => {}
        }
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped, kind));
        let instr = self.select_load_instr(name.kind, Name);
        self.write_instr(instr);
        self.write_arg(name.idx);
        self.stack_inc();
        self.mut_cur_block_codeobj().stacksize += 2;
        if instr == self.common_byte(LOAD_GLOBAL) && self.opcode_set.is_3_11_plus() {
            let cache = self.opcode_set.cache_entries_load_global() * 2;
            self.write_bytes(&vec![0; cache]); // CACHE
        }
    }

    pub(crate) fn emit_load_global_instr(&mut self, ident: Identifier) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped, NonFast));
        let instr = LOAD_GLOBAL;
        self.write_opcode(instr);
        self.write_arg(name.idx);
        self.stack_inc();
    }

    fn emit_import_name_instr(&mut self, ident: Identifier, items_len: usize) {
        log!(info "entered {}({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped, Import));
        self.write_opcode(IMPORT_NAME);
        self.write_arg(name.idx);
        self.stack_inc_n(items_len);
        self.stack_dec(); // (level + from_list) -> module object
    }

    fn emit_import_from_instr(&mut self, ident: Identifier) {
        log!(info "entered {}", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped, Import));
        self.write_opcode(IMPORT_FROM);
        self.write_arg(name.idx);
        // self.stack_inc(); (module object) -> attribute
    }

    fn emit_import_all_instr(&mut self, ident: Identifier) {
        log!(info "entered {}", fn_name!());
        self.emit_load_const(0i32); // escaping to call access `Nat` before importing `Nat`
        self.emit_load_const([Str::ever("*")]);
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped, Import));
        self.write_opcode(IMPORT_NAME);
        self.write_arg(name.idx);
        self.stack_inc();
        if self.opcode_set.is_3_12_plus() {
            self.write_instr(self.opcode_set.call_intrinsic_1());
            self.write_arg(2); // Intrinsic1::ImportStar
                               // CALL_INTRINSIC_1(ImportStar) pushes None; must POP_TOP to clean up
            self.write_opcode(POP_TOP);
            self.write_arg(0);
        } else {
            self.write_opcode(IMPORT_STAR);
            self.write_arg(0);
        }
        self.stack_dec_n(3);
    }

    /// item: (name, renamed)
    fn emit_global_import_items(
        &mut self,
        module: Identifier,
        items: Vec<(Identifier, Option<Identifier>)>,
    ) {
        self.emit_load_const(0);
        let item_name_tuple = items
            .iter()
            .map(|ident| ValueObj::Str(ident.0.inspect().clone()))
            .collect::<Vec<_>>();
        let items_len = item_name_tuple.len();
        self.emit_load_const(item_name_tuple);
        self.emit_import_name_instr(module, items_len);
        for (item, renamed) in items.into_iter() {
            if let Some(renamed) = renamed {
                self.emit_import_from_instr(item);
                self.emit_store_global_instr(renamed);
            } else {
                self.emit_import_from_instr(item.clone());
                self.emit_store_global_instr(item);
            }
        }
        self.emit_pop_top(); // discard IMPORT_FROM object
    }

    fn emit_load_attr_instr(&mut self, ident: Identifier) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, UnboundAttr)
            .unwrap_or_else(|| self.register_attr(escaped));
        let instr = self.select_load_instr(name.kind, UnboundAttr);
        self.write_instr(instr);
        if instr == self.common_byte(LOAD_ATTR) && self.opcode_set.is_3_12_plus() {
            self.write_arg(name.idx * 2); // 3.12: LOAD_ATTR uses namei << 1 for attribute
            self.write_bytes(&[0; 18]); // 9 CACHE entries
        } else {
            self.write_arg(name.idx);
            if instr == self.common_byte(LOAD_ATTR) && self.opcode_set.is_3_11_plus() {
                self.write_bytes(&[0; 8]); // 4 CACHE entries
            }
        }
    }

    fn emit_load_method_instr(&mut self, ident: Identifier, acc_kind: AccessKind) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, acc_kind)
            .unwrap_or_else(|| self.register_method(escaped));
        let instr = self.select_load_instr(name.kind, acc_kind);
        self.write_instr(instr);
        if self.opcode_set.is_3_12_plus() {
            // 3.12: LOAD_ATTR replaces LOAD_METHOD; namei << 1 | 1 = method mode
            self.write_arg(name.idx * 2 + 1);
            self.stack_inc(); // pushes self + method
            self.write_bytes(&[0; 18]); // 9 CACHE entries
        } else {
            self.write_arg(name.idx);
            if self.opcode_set.is_3_11_plus() {
                self.stack_inc(); // instead of PUSH_NULL
                self.write_bytes(&[0; 20]); // 10 CACHE entries
            }
        }
    }

    /// How a store to `ident` binds it in this unit.
    ///
    /// A variable bound in a block codegen inlines -- an `if` branch, a loop
    /// body, a `match` arm -- is a local of the frame the block is inlined
    /// into. `is_fast_value` cannot say so: it looks at the scope lowering gave
    /// the variable, which is the block's own. Stored by name instead, the
    /// variable went to the module's globals -- a function frame without
    /// CO_OPTIMIZED uses them as its locals.
    fn store_kind(&self, ident: &Identifier) -> RegisterNameKind {
        if self.cur_block().kind == UnitKind::Function && ident.vi.is_control_block_local() {
            Fast
        } else {
            RegisterNameKind::from_ident(ident)
        }
    }

    /// How the parameter of an inlined block binds: it is a local of the frame
    /// the block is inlined into, and a name in a module or class body.
    fn block_param_kind(&self) -> RegisterNameKind {
        if self.cur_block().kind == UnitKind::Function {
            Fast
        } else {
            NonFast
        }
    }

    pub(crate) fn emit_store_instr(&mut self, ident: Identifier, acc_kind: AccessKind) {
        let kind = self.store_kind(&ident);
        let captured = self.captures.defs.contains(&ident.vi.def_loc);
        self.emit_store_instr_captured(ident, acc_kind, kind, captured)
    }

    /// `captured`: some nested function closes over this binding. Its cell is
    /// normally made when the frame starts (`emit_frame_cells`), and a store
    /// then goes through it; this is the fallback for a binding that was not
    /// known then, which becomes a cell here (3.11+; before, the frame makes
    /// the cells itself).
    fn emit_store_instr_captured(
        &mut self,
        ident: Identifier,
        acc_kind: AccessKind,
        kind: RegisterNameKind,
        captured: bool,
    ) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let mut name = self.local_search(&escaped, acc_kind).unwrap_or_else(|| {
            if acc_kind.is_local() {
                self.register_name(escaped, kind)
            } else {
                self.register_attr(escaped)
            }
        });
        if captured
            && self.opcode_set.is_3_11_plus()
            && matches!(name.kind, StoreLoadKind::Fast | StoreLoadKind::FastConst)
        {
            self.emit_make_cell(name.idx);
            name = Name::deref(name.idx);
        }
        let instr = self.select_store_instr(name.kind, acc_kind);
        self.write_instr(instr);
        self.write_arg(name.idx);
        self.stack_dec();
        if instr == self.common_byte(STORE_ATTR) {
            if self.opcode_set.is_3_11_plus() {
                self.write_bytes(&[0; 8]); // CACHE
            }
            self.stack_dec();
        } else if instr == self.common_byte(STORE_FAST) {
            self.mut_cur_block_codeobj().nlocals += 1;
        }
    }

    /// used for importing Erg builtin objects, etc. normally, this is not used
    // Ergの組み込みオブジェクトをimportするときなどに使う、通常は使わない
    fn emit_store_global_instr(&mut self, ident: Identifier) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped, NonFast));
        let instr = STORE_GLOBAL;
        self.write_opcode(instr);
        self.write_arg(name.idx);
        self.stack_dec();
    }

    /// Ergの文法として、属性への代入は存在しない(必ずオブジェクトはすべての属性を初期化しなくてはならないため)
    /// この関数はPythonへ落とし込むときに使う
    fn store_acc(&mut self, acc: Accessor) {
        log!(info "entered {} ({acc})", fn_name!());
        match acc {
            Accessor::Ident(ident) => {
                self.emit_store_instr(ident, Name);
            }
            Accessor::Attr(attr) => {
                self.emit_expr(*attr.obj);
                self.emit_store_instr(attr.ident, UnboundAttr);
            }
        }
    }

    pub(crate) fn emit_pop_top(&mut self) {
        self.write_opcode(POP_TOP);
        self.write_arg(0);
        self.stack_dec();
    }

    fn cancel_if_pop_top(&mut self) {
        if self.cur_block_codeobj().code.len() < 2 {
            return;
        }
        let lasop_t_idx = self.cur_block_codeobj().code.len() - 2;
        let pop_top_byte = self.common_byte(POP_TOP);
        if self.cur_block_codeobj().code.get(lasop_t_idx) == Some(&pop_top_byte) {
            self.mut_cur_block_codeobj().code.pop();
            self.mut_cur_block_codeobj().code.pop();
            self.mut_cur_block().lasti -= 2;
            self.stack_inc();
        }
    }

    /// Compileが継続不能になった際呼び出す
    /// 極力使わないこと
    #[track_caller]
    fn crash(&mut self, description: &str) -> ! {
        if cfg!(debug_assertions) || cfg!(feature = "debug") {
            println!("internal error: {description}");
            panic!("current block: {}", self.cur_block());
        } else {
            let err = CompileError::compiler_bug(
                0,
                self.input().clone(),
                Location::Unknown,
                fn_name!(),
                line!(),
            );
            err.write_to_stderr();
            process::exit(1);
        }
    }

    pub(crate) fn gen_param_names(&self, params: &Params) -> Vec<Str> {
        params
            .non_defaults
            .iter()
            .map(|p| (p.inspect().map(|s| &s[..]).unwrap_or("_"), &p.vi))
            .chain(
                params
                    .defaults
                    .iter()
                    .map(|p| (p.inspect().map(|s| &s[..]).unwrap_or("_"), &p.sig.vi)),
            )
            .chain(if let Some(var_args) = &params.var_params {
                vec![(
                    var_args.inspect().map(|s| &s[..]).unwrap_or("_"),
                    &var_args.vi,
                )]
            } else {
                vec![]
            })
            .chain(if let Some(kw_var_args) = &params.kw_var_params {
                vec![(
                    kw_var_args.inspect().map(|s| &s[..]).unwrap_or("_"),
                    &kw_var_args.vi,
                )]
            } else {
                vec![]
            })
            .enumerate()
            .map(|(i, (s, vi))| {
                if s == "_" {
                    format!("_{i}")
                } else {
                    escape_name(
                        s,
                        &VisibilityModifier::Public,
                        vi.def_loc.loc.ln_begin().unwrap_or(0),
                        vi.def_loc.loc.col_begin().unwrap_or(0),
                        false,
                    )
                    .to_string()
                }
            })
            .map(|s| self.get_cached(&s))
            .collect()
    }

    fn emit_acc(&mut self, acc: Accessor) {
        log!(info "entered {} ({acc})", fn_name!());
        let init_stack_len = self.stack_len();
        match acc {
            Accessor::Ident(ident) => {
                self.emit_load_name_instr(ident);
            }
            Accessor::Attr(mut a) => {
                // Python's namedtuple, a representation of Record, does not allow attribute names such as `::x`.
                // Since Erg does not allow the coexistence of private and public variables with the same name, there is no problem in this trick.
                let is_record = a.obj.ref_t().is_record();
                if is_record {
                    a.ident.raw.vis = VisModifierSpec::Public(Location::Unknown);
                }
                if let Some(varname) = debind(&a.ident) {
                    a.ident.raw.vis = VisModifierSpec::Private;
                    a.ident.raw.name = VarName::from_str(varname);
                    self.emit_load_name_instr(a.ident);
                } else {
                    self.emit_expr(*a.obj);
                    self.emit_load_attr_instr(a.ident);
                }
            }
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_def(&mut self, def: Def) {
        log!(info "entered {} ({})", fn_name!(), def.sig);
        if def.def_kind().is_trait() {
            return self.emit_trait_def(def);
        }
        match def.sig {
            Signature::Subr(sig) => self.emit_subr_def(None, sig, def.body),
            Signature::Var(sig) => self.emit_var_def(sig, def.body),
            Signature::Glob(sig) => self.emit_glob_def(sig, def.body),
        }
    }

    pub(crate) fn emit_push_null(&mut self) {
        if self.opcode_set.is_3_11_plus() {
            self.write_instr(self.opcode_set.push_null());
            self.write_arg(0);
            self.stack_inc();
        }
    }

    /// In 3.13+, CALL expects [callable, NULL, args...] instead of [NULL, callable, args...].
    /// Call this after loading the callable following an emit_push_null() to fix the order.
    fn fixup_push_null_order(&mut self) {
        if self.opcode_set.is_3_13_plus() {
            self.write_instr(self.opcode_set.swap());
            self.write_arg(2);
        }
    }

    /// Emit TO_BOOL + 3 CACHE entries before POP_JUMP_IF_FALSE/TRUE (3.13+).
    /// Returns the number of bytes written (0 or 8).
    pub(crate) fn emit_to_bool(&mut self) -> usize {
        let to_bool = self.opcode_set.to_bool();
        if to_bool != 0 {
            self.write_instr(to_bool);
            self.write_arg(0);
            let cache = self.opcode_set.cache_entries_to_bool() * 2; // 6 bytes
            self.write_bytes(&vec![0; cache]);
            2 + cache // 8 bytes total
        } else {
            0
        }
    }

    /// Emit CACHE entries after POP_JUMP_IF_FALSE/TRUE (1 entry for 3.13+).
    /// Returns the number of bytes written (0 or 2).
    pub(crate) fn emit_pop_jump_cache(&mut self) -> usize {
        let cache = self.opcode_set.cache_entries_pop_jump_if_false() * 2;
        if cache > 0 {
            self.write_bytes(&vec![0; cache]);
        }
        cache
    }

    /// Emit CACHE entries after JUMP_BACKWARD (1 entry for 3.13+).
    /// Returns the number of bytes written (0 or 2).
    fn emit_jump_backward_cache(&mut self) -> usize {
        let cache = self.opcode_set.cache_entries_jump_backward() * 2;
        if cache > 0 {
            self.write_bytes(&vec![0; cache]);
        }
        cache
    }

    pub(crate) fn emit_precall_and_call(&mut self, argc: usize) {
        if let Some(precall) = self.opcode_set.precall() {
            self.write_instr(precall);
            self.write_arg(argc);
            self.write_arg(0);
            self.write_arg(0);
        }
        self.write_instr(self.opcode_set.call());
        self.write_arg(argc);
        let cache = self.opcode_set.cache_entries_call() * 2;
        self.write_bytes(&vec![0; cache]); // CACHE
        self.stack_dec();
    }

    pub(crate) fn emit_call_instr(&mut self, argc: usize, kind: AccessKind) {
        if self.opcode_set.is_3_11_plus() {
            self.emit_precall_and_call(argc);
        } else {
            match kind {
                AccessKind::BoundAttr => self.write_instr(self.opcode_set.call_method()),
                _ => self.write_instr(self.opcode_set.call()),
            }
            self.write_arg(argc);
        }
    }

    pub(crate) fn emit_call_kw_instr(&mut self, argc: usize, kws: Vec<ValueObj>) {
        if self.opcode_set.is_3_13_plus() {
            // 3.13 removed KW_NAMES; push the names tuple and use CALL_KW
            self.emit_load_const(kws);
            self.write_instr(self.opcode_set.call_kw());
            self.write_arg(argc);
            let cache = self.opcode_set.cache_entries_call_kw() * 2;
            self.write_bytes(&vec![0; cache]); // CACHE
            self.stack_dec_n(2); // the kw names tuple (+ parity with `emit_precall_and_call`)
        } else if self.opcode_set.is_3_11_plus() {
            let idx = self.register_const(kws);
            self.write_instr(self.opcode_set.kw_names());
            self.write_arg(idx);
            self.emit_precall_and_call(argc);
        } else {
            self.emit_load_const(kws);
            self.write_instr(self.opcode_set.call_function_kw());
            self.write_arg(argc);
        }
    }

    fn emit_trait_def(&mut self, def: Def) {
        if !self.abc_loaded {
            self.load_abc();
            self.abc_loaded = true;
        }
        self.emit_push_null();
        self.write_opcode(LOAD_BUILD_CLASS);
        self.write_arg(0);
        self.stack_inc();
        self.fixup_push_null_order();
        let kind = def.def_kind();
        let code = self.emit_trait_block(kind, &def.sig, def.body.block);
        self.emit_load_const(code);
        if !self.opcode_set.is_3_11_plus() {
            self.emit_load_const(def.sig.ident().inspect().clone());
        } else {
            self.stack_inc();
        }
        self.write_opcode(MAKE_FUNCTION);
        self.write_arg(0);
        self.emit_load_const(def.sig.ident().inspect().clone());
        self.emit_load_name_instr(Identifier::private("#ABCMeta"));
        let subclasses_len = 1;
        self.emit_call_kw_instr(2 + subclasses_len, vec![ValueObj::from("metaclass")]);
        let sum = if self.opcode_set.is_3_11_plus() {
            1 + 2 + subclasses_len
        } else {
            1 + 2 + 1 + subclasses_len
        };
        self.stack_dec_n(sum - 1);
        self.emit_store_instr(def.sig.into_ident(), Name);
        self.stack_dec();
    }

    // trait variables will be removed
    // T = Trait {
    //     x = Int
    //     f = (self: Self) -> Int
    // }
    // ↓
    // class T(metaclass=ABCMeta):
    //    def f(): pass
    fn emit_trait_block(&mut self, kind: DefKind, sig: &Signature, mut block: Block) -> CodeObj {
        debug_assert!(kind.is_trait());
        let name = sig.ident().inspect().clone();
        let Expr::Call(mut trait_call) = block.remove(0) else {
            unreachable!()
        };
        // `T = Structural Trait {...}`: unwrap the `Structural` wrapper to reach the `Trait` call,
        // whose `Requirement` record holds the declared attributes.
        if kind.is_structural_trait() {
            if let Some(Expr::Call(inner)) = trait_call.args.remove_left_or_key("Type") {
                trait_call = inner;
            }
        }
        let req = if let Some(Expr::Record(req)) = trait_call.args.remove_left_or_key("Requirement")
        {
            req.attrs.into_iter()
        } else {
            vec![].into_iter()
        };
        self.unit_size += 1;
        let firstlineno = block
            .get(0)
            .and_then(|def| def.ln_begin())
            .unwrap_or_else(|| sig.ln_begin().unwrap());
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            vec![],
            0,
            Str::rc(self.cfg.input.enclosed_name()),
            &name,
            firstlineno,
            0,
        ));
        self.mut_cur_block().kind = UnitKind::ClassBody;
        let mod_name = self.toplevel_block_codeobj().name.clone();
        self.emit_load_const(mod_name);
        self.emit_store_instr(Identifier::static_public("__module__"), Name);
        self.emit_load_const(name);
        self.emit_store_instr(Identifier::static_public("__qualname__"), Name);
        for def in req {
            self.emit_empty_func(
                Some(sig.ident().inspect()),
                def.sig.into_ident(),
                Some(Identifier::private("#abstractmethod")),
            );
        }
        self.emit_load_const(ValueObj::None);
        self.write_opcode(RETURN_VALUE);
        self.write_arg(0);
        if self.stack_len() > 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit_trait_block: invalid stack size");
        }
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        self.remap_localsplus();
        // end of flagging
        let unit = self.units.pop().unwrap();
        if !self.units.is_empty() {
            let ld = unit.prev_lineno - self.cur_block().prev_lineno;
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += u8::try_from(ld).unwrap();
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        unit.codeobj
    }

    fn emit_empty_func(
        &mut self,
        class_name: Option<&str>,
        ident: Identifier,
        deco: Option<Identifier>,
    ) {
        log!(info "entered {} ({ident})", fn_name!());
        self.emit_push_null();
        let deco_is_some = deco.is_some();
        if let Some(deco) = deco {
            self.emit_load_name_instr(deco);
            self.fixup_push_null_order();
        }
        let code = {
            self.unit_size += 1;
            self.units.push(PyCodeGenUnit::new(
                self.unit_size,
                self.py_version,
                vec![],
                0,
                Str::rc(self.cfg.input.enclosed_name()),
                ident.inspect(),
                ident.ln_begin().unwrap_or(0),
                0,
            ));
            self.emit_load_const(ValueObj::None);
            self.write_opcode(RETURN_VALUE);
            self.write_arg(0);
            let unit = self.units.pop().unwrap();
            if !self.units.is_empty() {
                let ld = unit
                    .prev_lineno
                    .saturating_sub(self.cur_block().prev_lineno);
                if ld != 0 {
                    if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                        *l += u8::try_from(ld).unwrap();
                    }
                    self.mut_cur_block().prev_lineno += ld;
                }
            }
            unit.codeobj
        };
        self.emit_load_const(code);
        if !self.opcode_set.is_3_11_plus() {
            if let Some(class) = class_name {
                self.emit_load_const(Str::from(format!("{class}.{}", ident.inspect())));
            } else {
                self.emit_load_const(ident.inspect().clone());
            }
        } else {
            self.stack_inc();
        }
        self.write_opcode(MAKE_FUNCTION);
        self.write_arg(0);
        if deco_is_some {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
        // stack_dec: (<abstractmethod>) + <code obj> + <name> -> <function>
        self.stack_dec();
        self.emit_store_instr(ident, Name);
    }

    fn emit_class_def(&mut self, class_def: ClassDef) {
        log!(info "entered {} ({})", fn_name!(), class_def.sig);
        // For a polymorphic class, type application (e.g. `Box(Int)`) is emitted as a
        // subscript (`Box[Int]`), so the class must support `__class_getitem__`
        // (registered in `emit_class_block`; the runtime `GenericAlias` is imported here)
        if !class_def.obj.typ().is_monomorphic() && self.opcode_set.is_3_10_plus() {
            self.load_generic_alias();
        }
        self.emit_push_null();
        let ident = class_def.sig.ident().clone();
        let require_or_sup = class_def.require_or_sup.clone().map(|x| *x);
        let obj = *class_def.obj.clone();
        self.write_opcode(LOAD_BUILD_CLASS);
        self.write_arg(0);
        self.stack_inc();
        self.fixup_push_null_order();
        let code = self.emit_class_block(class_def);
        self.emit_load_const(code);
        if !self.opcode_set.is_3_11_plus() {
            self.emit_load_const(ident.inspect().clone());
        } else {
            self.stack_inc();
        }
        self.write_opcode(MAKE_FUNCTION);
        self.write_arg(0);
        self.emit_load_const(ident.inspect().clone());
        // LOAD subclasses
        let subclasses_len = self.emit_require_type(obj, require_or_sup);
        self.emit_call_instr(2 + subclasses_len, Name);
        self.stack_dec_n((1 + 2 + subclasses_len) - 1);
        self.emit_store_instr(ident, Name);
        self.stack_dec();
    }

    fn emit_patch_def(&mut self, patch_def: PatchDef) {
        log!(info "entered {} ({})", fn_name!(), patch_def.sig);
        for def in patch_def.methods {
            // Invert.
            //     invert self = ...
            // ↓
            // def Invert::invert(self): ...
            let Expr::Def(mut def) = def else { todo!() };
            let namespace = self.cur_block_codeobj().name.trim_start_matches("::");
            let name = format!(
                "{}{}{}",
                namespace,
                patch_def.sig.ident().to_string_notype(),
                def.sig.ident().to_string_notype()
            );
            def.sig.ident_mut().raw.name = VarName::from_str(Str::from(name));
            def.sig.ident_mut().raw.vis = VisModifierSpec::Private;
            self.emit_def(def);
        }
    }

    // NOTE: use `TypeVar`, `Generic` in `typing` module
    // fn emit_poly_type_def(&mut self, sig: SubrSignature, body: DefBody) {}

    /// Y = Inherit X => class Y(X): ...
    /// N = Inherit 1..10 => class N(Nat): ...
    fn emit_require_type(&mut self, obj: GenTypeObj, require_or_sup: Option<Expr>) -> usize {
        log!(info "entered {} ({obj}, {})", fn_name!(), fmt_option!(require_or_sup));
        match obj {
            GenTypeObj::Class(_) => 0,
            GenTypeObj::Subclass(typ) => {
                let require_or_sup = require_or_sup.unwrap();
                let typ = if require_or_sup.is_acc() {
                    require_or_sup
                } else {
                    Expr::try_from_type(typ.sup.typ().derefine()).unwrap_or(require_or_sup)
                };
                self.emit_expr(typ);
                1 // TODO: not always 1
            }
            _ => todo!(),
        }
    }

    fn emit_redef(&mut self, redef: ReDef) {
        log!(info "entered {} ({redef})", fn_name!());
        self.emit_simple_block(redef.block);
        self.store_acc(redef.attr);
    }

    fn emit_var_def(&mut self, sig: VarSignature, mut body: DefBody) {
        log!(info "entered {} ({sig} = {})", fn_name!(), body.block);
        if body.block.len() == 1 {
            self.emit_expr(body.block.remove(0));
        } else {
            self.emit_simple_block(body.block);
        }
        if sig.global {
            self.emit_store_global_instr(sig.ident);
        } else {
            self.emit_store_instr(sig.ident, Name);
        }
    }

    fn emit_glob_def(&mut self, _sig: GlobSignature, _body: DefBody) {}

    /// No parameter mangling is used
    /// so that Erg functions can be called from Python with keyword arguments.
    fn emit_params(
        &mut self,
        var_params: Option<&NonDefaultParamSignature>,
        defaults: Vec<DefaultParamSignature>,
        function_flag: &mut usize,
    ) {
        if var_params.is_some() && !defaults.is_empty() {
            let defaults_len = defaults.len();
            if self.opcode_set.is_3_14_plus() {
                // 3.14: BUILD_CONST_KEY_MAP removed; use interleaved key-value + BUILD_MAP
                for default in defaults {
                    let name = escape_name(
                        default.sig.inspect().map_or("_", |s| &s[..]),
                        &VisibilityModifier::Public,
                        0,
                        0,
                        false,
                    );
                    self.emit_load_const(name);
                    self.emit_expr(default.default_val);
                }
                self.write_opcode(BUILD_MAP);
                self.write_arg(defaults_len);
                self.stack_dec_n(defaults_len * 2 - 1);
            } else {
                let names = defaults
                    .iter()
                    .map(|default| {
                        escape_name(
                            default.sig.inspect().map_or("_", |s| &s[..]),
                            &VisibilityModifier::Public,
                            0,
                            0,
                            false,
                        )
                    })
                    .collect::<Vec<_>>();
                defaults
                    .into_iter()
                    .for_each(|default| self.emit_expr(default.default_val));
                self.emit_load_const(names);
                self.stack_dec();
                self.write_opcode(BUILD_CONST_KEY_MAP);
                self.write_arg(defaults_len);
                self.stack_dec_n(defaults_len - 1);
            }
            *function_flag += MakeFunctionFlags::KwDefaults as usize;
        } else if !defaults.is_empty() {
            let defaults_len = defaults.len();
            defaults
                .into_iter()
                .for_each(|default| self.emit_expr(default.default_val));
            self.write_opcode(BUILD_TUPLE);
            self.write_arg(defaults_len);
            self.stack_dec_n(defaults_len - 1);
            *function_flag += MakeFunctionFlags::Defaults as usize;
        }
    }

    fn emit_subr_def(&mut self, class_name: Option<&str>, sig: SubrSignature, body: DefBody) {
        log!(info "entered {} ({sig} = {})", fn_name!(), body.block);
        let name = sig.ident.inspect().clone();
        let mut make_function_flag = 0;
        let params = self.gen_param_names(&sig.params);
        let cell_names = self.frame_cells(&sig.params, &body.block);
        // A nested function some function closes over -- itself, when it is
        // recursive -- is bound to a cell, and the cell has to exist before the
        // function does: the recursive one carries its own cell in its
        // closure. So the name is given its slot now, ahead of the body, where
        // the body's reference to it then resolves to this scope rather than
        // to a local of its own that nothing ever stores.
        if !self.is_toplevel()
            && RegisterNameKind::from_ident(&sig.ident).is_fast()
            && self.captures.defs.contains(&sig.ident.vi.def_loc)
        {
            let escaped = escape_ident(sig.ident.clone());
            if self.opcode_set.is_3_11_plus() {
                let idx = self.register_fast_local(escaped);
                self.emit_make_cell(idx);
            } else if !self.cur_block_codeobj().cellvars.contains(&escaped) {
                // the frame makes the cell; the name only has to be found here
                self.mut_cur_block_codeobj().cellvars.push(escaped);
            }
        }
        let kwonlyargcount = if sig.params.var_params.is_some() {
            sig.params.defaults.len()
        } else {
            0
        };
        self.emit_params(
            sig.params.var_params.as_deref(),
            sig.params.defaults,
            &mut make_function_flag,
        );
        let mut flags = 0;
        if sig.params.var_params.is_some() {
            flags += CodeObjFlags::VarArgs as u32;
        }
        if sig.params.kw_var_params.is_some() {
            flags += CodeObjFlags::VarKeywords as u32;
        }
        let code = self.emit_block(
            body.block,
            sig.params.guards,
            Some(name.clone()),
            params,
            kwonlyargcount as u32,
            cell_names,
            flags,
        );
        // code.flags += CodeObjFlags::Optimized as u32;
        self.enclose_vars(&code, &mut make_function_flag);
        let n_decos = sig.decorators.len();
        for deco in sig.decorators {
            self.emit_expr(deco);
        }
        self.rewrite_captured_fast(&code);
        self.emit_load_const(code);
        if !self.opcode_set.is_3_11_plus() {
            if let Some(class) = class_name {
                self.emit_load_const(Str::from(format!("{class}.{name}")));
            } else {
                self.emit_load_const(name);
            }
        } else {
            self.stack_inc();
        }
        self.emit_make_function(make_function_flag);
        for _ in 0..n_decos {
            let argc = if self.opcode_set.is_3_11_plus() {
                0
            } else {
                self.stack_dec();
                1
            };
            self.emit_call_instr(argc, Name);
        }
        // stack_dec: <code obj> + <name> -> <function>
        self.stack_dec();
        if make_function_flag
            & (MakeFunctionFlags::Defaults as usize | MakeFunctionFlags::KwDefaults as usize)
            != 0
        {
            self.stack_dec();
        }
        self.emit_store_instr(sig.ident, Name);
    }

    fn emit_lambda(&mut self, lambda: Lambda) {
        log!(info "entered {} ({lambda})", fn_name!());
        let init_stack_len = self.stack_len();
        let mut make_function_flag = 0;
        let params = self.gen_param_names(&lambda.params);
        let cell_names = self.frame_cells(&lambda.params, &lambda.body);
        let kwonlyargcount = if lambda.params.var_params.is_some() {
            lambda.params.defaults.len()
        } else {
            0
        };
        self.emit_params(
            lambda.params.var_params.as_deref(),
            lambda.params.defaults,
            &mut make_function_flag,
        );
        let mut flags = 0;
        if lambda.params.var_params.is_some() {
            flags += CodeObjFlags::VarArgs as u32;
        }
        if lambda.params.kw_var_params.is_some() {
            flags += CodeObjFlags::VarKeywords as u32;
        }
        let code = self.emit_block(
            lambda.body,
            lambda.params.guards,
            Some(format!("<lambda_{}>", lambda.id).into()),
            params,
            kwonlyargcount as u32,
            cell_names,
            flags,
        );
        self.enclose_vars(&code, &mut make_function_flag);
        self.rewrite_captured_fast(&code);
        self.emit_load_const(code);
        if !self.opcode_set.is_3_11_plus() {
            self.emit_load_const(format!("<lambda_{}>", lambda.id));
        } else {
            self.stack_inc();
        }
        self.emit_make_function(make_function_flag);
        // stack_dec: <lambda code obj> + <name "<lambda>"> -> <function>
        self.stack_dec();
        if make_function_flag & MakeFunctionFlags::Defaults as usize != 0 {
            self.stack_dec();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    /// 3.13+: MAKE_FUNCTION no longer takes flags; attributes are attached with
    /// SET_FUNCTION_ATTRIBUTE in the reverse order of the pushed values
    /// (e.g. defaults, closure, code -> MAKE_FUNCTION -> closure(8), defaults(1))
    fn emit_make_function(&mut self, flag: usize) {
        if self.opcode_set.is_3_13_plus() {
            self.write_opcode(MAKE_FUNCTION);
            self.write_arg(0);
            for attr in [
                MakeFunctionFlags::Closure,
                MakeFunctionFlags::Annotations,
                MakeFunctionFlags::KwDefaults,
                MakeFunctionFlags::Defaults,
            ] {
                if flag & attr as usize != 0 {
                    self.write_instr(self.opcode_set.set_function_attribute());
                    self.write_arg(attr as usize);
                }
            }
        } else {
            self.write_opcode(MAKE_FUNCTION);
            self.write_arg(flag);
        }
    }

    /// Which of the parameters (in [`Self::gen_param_names`] order) some nested
    /// function closes over.
    fn captured_param_flags(&self, params: &Params) -> Vec<bool> {
        param_vis(params)
            .map(|vi| self.captures.defs.contains(&vi.def_loc))
            .collect()
    }

    /// The names a function's frame has to hold in cells: the parameters and
    /// locals of the function that some nested function closes over.
    fn frame_cells(&mut self, params: &Params, body: &Block) -> Vec<Str> {
        let mut walk = CaptureWalk {
            caps: &mut self.captures,
            into_functions: false,
        };
        let frame = walk.frame(params, body);
        frame.cells(&self.captures)
    }

    /// The slot of a local, given one if it has none yet.
    fn register_fast_local(&mut self, name: Str) -> usize {
        let varnames = &mut self.mut_cur_block_codeobj().varnames;
        varnames.iter().position(|v| v == &name).unwrap_or_else(|| {
            varnames.push(name);
            varnames.len() - 1
        })
    }

    /// Turn the slot into a cell (3.11+). Whatever the slot holds goes into the
    /// cell, and the variable is read and written through the cell from here on.
    fn emit_make_cell(&mut self, idx: usize) {
        debug_assert!(self.opcode_set.is_3_11_plus());
        let name = self.cur_block_codeobj().varnames[idx].clone();
        if self.cur_block().cells.contains(&name) {
            return;
        }
        self.write_instr(self.opcode_set.make_cell());
        self.write_arg(idx);
        self.mut_cur_block().cells.push(name.clone());
        if !self.cur_block_codeobj().cellvars.contains(&name) {
            self.mut_cur_block_codeobj().cellvars.push(name);
        }
    }

    /// The frame's cells, made as the frame starts -- before `COPY_FREE_VARS`
    /// and `RESUME`, where CPython's compiler puts them. One cell per variable
    /// per frame, so closures made in a loop share the loop's variable, as
    /// they do in Python.
    fn emit_frame_cells(&mut self, names: Vec<Str>) {
        for name in names {
            if self.opcode_set.is_3_11_plus() {
                let idx = self.register_fast_local(name);
                self.emit_make_cell(idx);
            } else if !self.cur_block_codeobj().cellvars.contains(&name) {
                // the frame makes the cells; every access goes through them
                self.mut_cur_block_codeobj().cellvars.push(name);
            }
        }
    }

    /// Put the slots in the order the frame will have them (3.11+).
    ///
    /// A slot is handed out when its name is first met, and the name of a
    /// variable a nested function closes over from further out is met in the
    /// middle of the body. But `COPY_FREE_VARS` fills the *last* slots of the
    /// frame with the closure, so those names have to end up last -- which
    /// [`CodeObj::localsplus_layout`] does. Reordering the names is not enough:
    /// every instruction that named a slot named it by the old number.
    fn remap_localsplus(&mut self) {
        if !self.opcode_set.is_3_11_plus() {
            return;
        }
        let codeobj = self.cur_block_codeobj();
        if codeobj.freevars.is_empty() {
            return;
        }
        let (order, _kinds) =
            CodeObj::localsplus_layout(&codeobj.varnames, &codeobj.freevars, &codeobj.cellvars);
        let map = codeobj
            .varnames
            .iter()
            .map(|v| order.iter().position(|o| o == v).unwrap())
            .collect::<Vec<_>>();
        if map.iter().enumerate().all(|(from, to)| from == *to) {
            return;
        }
        let slot_ops = [
            self.common_byte(LOAD_FAST),
            self.common_byte(STORE_FAST),
            self.common_byte(DELETE_FAST),
            self.opcode_set.load_deref(),
            self.opcode_set.store_deref(),
            self.opcode_set.load_closure(),
            self.opcode_set.make_cell(),
        ];
        let extended_arg = self.common_byte(EXTENDED_ARG);
        let mut overflow = false;
        let code = &mut self.mut_cur_block_codeobj().code;
        // wordcode: every unit is an opcode and an argument byte, inline
        // caches included, so even offsets are always opcodes
        let mut i = 0;
        while i + 1 < code.len() {
            if slot_ops.contains(&code[i]) {
                let extended = i >= 2 && code[i - 2] == extended_arg;
                let from = if extended {
                    (code[i - 1] as usize) << 8 | code[i + 1] as usize
                } else {
                    code[i + 1] as usize
                };
                if let Some(&to) = map.get(from) {
                    if extended {
                        code[i - 1] = (to >> 8) as u8;
                        code[i + 1] = (to & 0xff) as u8;
                    } else if let Ok(to) = u8::try_from(to) {
                        code[i + 1] = to;
                    } else {
                        overflow = true;
                    }
                }
            }
            i += 2;
        }
        if overflow {
            self.crash("too many locals to renumber a captured one");
        }
        self.mut_cur_block_codeobj().varnames = order;
    }

    fn enclose_vars(&mut self, code: &CodeObj, flag: &mut usize) {
        if self.opcode_set.is_3_11_plus() {
            // Since 3.11, LOAD_CLOSURE is simply an alias for LOAD_FAST.
            // The closure tuple must be built in the order of the child's freevars
            // (which may include variables passed through this block as its own freevars)
            let freevars = code.freevars.clone();
            let mut nloaded = 0;
            for name in freevars.iter() {
                let Some(idx) = self
                    .cur_block_codeobj()
                    .varnames
                    .iter()
                    .position(|n| n == name)
                else {
                    continue;
                };
                // What goes into the closure is the cell. The slot holds one
                // when the variable was made a cell where it was bound, and
                // when it is itself a closure variable passing through. Any
                // other is a capture lowering did not record: make the cell
                // now, late -- a use of the variable before this point that
                // runs again (in a loop) would read the cell, so this is a
                // fallback, not the design.
                let holds_cell = self.cur_block().cells.iter().any(|c| c == name)
                    || self.cur_block_codeobj().freevars.iter().any(|f| f == name);
                if !holds_cell {
                    log!(err "unrecorded capture of {name}, making its cell late");
                    self.emit_make_cell(idx);
                }
                self.write_instr(self.opcode_set.load_closure());
                self.write_arg(idx);
                nloaded += 1;
            }
            if nloaded > 0 {
                self.write_opcode(BUILD_TUPLE);
                self.write_arg(nloaded);
                *flag += MakeFunctionFlags::Closure as usize;
            }
        } else {
            // The closure is the child's freevars, in the child's order. Before
            // 3.11 a deref and LOAD_CLOSURE index cellvars then freevars.
            let freevars = code.freevars.clone();
            let mut nloaded = 0;
            for name in freevars.iter() {
                let codeobj = self.cur_block_codeobj();
                let idx = if let Some(i) = codeobj.cellvars.iter().position(|c| c == name) {
                    i
                } else if let Some(j) = codeobj.freevars.iter().position(|f| f == name) {
                    codeobj.cellvars.len() + j
                } else {
                    continue;
                };
                self.write_instr(self.opcode_set.load_closure());
                self.write_arg(idx);
                nloaded += 1;
            }
            if nloaded > 0 {
                self.write_opcode(BUILD_TUPLE);
                self.write_arg(nloaded);
                *flag += MakeFunctionFlags::Closure as usize;
            }
        }
    }

    /// ```erg
    /// f = i ->
    ///     log i
    ///     do i
    /// ```
    /// ↓
    /// ```pyc
    /// Disassembly of <code object f ...>
    /// 0 LOAD_NAME                0 (print)
    /// 2 LOAD_NAME                1 (Nat)
    /// 4 LOAD_DEREF               0 (::i) # not LOAD_FAST!
    /// 6 CALL_FUNCTION            1
    /// 8 CALL_FUNCTION            1
    /// 10 POP_TOP
    /// 12 LOAD_CLOSURE             0 (::i)
    /// 14 BUILD_TUPLE              1
    /// 16 LOAD_CONST               0 (<code object ...>)
    /// 18 LOAD_CONST               1 ("<lambda>")
    /// 20 MAKE_FUNCTION            8 (closure)
    /// 22 RETURN_VALUE
    /// ```
    fn rewrite_captured_fast(&mut self, code: &CodeObj) {
        if self.opcode_set.is_3_11_plus() {
            return;
        }
        let cellvars = self.cur_block_codeobj().cellvars.clone();
        for cellvar in cellvars {
            if code.freevars.iter().any(|n| n == &cellvar) {
                // a nested function bound straight to a cell has no local slot
                let Some(old_idx) = self
                    .cur_block_codeobj()
                    .varnames
                    .iter()
                    .position(|n| n == &cellvar)
                else {
                    continue;
                };
                let new_idx = self
                    .cur_block_codeobj()
                    .cellvars
                    .iter()
                    .position(|n| n == &cellvar)
                    .unwrap();
                self.mut_cur_block().captured_vars.push(cellvar);
                let load_deref_op = self.opcode_set.load_deref();
                let store_deref_op = self.opcode_set.store_deref();
                let load_fast_byte = self.common_byte(LOAD_FAST);
                let store_fast_byte = self.common_byte(STORE_FAST);
                let mut op_idx = 0;
                while let Some([op, arg]) = self
                    .mut_cur_block_codeobj()
                    .code
                    .get_mut(op_idx..=op_idx + 1)
                {
                    if *op == load_fast_byte && *arg == old_idx as u8 {
                        *op = load_deref_op;
                        *arg = new_idx as u8;
                    } else if *op == store_fast_byte && *arg == old_idx as u8 {
                        *op = store_deref_op;
                        *arg = new_idx as u8;
                    }
                    op_idx += 2;
                }
            }
        }
    }

    /// Emit the error propagation operator `x?`:
    ///
    /// ```python
    /// tmp = x
    /// if is_err(tmp):
    ///     return push_err_frame(tmp, name, line, file)
    /// tmp  # the success value
    /// ```
    ///
    /// The early `return` is a plain `RETURN_VALUE`, so `?` returns from the code
    /// object it is emitted into. Control-flow blocks (`if`'s `do:` etc.) are
    /// inlined, so that is the enclosing subroutine, as the lowerer assumes.
    ///
    /// `push_err_frame` records this call site on `Error.stack`, which is what
    /// makes an error carry a trace of the `?`s it travelled through. At the top
    /// level there is no subroutine to return from, so `panic_err` prints that
    /// trace and aborts instead.
    fn emit_try_instr(&mut self, unary: UnaryOp) {
        log!(info "entered {} ({unary})", fn_name!());
        let init_stack_len = self.stack_len();
        if !self.err_ops_loaded {
            self.load_err_ops();
        }
        let line = unary.op.ln_begin().unwrap_or(0) as usize;
        let subr_name = self.cur_block_codeobj().name.clone();
        let filename = self.cur_block_codeobj().filename.clone();
        let can_return = !self.is_toplevel();
        let tmp = Identifier::private_with_line(self.fresh_gen.fresh_varname(), 0);
        self.emit_expr(*unary.expr);
        self.emit_store_instr(tmp.clone(), Name);
        // is_err(tmp)
        self.emit_push_null();
        self.emit_load_name_instr(Identifier::private("#is_err"));
        self.fixup_push_null_order();
        self.emit_load_name_instr(tmp.clone());
        self.emit_call_instr(1, Name);
        self.stack_dec();
        self.emit_to_bool();
        let idx_pop_jump_if_false = self.lasti();
        self.write_opcode(EXTENDED_ARG);
        self.write_arg(0);
        self.write_instr(self.opcode_set.pop_jump_if_false());
        // cannot detect where to jump to at this moment, so put as 0
        self.write_arg(0);
        let pjc = self.emit_pop_jump_cache();
        // POP_JUMP pops the condition
        self.stack_dec();
        // error path: record this call site, then return the error object itself
        // (`panic_err` never returns, so the RETURN_VALUE after it is dead code
        // that only keeps the stack balanced)
        let helper = if can_return {
            "#push_err_frame"
        } else {
            "#panic_err"
        };
        self.emit_push_null();
        self.emit_load_name_instr(Identifier::private(helper));
        self.fixup_push_null_order();
        self.emit_load_name_instr(tmp.clone());
        self.emit_load_const(subr_name);
        self.emit_load_const(line);
        self.emit_load_const(filename);
        self.emit_call_instr(4, Name);
        self.stack_dec_n(4);
        self.write_opcode(RETURN_VALUE);
        self.write_arg(0);
        self.stack_dec();
        let idx_ok_begin = if self.opcode_set.is_3_11_plus() {
            self.lasti() - idx_pop_jump_if_false - 4 - pjc
        } else {
            self.lasti()
        };
        self.fill_jump(idx_pop_jump_if_false + 1, idx_ok_begin);
        // success path: the value is already narrowed to the success type
        self.emit_load_name_instr(tmp);
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_unaryop(&mut self, unary: UnaryOp) {
        log!(info "entered {} ({unary})", fn_name!());
        if unary.op.is(TokenKind::Try) {
            return self.emit_try_instr(unary);
        }
        let init_stack_len = self.stack_len();
        let val_t = unary
            .info
            .t
            .non_default_params()
            .and_then(|tys| tys.first().map(|pt| pt.typ()))
            .unwrap_or(Type::FAILURE);
        let tycode = TypeCode::from(val_t);
        let instr = match &unary.op.kind {
            // TODO:
            TokenKind::PrePlus => UNARY_POSITIVE,
            TokenKind::PreMinus => UNARY_NEGATIVE,
            TokenKind::PreBitNot => UNARY_INVERT,
            TokenKind::Mutate => {
                if !self.mutate_op_loaded {
                    self.load_mutate_op();
                }
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::private("#mutate_operator"));
                self.fixup_push_null_order();
                NOP // ERG_MUTATE,
            }
            _ => {
                CompileError::feature_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    unary.op.loc(),
                    &unary.op.inspect().clone(),
                    String::from(unary.op.content),
                )
                .write_to_stderr();
                NOT_IMPLEMENTED
            }
        };
        self.emit_expr(*unary.expr);
        if instr == UNARY_POSITIVE && self.opcode_set.is_3_12_plus() {
            // 3.12: UNARY_POSITIVE removed; use CALL_INTRINSIC_1(UnaryPositive=5)
            self.write_instr(self.opcode_set.call_intrinsic_1());
            self.write_arg(5);
        } else if instr != NOP {
            self.write_opcode(instr);
            self.write_arg(tycode as usize);
        } else {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    /// Emit a literal. `Ratio` literals (e.g. `0.1`, `1.5`, `3.14`) are constructed
    /// as `Ratio("<source>")` -- a `fractions.Fraction` -- so that rationals stay exact
    /// (`0.1 + 0.2 == 0.3` holds, unlike Python `float`). All other literals are
    /// emitted as plain constants. In `no_std` mode `Ratio` falls back to a float const.
    fn emit_literal(&mut self, lit: Literal) {
        if !self.cfg.no_std {
            if lit.is(TokenKind::RatioLit) {
                return self.emit_ratio(lit.token.content);
            }
            // A folded rational (`A = 0.1 + 0.2`) has no literal text to fall back
            // on, and `Fraction` is not a marshal constant, so build it from `n/d`.
            // Without this it would be serialized as the nearest `f64`, and the
            // program would print 0.3 where `0.1 + 0.2` prints 3/10.
            if let ValueObj::Ratio(n, d) = lit.value {
                return self.emit_ratio(Str::from(format!("{n}/{d}")));
            }
        }
        self.emit_load_const(lit.value);
    }

    /// Emit `Ratio("<source>")`. `Ratio` is a `fractions.Fraction`, whose *string*
    /// constructor parses decimals and scientific notation exactly
    /// (`Fraction("0.1") == 1/10`), so the literal text is passed through as
    /// written. `Fraction(0.1)` would take the float route and give it its binary
    /// value instead.
    ///
    /// Digit separators are dropped first: `Fraction` only learned to skip them
    /// in 3.11, and erg targets 3.7 up, where `Fraction("1_000.5")` raises.
    fn emit_ratio(&mut self, content: Str) {
        self.emit_push_null();
        self.emit_load_name_instr(Identifier::static_public("Ratio"));
        self.fixup_push_null_order();
        let content = Str::from(content.replace('_', ""));
        let arg = Expr::Literal(Literal::new(ValueObj::Str(content), Token::DUMMY));
        let args = Args::single(PosArg::new(arg));
        self.emit_args_311(args, Name);
    }

    fn emit_binop(&mut self, bin: BinOp) {
        log!(info "entered {} ({bin})", fn_name!());
        let init_stack_len = self.stack_len();
        // TODO: and/orのプリミティブ命令の実装
        // Range operators are not operators in Python
        match &bin.op.kind {
            // l..<r == range(l, r)
            TokenKind::RightOpen => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::static_public("RightOpenRange"));
                self.fixup_push_null_order();
            }
            TokenKind::LeftOpen => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::static_public("LeftOpenRange"));
                self.fixup_push_null_order();
            }
            TokenKind::Closed => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::static_public("ClosedRange"));
                self.fixup_push_null_order();
            }
            TokenKind::Open => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::static_public("OpenRange"));
                self.fixup_push_null_order();
            }
            // From 3.10, `or` can be used for types.
            // But Erg supports Python 3.7~, so we should use `typing.Union`.
            TokenKind::OrOp if bin.lhs.ref_t().is_type() => {
                self.load_union();
                let args = Args::pos_only(vec![PosArg::new(*bin.lhs), PosArg::new(*bin.rhs)], None);
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::private("#UnionType"));
                self.fixup_push_null_order();
                self.emit_args_311(args, Name);
                return;
            }
            // short circuiting
            TokenKind::OrOp => {
                self.emit_expr(*bin.lhs);
                if self.opcode_set.is_3_12_plus() {
                    // 3.12: JUMP_IF_TRUE_OR_POP removed
                    // → COPY(1) + TO_BOOL (3.13+) + POP_JUMP_IF_TRUE + POP_TOP
                    self.write_instr(self.opcode_set.copy());
                    self.write_arg(1);
                    self.stack_inc();
                    self.emit_to_bool();
                    let idx = self.lasti();
                    self.write_opcode(EXTENDED_ARG);
                    self.write_arg(0);
                    self.write_instr(self.opcode_set.pop_jump_forward_if_true());
                    self.write_arg(0);
                    let pjc = self.emit_pop_jump_cache();
                    self.stack_dec(); // POP_JUMP pops the copy
                    self.emit_pop_top(); // pop the original (falsy) value
                    self.emit_expr(*bin.rhs);
                    let arg = self.lasti() - idx - 4 - pjc;
                    self.fill_jump(idx + 1, arg);
                } else {
                    let idx = self.lasti();
                    self.write_opcode(EXTENDED_ARG);
                    self.write_arg(0);
                    self.write_instr(self.opcode_set.jump_if_true_or_pop());
                    self.write_arg(0);
                    self.emit_expr(*bin.rhs);
                    let arg = if self.opcode_set.is_3_11_plus() {
                        self.lasti() - idx - 4
                    } else {
                        self.lasti()
                    };
                    self.fill_jump(idx + 1, arg);
                    self.stack_dec();
                }
                return;
            }
            TokenKind::AndOp => {
                self.emit_expr(*bin.lhs);
                if self.opcode_set.is_3_12_plus() {
                    // 3.12: JUMP_IF_FALSE_OR_POP removed
                    // → COPY(1) + TO_BOOL (3.13+) + POP_JUMP_IF_FALSE + POP_TOP
                    self.write_instr(self.opcode_set.copy());
                    self.write_arg(1);
                    self.stack_inc();
                    self.emit_to_bool();
                    let idx = self.lasti();
                    self.write_opcode(EXTENDED_ARG);
                    self.write_arg(0);
                    self.write_instr(self.opcode_set.pop_jump_forward_if_false());
                    self.write_arg(0);
                    let pjc = self.emit_pop_jump_cache();
                    self.stack_dec(); // POP_JUMP pops the copy
                    self.emit_pop_top(); // pop the original (truthy) value
                    self.emit_expr(*bin.rhs);
                    let arg = self.lasti() - idx - 4 - pjc;
                    self.fill_jump(idx + 1, arg);
                } else {
                    let idx = self.lasti();
                    self.write_opcode(EXTENDED_ARG);
                    self.write_arg(0);
                    self.write_instr(self.opcode_set.jump_if_false_or_pop());
                    self.write_arg(0);
                    self.emit_expr(*bin.rhs);
                    let arg = if self.opcode_set.is_3_11_plus() {
                        self.lasti() - idx - 4
                    } else {
                        self.lasti()
                    };
                    self.fill_jump(idx + 1, arg);
                    self.stack_dec();
                }
                return;
            }
            TokenKind::ContainsOp => {
                // if no-std, always `x contains y == True`
                if self.cfg.no_std {
                    self.emit_load_const(true);
                    return;
                }
                if !self.contains_op_loaded {
                    self.load_contains_op();
                }
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::private("#contains_operator"));
                self.fixup_push_null_order();
            }
            TokenKind::Less | TokenKind::LessEq | TokenKind::Gre | TokenKind::GreEq
                if !self.cfg.no_std && Self::is_type_inclusion_cmp(&bin) =>
            {
                if !self.subtype_op_loaded {
                    self.load_type_cmp_ops();
                }
                let helper = match &bin.op.kind {
                    TokenKind::Less => "#is_lt",
                    TokenKind::LessEq => "#is_le",
                    TokenKind::Gre => "#is_gt",
                    TokenKind::GreEq => "#is_ge",
                    _ => unreachable!(),
                };
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::private(helper));
                self.fixup_push_null_order();
                self.emit_expr(*bin.lhs);
                self.emit_expr(*bin.rhs);
                self.emit_binop_instr(
                    Token::dummy(TokenKind::ContainsOp, "in"),
                    TypePair::new(&Type::Type, &Type::Type),
                );
                debug_assert_eq!(self.stack_len(), init_stack_len + 1);
                return;
            }
            _ => {}
        }
        let lhs_t = bin
            .info
            .t
            .non_default_params()
            .and_then(|tys| tys.first().map(|pt| pt.typ()))
            .unwrap_or(Type::FAILURE);
        let rhs_t = bin
            .info
            .t
            .non_default_params()
            .and_then(|tys| tys.get(1).map(|pt| pt.typ()))
            .unwrap_or(Type::FAILURE);
        let type_pair = TypePair::new(lhs_t, rhs_t);
        // Polymorphic division (e.g. `f x = x / 2`) has the projected result type
        // `L.Output`, so the concrete operand types aren't known here. Defer to the
        // runtime `true_div` helper, which keeps integer division exact while
        // leaving `Float`/`Complex` untouched.
        if !self.cfg.no_std
            && bin.op.is(TokenKind::Slash)
            && matches!(bin.ref_t(), Type::Proj { .. })
        {
            if !self.true_div_loaded {
                self.load_true_div();
            }
            self.emit_push_null();
            self.emit_load_name_instr(Identifier::private("#true_div"));
            self.fixup_push_null_order();
            let args = Args::pos_only(vec![PosArg::new(*bin.lhs), PosArg::new(*bin.rhs)], None);
            self.emit_args_311(args, Name);
            debug_assert_eq!(self.stack_len(), init_stack_len + 1);
            return;
        }
        // `a / b : Ratio` is exact: emit `Fraction(a) / b` so that integer
        // division (`1 / 3`) yields `Fraction(1, 3)` rather than a lossy float.
        // When `a` is already a `Ratio` (`Fraction` at runtime), `Fraction(a)` is
        // idempotent. `Fraction(a) / b` propagates exactness whatever `b` is.
        // `derefine()` so a refined result (e.g. `{R: Ratio | R >= 0}` from interval
        // arithmetic) is still recognized as `Ratio` and emitted exactly via `Fraction`.
        // `**` needs the same treatment: `2 ** -1` is 1/2, and a plain
        // `2 ** -1` would be the float 0.5. `Fraction(a) ** b` is exact for any
        // integer `b`, and for a fractional one it falls back to a float, which
        // is what the `(Ratio, Ratio) -> Float` branch of `**` declares.
        let ratio_div = !self.cfg.no_std
            && (bin.op.is(TokenKind::Slash) || bin.op.is(TokenKind::Pow))
            && bin.ref_t().derefine() == Type::Ratio;
        if ratio_div {
            self.emit_push_null();
            self.emit_load_name_instr(Identifier::static_public("Ratio"));
            self.fixup_push_null_order();
            let args = Args::single(PosArg::new(*bin.lhs));
            self.emit_args_311(args, Name);
            self.emit_expr(*bin.rhs);
        } else {
            self.emit_expr(*bin.lhs);
            self.emit_expr(*bin.rhs);
        }
        self.emit_binop_instr(bin.op, type_pair);
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    pub(crate) fn emit_binop_instr(&mut self, binop: Token, type_pair: TypePair) {
        if self.opcode_set.is_3_11_plus() {
            self.emit_binop_instr_311(binop, type_pair);
        } else if self.opcode_set.is_3_9_plus() {
            self.emit_binop_instr_309(binop, type_pair);
        } else {
            self.emit_binop_instr_307(binop, type_pair);
        }
    }

    // Binop implementations are in codegen_v307.rs, codegen_v309.rs, codegen_v311.rs

    fn emit_del_instr(&mut self, mut args: Args) {
        let Some(Expr::Accessor(Accessor::Ident(ident))) = args.remove_left_or_key("obj") else {
            log!(err "del instruction requires an identifier");
            return;
        };
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped, Fast));
        self.write_opcode(DELETE_NAME);
        self.write_arg(name.idx);
        self.emit_load_const(ValueObj::None);
    }

    fn emit_not_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let expr = args.remove_left_or_key("b").unwrap();
        self.emit_expr(expr);
        self.emit_to_bool();
        self.write_opcode(UNARY_NOT);
        self.write_arg(0);
    }

    fn emit_discard_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        while let Some(arg) = args.try_remove(0) {
            self.emit_expr(arg);
            self.emit_pop_top();
        }
        self.emit_load_const(ValueObj::None);
    }

    pub(crate) fn deopt_instr(&mut self, kind: ControlKind, args: Args) {
        if !self.control_loaded {
            self.load_control();
        }
        let local = match kind {
            ControlKind::If => Identifier::static_public("if__"),
            ControlKind::For => Identifier::static_public("for__"),
            ControlKind::While => Identifier::static_public("while__"),
            ControlKind::With => Identifier::static_public("with__"),
            ControlKind::Discard => Identifier::static_public("discard__"),
            ControlKind::Assert => Identifier::static_public("assert__"),
            kind => todo!("{kind:?}"),
        };
        self.emit_call_local(local, args);
    }

    fn emit_if_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let init_stack_len = self.stack_len();
        let cond = args.remove(0);
        self.emit_expr(cond);
        self.emit_to_bool();
        let idx_pop_jump_if_false = self.lasti();
        self.write_opcode(EXTENDED_ARG);
        self.write_arg(0);
        self.write_instr(self.opcode_set.pop_jump_if_false());
        // cannot detect where to jump to at this moment, so put as 0
        self.write_arg(0);
        let pjc = self.emit_pop_jump_cache();
        match args.remove(0) {
            // then block
            Expr::Lambda(lambda) => {
                // let params = self.gen_param_names(&lambda.params);
                self.emit_simple_block(lambda.body);
            }
            other => {
                self.emit_expr(other);
            }
        }
        if args.get(0).is_some() {
            let idx_jump_forward = self.lasti();
            self.write_opcode(EXTENDED_ARG);
            self.write_arg(0);
            self.write_instr(self.opcode_set.jump_forward()); // jump to end
            self.write_arg(0);
            // else block
            let idx_else_begin = if self.opcode_set.is_3_11_plus() {
                self.lasti() - idx_pop_jump_if_false - 4 - pjc
            } else {
                self.lasti()
            };
            self.fill_jump(idx_pop_jump_if_false + 1, idx_else_begin);
            match args.remove(0) {
                Expr::Lambda(lambda) => {
                    // let params = self.gen_param_names(&lambda.params);
                    self.emit_simple_block(lambda.body);
                }
                other => {
                    self.emit_expr(other);
                }
            }
            let idx_end = self.lasti();
            self.fill_jump(idx_jump_forward + 1, idx_end - idx_jump_forward - 2 - 1);
            // FIXME: this is a hack to make sure the stack is balanced
            while self.stack_len() != init_stack_len + 1 {
                self.stack_dec();
            }
        } else {
            self.write_instr(self.opcode_set.jump_forward());
            let jump_to = if self.opcode_set.is_3_10_plus() { 1 } else { 2 };
            self.write_arg(jump_to);
            // no else block
            let idx_end = if self.opcode_set.is_3_11_plus() {
                self.lasti() - idx_pop_jump_if_false - 3 - pjc
            } else {
                self.lasti()
            };
            self.fill_jump(idx_pop_jump_if_false + 1, idx_end);
            self.emit_load_const(ValueObj::None);
            while self.stack_len() != init_stack_len + 1 {
                self.stack_dec();
            }
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_for_instr(&mut self, mut args: Args) {
        log!(info "entered {} ({})", fn_name!(), args);
        // the same decision `CaptureWalk::call` makes
        let inlined = matches!(args.get(1), Some(Expr::Lambda(l)) if !loop_body_needs_frame(l));
        if !inlined {
            return self.deopt_instr(ControlKind::For, args);
        }
        let _init_stack_len = self.stack_len();
        let iterable = args.remove(0);
        self.emit_expr(iterable);
        self.write_opcode(GET_ITER);
        self.write_arg(0);
        let idx_for_iter = self.lasti();
        self.write_opcode(EXTENDED_ARG);
        self.write_arg(0);
        self.write_opcode(FOR_ITER);
        self.stack_inc();
        // FOR_ITER pushes a value onto the stack, but we can't know how many
        // but after executing this instruction, stack_len should be 1
        // cannot detect where to jump to at this moment, so put as 0
        self.write_arg(0);
        // 3.12+: FOR_ITER has 1 inline cache entry that the interpreter skips.
        // Without this, the next instruction (STORE_NAME) gets eaten as CACHE data.
        if self.opcode_set.is_3_12_plus() {
            let cache = self.opcode_set.cache_entries_for_iter() * 2;
            self.write_bytes(&vec![0; cache]);
        }
        let Expr::Lambda(lambda) = args.remove(0) else {
            unreachable!()
        };
        // If there is nothing on the stack at the start, init_stack_len == 2 (an iterator and the first iterator value)
        let init_stack_len = self.stack_len();
        // store the iterator value, stack_len == 1 or 2 in the end
        self.emit_control_block(lambda.body, lambda.params);
        if self.stack_len() > init_stack_len - 1 {
            self.emit_pop_top();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len - 1); // the iterator is remained
        let idx = self.lasti();
        self.write_opcode(EXTENDED_ARG);
        self.write_arg(0);
        if self.opcode_set.is_3_11_plus() {
            self.write_instr(self.opcode_set.jump_backward());
            self.write_arg(0);
            let jbc = self.emit_jump_backward_cache();
            self.fill_jump(idx + 1, self.lasti() - idx_for_iter);
            let _ = jbc; // cache included in lasti
        } else {
            self.write_instr(self.opcode_set.jump_absolute());
            self.write_arg(0);
            self.fill_jump(idx + 1, idx_for_iter);
        }
        // 3.12+: emit END_FOR after the loop (FOR_ITER's exhausted path jumps to END_FOR)
        if self.opcode_set.is_3_12_plus() {
            self.write_instr(self.opcode_set.end_for());
            self.write_arg(0);
        }
        // Record idx_end BEFORE POP_ITER: FOR_ITER should jump to END_FOR, not past POP_ITER
        let idx_end = self.lasti();
        if self.opcode_set.is_3_14_plus() {
            // 3.14+: POP_ITER pops the exhausted iterator after END_FOR
            self.write_instr(self.opcode_set.pop_iter());
            self.write_arg(0);
        } else if self.opcode_set.is_3_13_plus() {
            // 3.13: END_FOR pops only the yielded value (in 3.12 it popped both
            // the value and the iterator). A trailing POP_TOP is needed to pop
            // the exhausted iterator. See CPython gh-121399.
            self.write_opcode(POP_TOP);
            self.write_arg(0);
        }
        if self.opcode_set.is_3_12_plus() {
            // 3.12: FOR_ITER exhausted does JUMPBY(oparg+1) from after CACHE.
            // next_instr = idx_for_iter + 6 (EXTENDED_ARG + FOR_ITER + CACHE)
            // target = idx_end (END_FOR), so (oparg+1)*2 = idx_end - idx_for_iter - 6
            // fill_jump value = oparg * 2 = idx_end - idx_for_iter - 6 - 2 = idx_end - idx_for_iter - 8
            self.fill_jump(idx_for_iter + 1, idx_end - idx_for_iter - 2 - 2 - 4);
        } else {
            self.fill_jump(idx_for_iter + 1, idx_end - idx_for_iter - 2 - 2);
        }
        self.stack_dec();
        self.emit_load_const(ValueObj::None);
        debug_assert_eq!(self.stack_len(), _init_stack_len + 1);
    }

    fn emit_while_instr(&mut self, mut args: Args) {
        log!(info "entered {} ({})", fn_name!(), args);
        // the same decision `CaptureWalk::call` makes
        let inlined = matches!(args.get(1), Some(Expr::Lambda(l)) if !loop_body_needs_frame(l));
        if !inlined {
            return self.deopt_instr(ControlKind::While, args);
        }
        let _init_stack_len = self.stack_len();
        // e.g. is_foo!: () => Bool, do!(is_bar)
        let cond_block = args.remove(0);
        let cond = match cond_block {
            Expr::Lambda(mut lambda) => lambda.body.remove(0),
            Expr::Accessor(acc) => Expr::Accessor(acc).call_expr(Args::empty()),
            _ => todo!(),
        };
        // Evaluate again at the end of the loop
        self.emit_expr(cond.clone());
        self.emit_to_bool();
        let idx_while = self.lasti();
        self.write_opcode(EXTENDED_ARG);
        self.write_arg(0);
        self.write_instr(self.opcode_set.pop_jump_if_false());
        self.write_arg(0);
        let pjc1 = self.emit_pop_jump_cache();
        self.stack_dec();
        let Expr::Lambda(lambda) = args.remove(0) else {
            unreachable!()
        };
        let init_stack_len = self.stack_len();
        self.emit_control_block(lambda.body, lambda.params);
        if self.stack_len() > init_stack_len {
            self.emit_pop_top();
        }
        self.emit_expr(cond);
        self.emit_to_bool();
        let idx = self.lasti();
        self.write_opcode(EXTENDED_ARG);
        self.write_arg(0);
        if self.opcode_set.is_3_12_plus() {
            // 3.12: POP_JUMP_BACKWARD_IF_TRUE was removed.
            // Use POP_JUMP_IF_FALSE (forward → end) + JUMP_BACKWARD (back → body).
            self.write_instr(self.opcode_set.pop_jump_if_false());
            self.write_arg(0);
            let pjc2 = self.emit_pop_jump_cache();
            self.stack_dec();
            let idx_jb = self.lasti();
            self.write_opcode(EXTENDED_ARG);
            self.write_arg(0);
            self.write_instr(self.opcode_set.jump_backward());
            self.write_arg(0);
            self.emit_jump_backward_cache();
            // Fill JUMP_BACKWARD: backward to body start (idx_while + 4 + pjc1)
            self.fill_jump(idx_jb + 1, self.lasti() - idx_while - 4 - pjc1);
            // Fill second POP_JUMP_IF_FALSE: forward to end (self.lasti())
            self.fill_jump(idx + 1, self.lasti() - idx - 4 - pjc2);
            // Fill first POP_JUMP_IF_FALSE: forward to end (self.lasti())
            self.fill_jump(idx_while + 1, self.lasti() - idx_while - 4 - pjc1);
        } else if self.opcode_set.is_3_11_plus() {
            let arg = self.lasti() - (idx_while + 2);
            self.write_instr(self.opcode_set.pop_jump_backward_if_true());
            self.write_arg(0);
            self.fill_jump(idx + 1, arg);
            self.stack_dec();
            let idx_end = self.lasti() - idx_while - 3;
            self.fill_jump(idx_while + 1, idx_end);
        } else {
            self.write_instr(self.opcode_set.pop_jump_if_true());
            self.write_arg(0);
            self.fill_jump(idx + 1, idx_while + 4);
            self.stack_dec();
            self.fill_jump(idx_while + 1, self.lasti());
        }
        self.emit_load_const(ValueObj::None);
        debug_assert_eq!(self.stack_len(), _init_stack_len + 1);
    }

    fn emit_match_instr(&mut self, mut args: Args, _use_erg_specific: bool) {
        log!(info "entered {}", fn_name!());
        let init_stack_len = self.stack_len();
        let expr = args.remove(0);
        self.emit_expr(expr);
        let len = args.len();
        let mut jump_forward_points = vec![];
        while let Some(expr) = args.try_remove(0) {
            if len > 1 && !args.is_empty() {
                self.dup_top();
            }
            // compilerで型チェック済み(可読性が下がるため、matchでNamedは使えない)
            let Expr::Lambda(lambda) = expr else {
                unreachable!()
            };
            debug_power_assert!(lambda.params.len(), ==, 1);
            if !lambda.params.defaults.is_empty() {
                todo!("default values in match expression are not supported yet")
            }
            let pop_jump_points = self.emit_match_pattern(lambda.params, args.is_empty());
            self.emit_simple_block(lambda.body);
            // If we move on to the next arm, the stack size will increase
            // so `self.stack_dec();` for now (+1 at the end).
            self.stack_dec();
            for pop_jump_point in pop_jump_points {
                let idx = if self.opcode_set.is_3_11_plus() {
                    // The arm to jump to starts after the EXTENDED_ARG + JUMP_FORWARD
                    // written just below, so the target is `lasti + 4`; a relative
                    // jump counts from after POP_JUMP_IF_FALSE *and its cache*, at
                    // `pop_jump_point + 4 + cache`. The two fours cancel; the cache
                    // does not, and 3.13 is where it stopped being zero.
                    self.lasti()
                        - pop_jump_point
                        - self.opcode_set.cache_entries_pop_jump_if_false() * 2
                } else {
                    self.lasti() + 4
                };
                self.fill_jump(pop_jump_point + 1, idx); // jump to the next arm
            }
            jump_forward_points.push(self.lasti());
            self.write_opcode(EXTENDED_ARG);
            self.write_arg(0);
            self.write_instr(self.opcode_set.jump_forward()); // jump to the end
            self.write_arg(0);
        }
        let lasti = self.lasti();
        for jump_point in jump_forward_points.into_iter() {
            let jump_to = lasti - jump_point - 2 - 2;
            self.fill_jump(jump_point + 1, jump_to);
        }
        self.stack_inc();
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    /// return `None` if the arm is the last one
    fn emit_match_pattern(&mut self, mut params: Params, is_last_arm: bool) -> Vec<usize> {
        let param = params.non_defaults.remove(0);
        log!(info "entered {}({param})", fn_name!());
        match &param.raw.pat {
            ParamPattern::VarName(name) => {
                let ident = erg_parser::ast::Identifier::private_from_varname(name.clone());
                let ident = Identifier::new(ident, None, param.vi);
                let kind = self.block_param_kind();
                let captured = self.captures.defs.contains(&ident.vi.def_loc);
                self.emit_store_instr_captured(ident, AccessKind::Name, kind, captured);
            }
            ParamPattern::Discard(_) => {
                self.emit_pop_top();
            }
            _other => unreachable!(),
        }
        let mut pop_jump_points = vec![];
        let last = params.guards.len().saturating_sub(1);
        for (i, mut guard) in params.guards.into_iter().enumerate() {
            if let GuardClause::Condition(Expr::BinOp(BinOp {
                op:
                    Token {
                        kind: TokenKind::ContainsOp,
                        ..
                    },
                lhs,
                rhs,
                ..
            })) = &mut guard
            {
                // Guards have not been checked.
                // Therefore, an invalid guard may be generated.
                if lhs.var_info().is_some_and(|vi| vi == &VarInfo::ILLEGAL) {
                    continue;
                }
                if rhs.var_info().is_some_and(|vi| vi == &VarInfo::ILLEGAL) {
                    continue;
                }
                // *lhs.ref_mut_t().unwrap() = Type::Obj;
                *rhs.ref_mut_t().unwrap() = Type::Obj;
            }
            match guard {
                GuardClause::Bind(def) => {
                    self.emit_def(def);
                }
                GuardClause::Condition(cond) => {
                    self.emit_expr(cond);
                    if is_last_arm {
                        self.emit_pop_top();
                    } else {
                        self.emit_to_bool();
                        pop_jump_points.push(self.lasti());
                        // HACK: match branches often jump very far (beyond the u8 range),
                        // so the jump destination should be reserved as the u16 range.
                        // Other jump instructions may need to be replaced by this way.
                        self.write_opcode(EXTENDED_ARG);
                        self.write_arg(0);
                        // in 3.11, POP_JUMP_IF_FALSE is replaced with POP_JUMP_FORWARD_IF_FALSE
                        // but the numbers are the same, only the way the jumping points are calculated is different.
                        self.write_instr(self.opcode_set.pop_jump_if_false()); // jump to the next case
                        self.write_arg(0);
                        self.emit_pop_jump_cache();
                        // if matched, pop original
                        if i == last {
                            self.emit_pop_top();
                        } else {
                            self.stack_dec();
                        }
                    }
                }
            }
        }
        pop_jump_points
    }

    // WITH implementations are in codegen_v307.rs, codegen_v309.rs, codegen_v311.rs

    fn emit_call(&mut self, call: Call) {
        log!(info "entered {} ({call})", fn_name!());
        let init_stack_len = self.stack_len();
        // Python cannot distinguish at compile time between a method call and a attribute call
        if let Some(attr_name) = call.attr_name {
            self.emit_call_method(*call.obj, attr_name, call.args);
        } else {
            match *call.obj {
                Expr::Accessor(Accessor::Ident(ident)) if ident.vis().is_private() => {
                    self.emit_call_local(ident, call.args)
                }
                other if other.ref_t().is_poly_meta_type() => {
                    self.emit_expr(other);
                    self.emit_index_args(call.args);
                }
                other => {
                    self.emit_push_null();
                    self.emit_expr(other);
                    self.fixup_push_null_order();
                    self.emit_args_311(call.args, Name);
                }
            }
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_call_local(&mut self, local: Identifier, args: Args) {
        log!(info "entered {}", fn_name!());
        match &local.inspect()[..] {
            "assert" => self.emit_assert_instr(args),
            "Del" => self.emit_del_instr(args),
            "not" => self.emit_not_instr(args),
            "discard" => self.emit_discard_instr(args),
            "Structural" => {
                let mut args = args;
                if self.cfg.no_std {
                    if let Some(base) = args.remove_left_or_key("Type") {
                        self.emit_expr(base);
                    } else {
                        self.emit_load_const(ValueObj::None);
                    }
                    return;
                }
                if !self.subtype_op_loaded {
                    self.load_type_cmp_ops();
                }
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::private("#StructuralType"));
                self.fixup_push_null_order();
                self.emit_args_311(args, Name);
            }
            "for" | "for!" => self.emit_for_instr(args),
            "while!" => self.emit_while_instr(args),
            "if" | "if!" => self.emit_if_instr(args),
            "match" | "match!" => self.emit_match_instr(args, true),
            "with!" => match self.opcode_set {
                OpcodeSetVersion::V311
                | OpcodeSetVersion::V312
                | OpcodeSetVersion::V313
                | OpcodeSetVersion::V314 => self.emit_with_instr_311(args),
                OpcodeSetVersion::V310 => self.emit_with_instr_310(args),
                OpcodeSetVersion::V309 => self.emit_with_instr_309(args),
                OpcodeSetVersion::V308 => {
                    if self.py_version.minor == Some(7) {
                        self.emit_with_instr_307(args)
                    } else {
                        self.emit_with_instr_308(args)
                    }
                }
            },
            "sum" if self.py_version.minor <= Some(7) && args.get_kw("start").is_some() => {
                self.load_builtins();
                self.emit_load_name_instr(Identifier::private("#sum"));
                self.emit_args_311(args, Name);
            }
            "ListIterator" => {
                let list = Expr::Literal(Literal::new(ValueObj::List(vec![].into()), Token::DUMMY));
                let iter = Identifier::static_public("iter");
                let iter_call = iter.call(Args::single(PosArg::new(list)));
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(iter_call.into())));
                self.emit_call(typ_call);
            }
            "SetIterator" => {
                let set = Expr::Set(Set::empty());
                let iter = Identifier::static_public("iter");
                let iter_call = iter.call(Args::single(PosArg::new(set)));
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(iter_call.into())));
                self.emit_call(typ_call);
            }
            "DictItems" => {
                let dict = Expr::Dict(Dict::empty());
                let iter = Identifier::static_public("iter");
                let items_call = iter.call(Args::single(PosArg::new(dict)));
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(items_call.into())));
                self.emit_call(typ_call);
            }
            "DictKeys" => {
                let dict = Expr::Dict(Dict::empty());
                let keys = Identifier::static_public("keys");
                let keys_call = dict.method_call(keys, Args::empty());
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(keys_call.into())));
                self.emit_call(typ_call);
            }
            "DictValues" => {
                let dict = Expr::Dict(Dict::empty());
                let values = Identifier::static_public("values");
                let values_call = dict.method_call(values, Args::empty());
                let typ = Identifier::static_public("type");
                let typ_call = typ.call(Args::single(PosArg::new(values_call.into())));
                self.emit_call(typ_call);
            }
            other if local.ref_t().is_poly_meta_type() && other != "classof" => {
                if !self.opcode_set.is_3_10_plus() {
                    self.load_fake_generic();
                    self.emit_load_name_instr(Identifier::private("#FakeGenericAlias"));
                    let mut args = args;
                    args.insert_pos(0, PosArg::new(Expr::Accessor(Accessor::Ident(local))));
                    self.emit_args_311(args, Name);
                } else {
                    self.emit_load_name_instr(local);
                    self.emit_index_args(args);
                }
            }
            // "pyimport" | "py" are here
            _ => {
                self.emit_push_null();
                self.emit_load_name_instr(local);
                self.fixup_push_null_order();
                self.emit_args_311(args, Name);
            }
        }
    }

    fn emit_call_method(&mut self, obj: Expr, method_name: Identifier, args: Args) {
        log!(info "entered {}", fn_name!());
        match &method_name.inspect()[..] {
            "return" if obj.ref_t().is_callable() => {
                return self.emit_return_instr(args);
            }
            // TODO: create `Generator` type
            "yield" /* if obj.ref_t().is_callable() */ => {
                return self.emit_yield_instr(args);
            }
            _ => {}
        }
        if let Some(func_name) = debind(&method_name) {
            return self.emit_call_fake_method(obj, func_name, method_name, args);
        }
        let is_type = method_name.ref_t().is_poly_meta_type();
        let kind = if self.opcode_set.is_3_11_plus()
            || (method_name.vi.t.is_method() && args.kw_args.is_empty())
        {
            BoundAttr
        } else {
            UnboundAttr
        };
        self.emit_expr(obj);
        self.emit_load_method_instr(method_name, kind);
        if is_type {
            self.emit_index_args(args);
        } else {
            self.emit_args_311(args, kind);
        }
    }

    // var_args implementations are in codegen_v307.rs (emit_var_args_308, emit_kw_var_args_308)
    // and codegen_v309.rs (emit_var_args_311, emit_kw_var_args_311)

    pub(crate) fn emit_args_311(&mut self, mut args: Args, kind: AccessKind) {
        let argc = args.len();
        let pos_len = args.pos_args.len();
        let kw_len = args.kw_len();
        let use_ex = args.var_args.is_some() || args.kw_var.is_some();
        let mut kws = Vec::with_capacity(kw_len);
        while let Some(arg) = args.try_remove_pos(0) {
            self.emit_expr(arg.expr);
        }
        if use_ex && self.opcode_set.is_3_9_plus() {
            return self.emit_call_function_ex(args, pos_len, kw_len);
        }
        if let Some(var_args) = &args.var_args {
            self.emit_var_args_308(pos_len, var_args);
        }
        while let Some(arg) = args.try_remove_kw(0) {
            kws.push(ValueObj::Str(arg.keyword.content));
            self.emit_expr(arg.expr);
        }
        if let Some(kw_var) = &args.kw_var {
            self.emit_kw_var_args_308(pos_len, kw_var);
        }
        let kwsc = if !kws.is_empty() {
            self.emit_call_kw_instr(argc, kws);
            #[allow(clippy::bool_to_int_with_if)]
            if self.opcode_set.is_3_11_plus() {
                0
            } else {
                1
            }
        } else if args.var_args.is_some() || args.kw_var.is_some() {
            self.write_opcode(CALL_FUNCTION_EX);
            if args.kw_var.is_none() {
                self.write_arg(0);
            } else {
                self.write_arg(1);
            }
            if args.kw_var.is_some() {
                1
            } else {
                0
            }
        } else {
            self.emit_call_instr(argc, kind);
            0
        };
        // (1 (subroutine) + argc + kwsc) input objects -> 1 return object
        self.stack_dec_n((1 + argc + kwsc) - 1);
    }

    /// Call with `*args`/`**kwargs` expansion for Python 3.9+.
    /// Stack layout: `f, NULL (3.11+), callargs: Tuple, kwargs: Dict (or NULL on 3.14)`.
    /// The positional args (`pos_len` items) are already on the stack.
    fn emit_call_function_ex(&mut self, mut args: Args, pos_len: usize, kw_len: usize) {
        if let Some(var_args) = &args.var_args {
            self.emit_var_args_311(pos_len, var_args);
        } else {
            self.write_opcode(BUILD_TUPLE);
            self.write_arg(pos_len);
        }
        self.stack_dec_n(pos_len);
        self.stack_inc(); // the callargs tuple
        if kw_len > 0 {
            while let Some(arg) = args.try_remove_kw(0) {
                self.emit_load_const(ValueObj::Str(arg.keyword.content));
                self.emit_expr(arg.expr);
            }
            self.write_opcode(BUILD_MAP);
            self.write_arg(kw_len);
            self.stack_dec_n(2 * kw_len);
            self.stack_inc(); // the kwargs dict
        }
        if let Some(kw_var) = &args.kw_var {
            self.emit_kw_var_args_311(kw_len > 0, kw_var);
        }
        let has_kwargs = kw_len > 0 || args.kw_var.is_some();
        if self.opcode_set.is_3_14_plus() {
            // 3.14: CALL_FUNCTION_EX always takes 4 stack items (func, NULL, callargs, kwargs);
            // NULL fills the kwargs slot when there are no keyword args
            if !has_kwargs {
                self.write_instr(self.opcode_set.push_null());
                self.write_arg(0);
                self.stack_inc();
            }
            self.write_opcode(CALL_FUNCTION_EX);
            self.write_arg(0);
            // func + NULL + callargs + kwargs -> result
            self.stack_dec_n(3);
        } else {
            self.write_opcode(CALL_FUNCTION_EX);
            self.write_arg(has_kwargs as usize);
            // func (+ NULL on 3.11+) + callargs (+ kwargs) -> result
            let null = self.opcode_set.is_3_11_plus() as usize;
            self.stack_dec_n(null + has_kwargs as usize + 1);
        }
    }

    fn emit_index_args(&mut self, mut args: Args) {
        let argc = args.pos_args.len();
        while let Some(arg) = args.try_remove_pos(0) {
            self.emit_expr(arg.expr);
        }
        if argc > 1 {
            self.write_opcode(BUILD_TUPLE);
            self.write_arg(argc);
        }
        if self.opcode_set.is_3_14_plus() {
            // 3.14: BINARY_SUBSCR removed; use BINARY_OP with NB_SUBSCR=26
            self.write_instr(self.opcode_set.binary_op());
            self.write_arg(26); // NB_SUBSCR
            let cache = self.opcode_set.cache_entries_binary_op() * 2;
            self.write_bytes(&vec![0; cache]);
        } else {
            self.write_instr(self.opcode_set.binary_subscr());
            self.write_arg(0);
            if self.opcode_set.is_3_11_plus() {
                let cache = self.opcode_set.cache_entries_binary_subscr() * 2;
                self.write_bytes(&vec![0; cache]); // CACHE
            }
        }
        // (1 (subroutine) + argc) input objects -> 1 return object
        self.stack_dec_n((1 + argc) - 1);
    }

    // TODO: use exception
    fn emit_return_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if args.is_empty() {
            self.emit_load_const(ValueObj::None);
        } else {
            self.emit_expr(args.remove(0));
        }
        self.write_opcode(RETURN_VALUE);
        self.write_arg(0);
    }

    fn emit_yield_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if args.is_empty() {
            self.emit_load_const(ValueObj::None);
        } else {
            self.emit_expr(args.remove(0));
        }
        self.write_opcode(YIELD_VALUE);
        self.write_arg(0);
    }

    /// 1.abs() => abs(1)
    fn emit_call_fake_method(
        &mut self,
        obj: Expr,
        func_name: Str,
        mut method_name: Identifier,
        mut args: Args,
    ) {
        log!(info "entered {}", fn_name!());
        method_name.raw.vis = VisModifierSpec::Private;
        method_name.vi.py_name = Some(func_name);
        self.emit_push_null();
        self.emit_load_name_instr(method_name);
        self.fixup_push_null_order();
        args.insert_pos(0, PosArg::new(obj));
        self.emit_args_311(args, Name);
    }

    // assert takes 1 or 2 arguments (0: cond, 1: message)
    fn emit_assert_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let init_stack_len = self.stack_len();
        self.emit_expr(args.remove(0));
        self.emit_to_bool();
        let pop_jump_point = self.lasti();
        self.write_opcode(EXTENDED_ARG);
        self.write_arg(0);
        self.write_instr(self.opcode_set.pop_jump_if_true());
        self.write_arg(0);
        let pjc = self.emit_pop_jump_cache();
        self.stack_dec();
        if self.opcode_set.is_3_14_plus() {
            // 3.14: LOAD_ASSERTION_ERROR removed; use LOAD_COMMON_CONSTANT 0
            self.write_instr(self.opcode_set.load_common_constant());
            self.write_arg(0); // 0 = AssertionError
            self.stack_inc();
        } else if self.opcode_set.is_3_10_plus() {
            self.write_instr(self.opcode_set.load_assertion_error());
            self.write_arg(0);
            self.stack_inc();
        } else {
            self.emit_load_global_instr(Identifier::static_public("AssertionError"));
        }
        if let Some(expr) = args.try_remove(0) {
            self.emit_expr(expr);
            self.emit_call_instr(if self.opcode_set.is_3_11_plus() { 0 } else { 1 }, Name);
            if !self.opcode_set.is_3_11_plus() {
                self.stack_dec();
            }
        }
        self.write_opcode(RAISE_VARARGS);
        self.write_arg(1);
        self.stack_dec();
        let idx = if self.opcode_set.is_3_11_plus() {
            self.lasti() - pop_jump_point - 4 - pjc
        } else {
            self.lasti()
        };
        self.fill_jump(pop_jump_point + 1, idx);
        self.emit_load_const(ValueObj::None);
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    // TODO: list comprehension
    fn emit_list(&mut self, list: List) {
        let init_stack_len = self.stack_len();
        if !self.cfg.no_std {
            self.emit_push_null();
            if list.is_unsized() {
                self.emit_load_name_instr(Identifier::static_public("UnsizedList"));
            } else {
                self.emit_load_name_instr(Identifier::static_public("List"));
            }
            self.fixup_push_null_order();
        }
        match list {
            List::Normal(mut lis) => {
                let len = lis.elems.len();
                while let Some(arg) = lis.elems.try_remove_pos(0) {
                    self.emit_expr(arg.expr);
                }
                self.write_opcode(BUILD_LIST);
                self.write_arg(len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(len - 1);
                }
            }
            List::WithLength(ListWithLength {
                elem,
                len: Some(len),
                ..
            }) => {
                self.emit_expr(*elem);
                self.write_opcode(BUILD_LIST);
                self.write_arg(1);
                self.emit_call_instr(1, Name);
                self.stack_dec();
                self.emit_expr(*len);
                self.emit_binop_instr(Token::dummy(TokenKind::Star, "*"), TypePair::ListNat);
                return;
            }
            List::WithLength(ListWithLength {
                elem, len: None, ..
            }) => {
                self.emit_expr(*elem);
            }
            other => todo!("{other}"),
        }
        if !self.cfg.no_std {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    // TODO: tuple comprehension
    // TODO: tuples can be const
    fn emit_tuple(&mut self, tuple: Tuple) {
        let init_stack_len = self.stack_len();
        match tuple {
            Tuple::Normal(mut tup) => {
                let len = tup.elems.len();
                while let Some(arg) = tup.elems.try_remove_pos(0) {
                    self.emit_expr(arg.expr);
                }
                self.write_opcode(BUILD_TUPLE);
                self.write_arg(len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(len - 1);
                }
            }
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_set(&mut self, set: crate::hir::Set) {
        let init_stack_len = self.stack_len();
        match set {
            crate::hir::Set::Normal(mut set) => {
                let len = set.elems.len();
                while let Some(arg) = set.elems.try_remove_pos(0) {
                    self.emit_expr(arg.expr);
                }
                self.write_opcode(BUILD_SET);
                self.write_arg(len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(len - 1);
                }
            }
            crate::hir::Set::WithLength(st) => {
                self.emit_expr(*st.elem);
                self.write_opcode(BUILD_SET);
                self.write_arg(1);
            }
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_dict(&mut self, dict: crate::hir::Dict) {
        let init_stack_len = self.stack_len();
        if !self.cfg.no_std {
            self.emit_push_null();
            self.emit_load_name_instr(Identifier::static_public("Dict"));
            self.fixup_push_null_order();
        }
        match dict {
            crate::hir::Dict::Normal(dic) => {
                let len = dic.kvs.len();
                for kv in dic.kvs.into_iter() {
                    self.emit_expr(kv.key);
                    self.emit_expr(kv.value);
                }
                self.write_opcode(BUILD_MAP);
                self.write_arg(len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(2 * len - 1);
                }
            }
            other => todo!("{other}"),
        }
        if !self.cfg.no_std {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    #[allow(clippy::identity_op)]
    fn emit_record(&mut self, rec: Record) {
        log!(info "entered {} ({rec})", fn_name!());
        let init_stack_len = self.stack_len();
        let attrs_len = rec.attrs.len();
        self.emit_push_null();
        // making record type
        let ident = Identifier::private("#NamedTuple");
        self.emit_load_name_instr(ident);
        self.fixup_push_null_order();
        // record name, let it be anonymous
        self.emit_load_const("Record");
        for field in rec.attrs.iter() {
            self.emit_load_const(ValueObj::Str(field.sig.ident().inspect().clone()));
        }
        self.write_opcode(BUILD_LIST);
        self.write_arg(attrs_len);
        if attrs_len == 0 {
            self.stack_inc();
        } else {
            self.stack_dec_n(attrs_len - 1);
        }
        self.emit_call_instr(2, Name);
        // (1 (subroutine) + argc + kwsc) input objects -> 1 return object
        self.stack_dec_n((1 + 2 + 0) - 1);
        let ident = Identifier::private("#rec");
        self.emit_store_instr(ident, Name);
        // making record instance
        let ident = Identifier::private("#rec");
        self.emit_push_null();
        self.emit_load_name_instr(ident);
        self.fixup_push_null_order();
        for field in rec.attrs.into_iter() {
            self.emit_simple_block(field.body.block);
        }
        self.emit_call_instr(attrs_len, Name);
        // (1 (subroutine) + argc + kwsc) input objects -> 1 return object
        self.stack_dec_n((1 + attrs_len + 0) - 1);
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    /// Emits independent code blocks (e.g., linked other modules)
    fn emit_code(&mut self, code: Block) {
        let mut gen = self.inherit();
        let code = gen.emit_block(code, vec![], None, vec![], 0, vec![], 0);
        self.emit_load_const(code);
    }

    pub(crate) fn get_root(acc: &Accessor) -> Identifier {
        match acc {
            Accessor::Ident(ident) => ident.clone(),
            Accessor::Attr(attr) => {
                if let Expr::Accessor(acc) = attr.obj.as_ref() {
                    Self::get_root(acc)
                } else {
                    todo!("{:?}", attr.obj)
                }
            }
        }
    }

    fn emit_import(&mut self, acc: Accessor) {
        self.emit_load_const(0i32);
        self.emit_load_const(ValueObj::None);
        let full_name = Str::from(
            acc.qual_name()
                .map_or(acc.show(), |s| s.replace(".__init__", "")),
        );
        let name = self
            .local_search(&full_name, Name)
            .unwrap_or_else(|| self.register_name(full_name, Import));
        self.write_opcode(IMPORT_NAME);
        self.write_arg(name.idx);
        let root = Self::get_root(&acc);
        self.emit_store_instr(root, Name);
        self.stack_dec();
    }

    fn emit_compound(&mut self, chunks: Block) {
        let is_module_loading_chunks = chunks
            .get(2)
            .map(|chunk| {
                option_enum_unwrap!(chunk, Expr::Call)
                    .map(|call| call.obj.show_acc().as_ref().map(|s| &s[..]) == Some("exec"))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !self.module_type_loaded && is_module_loading_chunks {
            self.load_module_type();
            self.module_type_loaded = true;
        }
        let init_stack_len = self.stack_len();
        for chunk in chunks.into_iter() {
            self.emit_chunk(chunk);
            if self.stack_len() == init_stack_len + 1 {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
    }

    /// See `cpython/Object/lnotab_notes.txt` in details
    fn push_lnotab(&mut self, expr: &Expr) {
        let ln_begin = expr.ln_begin().unwrap_or(0);
        if ln_begin > self.cur_block().prev_lineno {
            let mut sd = self.lasti() - self.cur_block().prev_lasti;
            let mut ld = ln_begin - self.cur_block().prev_lineno;
            if ld != 0 {
                if sd != 0 {
                    while sd > 254 {
                        self.mut_cur_block_codeobj().lnotab.push(255);
                        self.mut_cur_block_codeobj().lnotab.push(0);
                        sd -= 254;
                    }
                    while ld > 127 {
                        self.mut_cur_block_codeobj().lnotab.push(0);
                        self.mut_cur_block_codeobj().lnotab.push(127);
                        ld -= 127;
                    }
                    self.mut_cur_block_codeobj().lnotab.push(sd as u8);
                    self.mut_cur_block_codeobj().lnotab.push(ld as u8);
                } else {
                    // empty lines
                    if let Some(&last_ld) = self.cur_block_codeobj().lnotab.last() {
                        if last_ld as u32 + ld > 127 {
                            *self.mut_cur_block_codeobj().lnotab.last_mut().unwrap() = 127;
                            self.mut_cur_block_codeobj().lnotab.push(0);
                            ld -= 127;
                        }
                        while last_ld as u32 + ld > 127 {
                            self.mut_cur_block_codeobj().lnotab.push(127);
                            self.mut_cur_block_codeobj().lnotab.push(0);
                            ld -= 127;
                        }
                        self.mut_cur_block_codeobj().lnotab.push(ld as u8);
                    } else {
                        // a block starts with an empty line
                        self.mut_cur_block_codeobj().lnotab.push(0);
                        self.mut_cur_block_codeobj()
                            .lnotab
                            .push(u8::try_from(ld).unwrap());
                    }
                }
                self.mut_cur_block().prev_lineno += ld;
                self.mut_cur_block().prev_lasti = self.lasti();
            } else {
                CompileError::compiler_bug(
                    0,
                    self.cfg.input.clone(),
                    expr.loc(),
                    fn_name_full!(),
                    line!(),
                )
                .write_to_stderr();
                self.crash("codegen failed: invalid bytecode format");
            }
        }
    }

    fn emit_chunk(&mut self, chunk: Expr) {
        log!(info "entered {} ({chunk})", fn_name!());
        self.push_lnotab(&chunk);
        match chunk {
            Expr::Literal(lit) => self.emit_literal(lit),
            Expr::Accessor(acc) => self.emit_acc(acc),
            Expr::Def(def) => self.emit_def(def),
            Expr::ClassDef(class) => self.emit_class_def(class),
            Expr::PatchDef(patch) => self.emit_patch_def(patch),
            Expr::ReDef(attr) => self.emit_redef(attr),
            Expr::Lambda(lambda) => self.emit_lambda(lambda),
            Expr::UnaryOp(unary) => self.emit_unaryop(unary),
            Expr::BinOp(bin) => self.emit_binop(bin),
            Expr::Call(call) => self.emit_call(call),
            Expr::List(lis) => self.emit_list(lis),
            Expr::Tuple(tup) => self.emit_tuple(tup),
            Expr::Set(set) => self.emit_set(set),
            Expr::Dict(dict) => self.emit_dict(dict),
            Expr::Record(rec) => self.emit_record(rec),
            Expr::Code(code) => self.emit_code(code),
            Expr::Compound(chunks) => self.emit_compound(chunks),
            Expr::Import(acc) => self.emit_import(acc),
            Expr::Dummy(_) | Expr::TypeAsc(_) => {}
        }
    }

    pub(crate) fn emit_expr(&mut self, expr: Expr) {
        log!(info "entered {} ({expr})", fn_name!());
        self.push_lnotab(&expr);
        let init_stack_len = self.stack_len();
        let mut wrapped = true;
        if !self.cfg.no_std && expr.should_wrap() {
            match expr.ref_t().derefine() {
                v @ (Bool | Nat | Int | Float | Str) => {
                    self.emit_push_null();
                    self.emit_load_name_instr(Identifier::public(&v.qual_name()));
                    self.fixup_push_null_order();
                }
                other => match &other.qual_name()[..] {
                    t @ ("Bytes" | "List" | "Dict" | "Set") => {
                        self.emit_push_null();
                        self.emit_load_name_instr(Identifier::public(t));
                        self.fixup_push_null_order();
                    }
                    _ => {
                        wrapped = false;
                    }
                },
            }
        } else {
            wrapped = false;
        }
        match expr {
            Expr::Literal(lit) => self.emit_literal(lit),
            Expr::Accessor(acc) => self.emit_acc(acc),
            Expr::Def(def) => self.emit_def(def),
            Expr::ClassDef(class) => self.emit_class_def(class),
            Expr::PatchDef(patch) => self.emit_patch_def(patch),
            Expr::ReDef(attr) => self.emit_redef(attr),
            Expr::Lambda(lambda) => self.emit_lambda(lambda),
            Expr::UnaryOp(unary) => self.emit_unaryop(unary),
            Expr::BinOp(bin) => self.emit_binop(bin),
            Expr::Call(call) => self.emit_call(call),
            Expr::List(lis) => self.emit_list(lis),
            Expr::Tuple(tup) => self.emit_tuple(tup),
            Expr::Set(set) => self.emit_set(set),
            Expr::Dict(dict) => self.emit_dict(dict),
            Expr::Record(rec) => self.emit_record(rec),
            Expr::Code(code) => self.emit_code(code),
            Expr::Compound(chunks) => self.emit_compound(chunks),
            Expr::TypeAsc(tasc) => self.emit_expr(*tasc.expr),
            Expr::Import(acc) => self.emit_import(acc),
            Expr::Dummy(_) => {}
        }
        if !self.cfg.no_std && wrapped {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    /// forブロックなどで使う
    fn emit_control_block(&mut self, block: Block, params: Params) {
        log!(info "entered {}", fn_name!());
        let param_names = self.gen_param_names(&params);
        let captured = self.captured_param_flags(&params);
        let line = block.ln_begin().unwrap_or(0);
        // the name is remade here without its `VarInfo`, which would make it
        // a name rather than a local
        let kind = self.block_param_kind();
        for (param, captured) in param_names.into_iter().zip(captured) {
            let ident = Identifier::public_with_line(DOT, param, line);
            self.emit_store_instr_captured(ident, Name, kind, captured);
        }
        for guard in params.guards {
            if let GuardClause::Bind(bind) = guard {
                self.emit_def(bind);
            }
        }
        if block.is_empty() {
            return;
        }
        let init_stack_len = self.stack_len();
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
    }

    fn emit_simple_block(&mut self, block: Block) {
        log!(info "entered {}", fn_name!());
        if block.is_empty() {
            return;
        }
        let init_stack_len = self.stack_len();
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
    }

    pub(crate) fn emit_with_block(&mut self, block: Block, params: &Params) {
        log!(info "entered {}", fn_name!());
        let param_names = self.gen_param_names(params);
        let captured = self.captured_param_flags(params);
        let line = block.ln_begin().unwrap_or(0);
        let kind = self.block_param_kind();
        for (param, captured) in param_names.into_iter().zip(captured) {
            let ident = Identifier::public_with_line(DOT, param, line);
            self.emit_store_instr_captured(ident, Name, kind, captured);
        }
        let init_stack_len = self.stack_len();
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            // __exit__, __enter__() are on the stack
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
    }

    fn emit_class_block(&mut self, mut class: ClassDef) -> CodeObj {
        log!(info "entered {}", fn_name!());
        let name = class.sig.ident().inspect().clone();
        self.unit_size += 1;
        let firstlineno = match class.methods_list.first().and_then(|def| def.ln_begin()) {
            Some(l) => l,
            None => class.sig.ln_begin().unwrap_or(0),
        };
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            vec![],
            0,
            Str::rc(self.cfg.input.enclosed_name()),
            &name,
            firstlineno,
            0,
        ));
        self.mut_cur_block().kind = UnitKind::ClassBody;
        let init_stack_len = self.stack_len();
        let mod_name = self.toplevel_block_codeobj().name.clone();
        self.emit_load_const(mod_name);
        self.emit_store_instr(Identifier::static_public("__module__"), Name);
        self.emit_load_const(name);
        self.emit_store_instr(Identifier::static_public("__qualname__"), Name);
        if !class.obj.typ().is_monomorphic() && self.opcode_set.is_3_10_plus() {
            // __class_getitem__ = classmethod(#GenericAlias)
            // (supports runtime type application: `Box[Int].new(...)` forwards to `Box.new(...)`)
            let classmethod = Identifier::static_public("classmethod");
            let generic_alias =
                Expr::Accessor(Accessor::Ident(Identifier::private("#GenericAlias")));
            let call = classmethod.call(Args::single(PosArg::new(generic_alias)));
            self.emit_call(call);
            self.emit_store_instr(Identifier::static_public("__class_getitem__"), Name);
        }
        let mut methods = ClassDef::take_all_methods(class.methods_list);
        let __init__ = methods
            .get_def("__init__")
            .or_else(|| methods.get_def("__init__!"))
            .cloned();
        let is_subclass = matches!(class.obj.as_ref(), GenTypeObj::Subclass(_));
        self.emit_init_method(&class.sig, __init__, class.constructor.clone(), is_subclass);
        match class.gen_new {
            GenNew::Defined => {}
            GenNew::FromCall => self.emit_new_func(&class.sig, class.constructor),
            GenNew::FromSuper => {
                let sup = Self::sup_class_expr(class.obj.as_ref(), class.require_or_sup.take());
                match sup {
                    Some(sup) => self.emit_inherited_new_func(&class.sig, sup),
                    None => self.emit_new_func(&class.sig, class.constructor),
                }
            }
        }
        let __del__ = methods
            .remove_def("__del__")
            .or_else(|| methods.remove_def("__del__!"));
        if let Some(mut __del__) = __del__ {
            __del__.sig.ident_mut().vi.py_name = Some(Str::from("__del__"));
            self.emit_def(__del__);
        }
        if !methods.is_empty() {
            self.emit_simple_block(methods);
        }
        if self.stack_len() == init_stack_len {
            self.emit_load_const(ValueObj::None);
        }
        self.write_opcode(RETURN_VALUE);
        self.write_arg(0);
        if self.stack_len() > 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit_class_block: invalid stack size");
        }
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        self.remap_localsplus();
        // end of flagging
        let unit = self.units.pop().unwrap();
        if !self.units.is_empty() {
            let ld = unit.prev_lineno - self.cur_block().prev_lineno;
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += u8::try_from(ld).unwrap();
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        unit.codeobj
    }

    fn emit_init_method(
        &mut self,
        sig: &Signature,
        __init__: Option<Def>,
        constructor: Type,
        is_subclass: bool,
    ) {
        log!(info "entered {}", fn_name!());
        let line = sig.ln_begin().unwrap_or(0);
        let class_name = sig.ident().inspect();
        let mut ident = Identifier::public_with_line(DOT, Str::ever("__init__"), line);
        ident.vi.t = constructor.clone();
        let new_first_param = constructor.non_default_params().unwrap().first();
        // A subclass that adds no field of its own and has no `__init__!` body needs no
        // `__init__` at all: Python's inheritance already forwards to the superclass's.
        // The forwarder that used to be generated could not name the class it belongs to
        // -- `super(type(self), self)` is the *instance's* class, so a third class down
        // the chain called the same frame again and recursed until the stack ran out.
        if is_subclass && new_first_param.is_none() && __init__.is_none() {
            return;
        }
        // For Inherit classes where the parent has no non-default params (e.g., Python classes
        // like TestCase whose __init__ only has default params), use *args/**kwargs + super().
        // For Inherit classes with explicit params (Erg native), keep field-assignment approach.
        if is_subclass && new_first_param.is_none() {
            // def __init__(self, *args, **kwargs):
            //     super(type(self), self).__init__(*args, **kwargs)
            //     <user __init__ body>
            let self_param = VarName::from_str_and_line(Str::ever("self"), line);
            let vi = VarInfo::nd_parameter(
                constructor.return_t().unwrap().clone(),
                ident.vi.def_loc.clone(),
                "?".into(),
            );
            let raw = erg_parser::ast::NonDefaultParamSignature::new(
                ParamPattern::VarName(self_param),
                None,
            );
            let self_param = NonDefaultParamSignature::new(raw, vi.clone(), None);
            // *args
            let args_var = VarName::from_str_and_line(Str::ever("args"), line);
            let raw_args = erg_parser::ast::NonDefaultParamSignature::new(
                ParamPattern::VarName(args_var),
                None,
            );
            let args_param = NonDefaultParamSignature::new(raw_args, vi.clone(), None);
            // **kwargs
            let kwargs_var = VarName::from_str_and_line(Str::ever("kwargs"), line);
            let raw_kwargs = erg_parser::ast::NonDefaultParamSignature::new(
                ParamPattern::VarName(kwargs_var),
                None,
            );
            let kwargs_param = NonDefaultParamSignature::new(raw_kwargs, vi, None);
            let params = Params::new(
                vec![self_param],
                Some(Box::new(args_param)),
                vec![],
                Some(Box::new(kwargs_param)),
                vec![],
                None,
            );
            let bounds = TypeBoundSpecs::empty();
            let subr_sig = SubrSignature::new(
                set! {},
                ident,
                bounds,
                params,
                sig.t_spec_with_op().cloned(),
                vec![],
            );
            let mut attrs = vec![];
            if let Some(__init__) = __init__ {
                attrs.extend(__init__.body.block.clone());
            }
            let none = Token::new_fake(TokenKind::NoneLit, "None", line, 0, 0);
            attrs.push(Expr::Literal(Literal::new(ValueObj::None, none)));
            let block = Block::new(attrs);
            let body = DefBody::new(EQUAL, block, DefId(0));
            self.emit_subclass_init_def(Some(class_name), subr_sig, body);
        } else {
            let self_param = VarName::from_str_and_line(Str::ever("self"), line);
            let vi = VarInfo::nd_parameter(
                constructor.return_t().unwrap().clone(),
                ident.vi.def_loc.clone(),
                "?".into(),
            );
            let raw = erg_parser::ast::NonDefaultParamSignature::new(
                ParamPattern::VarName(self_param),
                None,
            );
            let self_param = NonDefaultParamSignature::new(raw, vi, None);
            let (param_name, params) = if let Some(new_first_param) = new_first_param {
                let param_name = new_first_param
                    .name()
                    .cloned()
                    .unwrap_or_else(|| self.fresh_gen.fresh_varname());
                let param = VarName::from_str_and_line(param_name.clone(), line);
                let raw = erg_parser::ast::NonDefaultParamSignature::new(
                    ParamPattern::VarName(param),
                    None,
                );
                let vi = VarInfo::nd_parameter(
                    new_first_param.typ().clone(),
                    ident.vi.def_loc.clone(),
                    "?".into(),
                );
                let param = NonDefaultParamSignature::new(raw, vi, None);
                let params = Params::new(vec![self_param, param], None, vec![], None, vec![], None);
                (param_name, params)
            } else {
                ("_".into(), Params::single(self_param))
            };
            let bounds = TypeBoundSpecs::empty();
            let subr_sig = SubrSignature::new(
                set! {},
                ident,
                bounds,
                params,
                sig.t_spec_with_op().cloned(),
                vec![],
            );
            let mut attrs = vec![];
            match new_first_param.map(|pt| pt.typ()) {
                // namedtupleは仕様上::xなどの名前を使えない
                // {x = Int; y = Int}
                //   => self::x = %x.x; self::y = %x.y
                // {.x = Int; .y = Int}
                //   => self.x = %x.x; self.y = %x.y
                // () => pass
                Some(Type::Record(rec)) => {
                    for field in rec.keys() {
                        let obj = Expr::Accessor(Accessor::private_with_line(
                            Str::from(&param_name),
                            line,
                        ));
                        let ident = erg_parser::ast::Identifier::public(field.symbol.clone());
                        let expr = obj.attr_expr(Identifier::bare(ident));
                        let obj =
                            Expr::Accessor(Accessor::private_with_line(Str::ever("self"), line));
                        let dot = if field.vis.is_private() {
                            VisModifierSpec::Private
                        } else {
                            VisModifierSpec::Public(Location::Unknown)
                        };
                        let attr = erg_parser::ast::Identifier::new(
                            dot,
                            VarName::from_str(field.symbol.clone()),
                        );
                        let attr = obj.attr(Identifier::bare(attr));
                        let redef = ReDef::new(attr, Block::new(vec![expr]));
                        attrs.push(Expr::ReDef(redef));
                    }
                }
                // self::base = %x
                Some(_) => {
                    let expr =
                        Expr::Accessor(Accessor::private_with_line(Str::from(&param_name), line));
                    let obj = Expr::Accessor(Accessor::private_with_line(Str::ever("self"), line));
                    let attr = obj.attr(Identifier::private_with_line(Str::ever("base"), line));
                    let redef = ReDef::new(attr, Block::new(vec![expr]));
                    attrs.push(Expr::ReDef(redef));
                }
                None => {}
            }
            if let Some(__init__) = __init__ {
                attrs.extend(__init__.body.block.clone());
            }
            let none = Token::new_fake(TokenKind::NoneLit, "None", line, 0, 0);
            attrs.push(Expr::Literal(Literal::new(ValueObj::None, none)));
            let block = Block::new(attrs);
            let body = DefBody::new(EQUAL, block, DefId(0));
            self.emit_subr_def(Some(class_name), subr_sig, body);
        }
    }

    /// Like `emit_subr_def`, but injects `super().__init__(*args, **kwargs)` at the
    /// start of the function body. Used for `Inherit` (subclass) `__init__` methods.
    fn emit_subclass_init_def(
        &mut self,
        class_name: Option<&str>,
        sig: SubrSignature,
        body: DefBody,
    ) {
        log!(info "entered {} ({sig} = {})", fn_name!(), body.block);
        let name = sig.ident.inspect().clone();
        let params = self.gen_param_names(&sig.params);
        let cell_names = self.frame_cells(&sig.params, &body.block);
        // No defaults to emit for *args, **kwargs
        let make_function_flag = 0;
        let mut flags = 0;
        if sig.params.var_params.is_some() {
            flags += CodeObjFlags::VarArgs as u32;
        }
        if sig.params.kw_var_params.is_some() {
            flags += CodeObjFlags::VarKeywords as u32;
        }
        let code = self.emit_init_block_with_super(
            body.block,
            Some(name.clone()),
            params,
            cell_names,
            flags,
        );
        self.emit_load_const(code);
        if !self.opcode_set.is_3_11_plus() {
            if let Some(class) = class_name {
                self.emit_load_const(Str::from(format!("{class}.{name}")));
            } else {
                self.emit_load_const(name);
            }
        } else {
            self.stack_inc();
        }
        self.emit_make_function(make_function_flag);
        // stack_dec: <code obj> + <name> -> <function>
        self.stack_dec();
        self.emit_store_instr(sig.ident, Name);
    }

    /// Like `emit_block`, but emits `super().__init__(*args, **kwargs)` before the block body.
    fn emit_init_block_with_super(
        &mut self,
        block: Block,
        opt_name: Option<Str>,
        params: Vec<Str>,
        cell_names: Vec<Str>,
        flags: u32,
    ) -> CodeObj {
        log!(info "entered {}", fn_name!());
        self.unit_size += 1;
        let name = if let Some(name) = opt_name {
            name
        } else {
            self.fresh_gen.fresh_varname()
        };
        let firstlineno = block
            .first()
            .and_then(|first| first.ln_begin())
            .unwrap_or_else(|| {
                // Fallback: use parent unit's lineno to avoid subtraction overflow
                self.cur_block().prev_lineno
            });
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            params,
            0, // kwonlyargcount
            Str::rc(self.cfg.input.enclosed_name()),
            name,
            firstlineno,
            flags,
        ));
        self.emit_frame_cells(cell_names);
        let idx_copy_free_vars = if self.opcode_set.is_3_11_plus() {
            let idx_copy_free_vars = self.lasti();
            self.write_instr(self.opcode_set.copy_free_vars());
            self.write_arg(0);
            self.write_instr(self.opcode_set.resume());
            self.write_arg(0);
            idx_copy_free_vars
        } else {
            0
        };
        let init_stack_len = self.stack_len();
        // Inject super().__init__(*args, **kwargs)
        self.emit_super_init_bytecode();
        // Emit the rest of the block (user's __init__ body + return None)
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
        if self.stack_len() == init_stack_len {
            self.emit_load_const(ValueObj::None);
        } else if self.stack_len() > init_stack_len + 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit_init_block_with_super: invalid stack size");
        }
        self.write_opcode(RETURN_VALUE);
        self.write_arg(0);
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        let freevars_len = self.cur_block_codeobj().freevars.len();
        if freevars_len > 0 {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::Nested as u32;
            if self.opcode_set.is_3_11_plus() {
                self.edit_code(idx_copy_free_vars + 1, freevars_len);
            }
        } else if self.opcode_set.is_3_11_plus() {
            let nop = self.opcode_set.translate_common(CommonOpcode::NOP as u8);
            self.edit_code(idx_copy_free_vars, nop as usize);
        }
        // end of flagging
        self.remap_localsplus();
        let unit = self.units.pop().unwrap();
        if !self.units.is_empty() {
            let ld = unit
                .prev_lineno
                .saturating_sub(self.cur_block().prev_lineno);
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += u8::try_from(ld).unwrap_or(0);
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        unit.codeobj
    }

    /// Emit raw bytecode for `super(type(self), self).__init__(*args, **kwargs)`.
    /// Uses `type(self)` instead of zero-arg `super()` to avoid needing `__class__` cell.
    /// The function parameters must be (self, *args, **kwargs) with LOAD_FAST indices 0, 1, 2.
    fn emit_super_init_bytecode(&mut self) {
        // Step 1: super(type(self), self)
        // Outer call setup: push super callable
        self.emit_push_null(); // NULL sentinel for 3.11+
        let super_ident = Identifier::static_public("super");
        self.emit_load_name_instr(super_ident);
        self.fixup_push_null_order();
        // Inner call: type(self) — first arg to super
        self.emit_push_null(); // NULL sentinel for type() call
        let type_ident = Identifier::static_public("type");
        self.emit_load_name_instr(type_ident);
        self.fixup_push_null_order();
        self.write_opcode(LOAD_FAST);
        self.write_arg(0); // self
        self.stack_inc();
        // Call type(self) with argc=1
        self.emit_call_instr(1, Name);
        self.stack_dec_n(1); // argc=1 arg consumed
                             // Stack: [..., super, type_of_self] (NULL consumed by CALL)
                             // Load self again as second argument to super
        self.write_opcode(LOAD_FAST);
        self.write_arg(0); // self
        self.stack_inc();
        // Call super(type_of_self, self) with argc=2
        self.emit_call_instr(2, Name);
        self.stack_dec_n(2); // argc=2 args consumed
                             // Stack: [super_instance]

        // Step 2: Load __init__ attribute (NOT method mode — regular attr access)
        // Returns a bound method suitable for CALL_FUNCTION_EX
        let init_ident = Identifier::static_public("__init__");
        self.emit_load_attr_instr(init_ident);
        // Stack: [bound_init_method]
        // `CALL_FUNCTION_EX` reserves a slot next to the callable, and 3.13 moved
        // it from before to after. An attribute load leaves only the bound
        // method, so fill it here or the argument tuple slides into the callable
        // slot and the keyword dict is called instead.
        let null = self.opcode_set.is_3_13_plus();
        if null {
            self.write_instr(self.opcode_set.push_null());
            self.write_arg(0);
            self.stack_inc();
        }

        // Step 3: Load *args and **kwargs
        self.write_opcode(LOAD_FAST);
        self.write_arg(1); // args (index 1, after self=0)
        self.stack_inc();
        self.write_opcode(LOAD_FAST);
        self.write_arg(2); // kwargs (index 2)
        self.stack_inc();
        // Stack: [bound_init_method, args_tuple, kwargs_dict]

        // Step 4: CALL_FUNCTION_EX with kwargs (flag=1; 3.14 dropped the flag,
        // where the keyword slot is always present)
        self.write_opcode(CALL_FUNCTION_EX);
        self.write_arg(!self.opcode_set.is_3_14_plus() as usize);
        // consumes func (+ NULL) + args + kwargs, produces the result
        self.stack_dec_n(2 + null as usize);
        // Stack: [return_value]

        // Step 5: Discard the return value
        self.emit_pop_top();
        // Stack: []
    }

    /// The expression the superclass of a `Subclass` is referred to by: the `Super`
    /// argument of `Inherit` when it is a name, else the class's own name.
    fn sup_class_expr(obj: &GenTypeObj, require_or_sup: Option<Box<Expr>>) -> Option<Expr> {
        let GenTypeObj::Subclass(sub) = obj else {
            return None;
        };
        require_or_sup
            .map(|expr| *expr)
            .filter(|expr| expr.is_acc())
            .or_else(|| Expr::try_from_type(sub.sup.typ().derefine()).ok())
    }

    /// The `new` of a subclass whose superclass has a `new` of its own (`GenNew::FromSuper`).
    /// The type checker gave it the superclass's signature, so the arguments are forwarded
    /// as they are, and the subclass is made from the object the superclass's `new` built
    /// (the fields `__init__` copies are the same):
    ///
    /// ```python
    /// class D(C):
    ///     def new(*args, **kwargs): return D(C.new(*args, **kwargs))
    /// ```
    fn emit_inherited_new_func(&mut self, sig: &Signature, sup: Expr) {
        log!(info "entered {} ({sup})", fn_name!());
        let class_ident = sig.ident();
        let line = sig.ln_begin().unwrap_or(0);
        let mut ident = Identifier::public_with_line(DOT, Str::ever("new"), line);
        ident.vi.t = Type::ClassType;
        let mut callee_ident = class_ident.clone();
        callee_ident.vi.t = Type::ClassType;
        let class = Expr::Accessor(Accessor::Ident(callee_ident));
        let param = |name: &'static str| {
            let var = VarName::from_str_and_line(Str::ever(name), line);
            let vi = VarInfo::nd_parameter(Type::Obj, ident.vi.def_loc.clone(), "?".into());
            let raw =
                erg_parser::ast::NonDefaultParamSignature::new(ParamPattern::VarName(var), None);
            NonDefaultParamSignature::new(raw, vi, None)
        };
        let params = Params::new(
            vec![],
            Some(Box::new(param("args"))),
            vec![],
            Some(Box::new(param("kwargs"))),
            vec![],
            None,
        );
        let sup_new = Expr::Accessor(Accessor::Attr(Attribute::new(
            sup,
            Identifier::public_with_line(DOT, Str::ever("new"), line),
        )));
        let forwarded = Args::new(
            vec![],
            Some(PosArg::new(Expr::Accessor(Accessor::public_with_line(
                Str::ever("args"),
                line,
            )))),
            vec![],
            Some(PosArg::new(Expr::Accessor(Accessor::public_with_line(
                Str::ever("kwargs"),
                line,
            )))),
            None,
        );
        let built = sup_new.call_expr(forwarded);
        let call = class.call_expr(Args::single(PosArg::new(built)));
        let sig = SubrSignature::new(
            set! {},
            ident,
            TypeBoundSpecs::empty(),
            params,
            sig.t_spec_with_op().cloned(),
            vec![],
        );
        let body = DefBody::new(EQUAL, Block::new(vec![call]), DefId(0));
        self.emit_subr_def(Some(class_ident.inspect()), sig, body);
    }

    /// ```python
    /// class C:
    ///     # constructor => C
    ///     def new(x): return C(x)
    /// ```
    fn emit_new_func(&mut self, sig: &Signature, constructor: Type) {
        log!(info "entered {}", fn_name!());
        let class_ident = sig.ident();
        let line = sig.ln_begin().unwrap_or(0);
        let mut ident = Identifier::public_with_line(DOT, Str::ever("new"), line);
        // NOTE: for a polymorphic class, the identifier's type is a poly meta type
        // (e.g. `|T|({T}) -> {Box(T)}`), which would be emitted as a subscript
        // (`Box[x]`). The generated `new` must call the constructor directly.
        let mut callee_ident = class_ident.clone();
        callee_ident.vi.t = Type::ClassType;
        let class = Expr::Accessor(Accessor::Ident(callee_ident));
        ident.vi.t = constructor;
        if let Ok(subr) = <&SubrType>::try_from(&ident.vi.t) {
            let mut params = Params::empty();
            let mut args = Args::empty();
            for nd_param in subr.non_default_params.iter() {
                let param_name = nd_param
                    .name()
                    .cloned()
                    .unwrap_or_else(|| self.fresh_gen.fresh_varname());
                let param = VarName::from_str_and_line(param_name.clone(), line);
                let vi = VarInfo::nd_parameter(
                    nd_param.typ().clone(),
                    ident.vi.def_loc.clone(),
                    "?".into(),
                );
                let raw = erg_parser::ast::NonDefaultParamSignature::new(
                    ParamPattern::VarName(param),
                    None,
                );
                let param = NonDefaultParamSignature::new(raw, vi, None);
                params.push_non_default(param);
                let arg = PosArg::new(Expr::Accessor(Accessor::public_with_line(param_name, line)));
                args.push_pos(arg);
            }
            // FIXME: var params, default params, kw var params
            let bounds = TypeBoundSpecs::empty();
            let sig = SubrSignature::new(
                set! {},
                ident,
                bounds,
                params,
                sig.t_spec_with_op().cloned(),
                vec![],
            );
            let call = class.call_expr(args);
            let block = Block::new(vec![call]);
            let body = DefBody::new(EQUAL, block, DefId(0));
            self.emit_subr_def(Some(class_ident.inspect()), sig, body);
        } else {
            let params = Params::empty();
            let bounds = TypeBoundSpecs::empty();
            let sig = SubrSignature::new(
                set! {},
                ident,
                bounds,
                params,
                sig.t_spec_with_op().cloned(),
                vec![],
            );
            let call = class.call_expr(Args::empty());
            let block = Block::new(vec![call]);
            let body = DefBody::new(EQUAL, block, DefId(0));
            self.emit_subr_def(Some(class_ident.inspect()), sig, body);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_block(
        &mut self,
        block: Block,
        guards: Vec<GuardClause>,
        opt_name: Option<Str>,
        params: Vec<Str>,
        kwonlyargcount: u32,
        cell_names: Vec<Str>,
        flags: u32,
    ) -> CodeObj {
        log!(info "entered {}", fn_name!());
        self.unit_size += 1;
        let name = if let Some(name) = opt_name {
            name
        } else {
            self.fresh_gen.fresh_varname()
        };
        let firstlineno = block
            .first()
            .and_then(|first| first.ln_begin())
            .unwrap_or(0);
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            params,
            kwonlyargcount,
            Str::rc(self.cfg.input.enclosed_name()),
            name,
            firstlineno,
            flags,
        ));
        self.emit_frame_cells(cell_names);
        let idx_copy_free_vars = if self.opcode_set.is_3_11_plus() {
            let idx_copy_free_vars = self.lasti();
            self.write_instr(self.opcode_set.copy_free_vars());
            self.write_arg(0);
            self.write_instr(self.opcode_set.resume());
            self.write_arg(0);
            idx_copy_free_vars
        } else {
            0
        };
        let init_stack_len = self.stack_len();
        for guard in guards {
            if let GuardClause::Bind(bind) = guard {
                self.emit_def(bind);
            }
        }
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            // NOTE: 各行のトップレベルでは0個または1個のオブジェクトが残っている
            // Pythonの場合使わなかったオブジェクトはそのまま捨てられるが、Ergではdiscardを使う必要がある
            // TODO: discard
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top(); // 最後の値は戻り値として取っておく
        if self.stack_len() == init_stack_len {
            self.emit_load_const(ValueObj::None);
        } else if self.stack_len() > init_stack_len + 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit_block: invalid stack size");
        }
        self.write_opcode(RETURN_VALUE);
        self.write_arg(0);
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        let freevars_len = self.cur_block_codeobj().freevars.len();
        if freevars_len > 0 {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::Nested as u32;
            if self.opcode_set.is_3_11_plus() {
                self.edit_code(idx_copy_free_vars + 1, freevars_len);
            }
        } else if self.opcode_set.is_3_11_plus() {
            // cancel copying
            let code = self.cur_block_codeobj().code.get(idx_copy_free_vars);
            debug_assert_eq!(code, Some(&self.opcode_set.copy_free_vars()));
            // Use translated NOP value (3.13+: opcodes are renumbered)
            let nop = self.opcode_set.translate_common(CommonOpcode::NOP as u8);
            self.edit_code(idx_copy_free_vars, nop as usize);
        }
        self.remap_localsplus();
        // end of flagging
        let unit = self.units.pop().unwrap();
        // increase lineno
        if !self.units.is_empty() {
            let ld = unit
                .prev_lineno
                .saturating_sub(self.cur_block().prev_lineno);
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += u8::try_from(ld).unwrap();
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        unit.codeobj
    }

    fn load_prelude(&mut self) {
        // NOTE: Integers need to be used in IMPORT_NAME
        // but `Int` are called before importing it, so they need to be no_std mode
        let no_std = self.cfg.no_std;
        self.cfg.no_std = true;
        self.load_record_type();
        self.load_prelude_py();
        self.prelude_loaded = true;
        self.record_type_loaded = true;
        self.err_ops_loaded = true;
        self.cfg.no_std = no_std;
    }

    fn load_contains_op(&mut self) {
        let mod_name = Identifier::static_public("_erg_std_prelude");
        self.emit_global_import_items(
            mod_name,
            vec![(
                Identifier::static_public("contains_operator"),
                Some(Identifier::private("#contains_operator")),
            )],
        );
        self.contains_op_loaded = true;
    }

    /// `Nat < Int`, `{=} > {x = Int}`, etc.: both operands are type objects.
    /// Record type literals have meta-type `Record` / `RecordMetaType`, not `Type`.
    fn is_type_inclusion_cmp(bin: &BinOp) -> bool {
        let op_lhs = bin.lhs_t().derefine();
        let op_rhs = bin.rhs_t().derefine();
        if op_lhs == Type && op_rhs == Type {
            return true;
        }
        Self::is_type_object_t(bin.lhs.ref_t()) && Self::is_type_object_t(bin.rhs.ref_t())
    }

    fn is_type_object_t(t: &Type) -> bool {
        let t = t.derefine();
        t.is_type()
            || matches!(t, Type::Record(_) | Type::Structural(_))
            || matches!(
                &t.qual_name()[..],
                "RecordMetaType" | "Record" | "StructuralType"
            )
    }

    fn load_type_cmp_ops(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("_erg_std_prelude"),
            vec![
                (
                    Identifier::static_public("is_lt"),
                    Some(Identifier::private("#is_lt")),
                ),
                (
                    Identifier::static_public("is_le"),
                    Some(Identifier::private("#is_le")),
                ),
                (
                    Identifier::static_public("is_gt"),
                    Some(Identifier::private("#is_gt")),
                ),
                (
                    Identifier::static_public("is_ge"),
                    Some(Identifier::private("#is_ge")),
                ),
                (
                    Identifier::static_public("StructuralType"),
                    Some(Identifier::private("#StructuralType")),
                ),
            ],
        );
        self.subtype_op_loaded = true;
    }

    /// The runtime helpers the error propagation operator (`x?`) needs:
    /// `is_err` tells the error alternative of a `T or E` value from the success
    /// one, `push_err_frame` records a propagation site on `Error.stack`, and
    /// `panic_err` reports the trace where `?` cannot return.
    ///
    /// Normally `load_prelude_py` has already imported these at module level;
    /// this is the fallback for when the prelude was not emitted, and it writes
    /// the import wherever the first `?` happens to be.
    fn load_err_ops(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("_erg_std_prelude"),
            Self::err_op_items(),
        );
        self.err_ops_loaded = true;
    }

    fn err_op_items() -> Vec<(Identifier, Option<Identifier>)> {
        vec![
            (
                Identifier::static_public("is_err"),
                Some(Identifier::private("#is_err")),
            ),
            (
                Identifier::static_public("push_err_frame"),
                Some(Identifier::private("#push_err_frame")),
            ),
            (
                Identifier::static_public("panic_err"),
                Some(Identifier::private("#panic_err")),
            ),
        ]
    }

    fn load_mutate_op(&mut self) {
        let mod_name = Identifier::static_public("_erg_std_prelude");
        self.emit_global_import_items(
            mod_name,
            vec![(
                Identifier::static_public("mutate_operator"),
                Some(Identifier::private("#mutate_operator")),
            )],
        );
        self.mutate_op_loaded = true;
    }

    /// Whether an import emitted right now has run by the time any later use of
    /// it is reached. Only a module-level one has: an import inside a function
    /// body runs when that function is *called*, so remembering it would leave a
    /// later module-level use with a bare `LOAD_NAME` of a name nothing defined
    /// (`f(x: Ratio) = x * 0.5` followed by `f(0.5)` used to raise NameError).
    fn loaded_for_whole_module(&self) -> bool {
        self.units.len() <= 1
    }

    /// Import the `true_div` helper, bound to `#true_div`. Used for divisions
    /// whose operand types are only known at runtime (a generic `f x = x / 2`):
    /// it returns an exact `Fraction` for integers and a `float`/`complex`
    /// otherwise. Concrete `Ratio` divisions use an inline `Fraction` instead.
    fn load_true_div(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("_erg_std_prelude"),
            vec![(
                Identifier::static_public("true_div"),
                Some(Identifier::private("#true_div")),
            )],
        );
        self.true_div_loaded = self.loaded_for_whole_module();
    }

    fn load_control(&mut self) {
        let mod_name = Identifier::static_public("_erg_control");
        self.emit_import_all_instr(mod_name);
        self.control_loaded = true;
    }

    fn load_convertors(&mut self) {
        let mod_name = Identifier::static_public("_erg_convertors");
        self.emit_import_all_instr(mod_name);
        self.convertors_loaded = true;
    }

    fn load_traits(&mut self) {
        let mod_name = Identifier::static_public("_erg_traits");
        self.emit_import_all_instr(mod_name);
        self.traits_loaded = true;
    }

    fn load_operators(&mut self) {
        let mod_name = Identifier::static_public("operator");
        self.emit_import_all_instr(mod_name);
        self.operators_loaded = true;
    }

    fn load_prelude_py(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("sys"),
            vec![(
                Identifier::static_public("path"),
                Some(Identifier::private("#path")),
            )],
        );
        self.emit_load_name_instr(Identifier::private("#path"));
        self.emit_load_method_instr(Identifier::static_public("append"), BoundAttr);
        self.emit_load_const(erg_core_path().to_str().unwrap());
        self.emit_call_instr(1, BoundAttr);
        self.stack_dec();
        self.emit_pop_top();
        let erg_std_mod = Identifier::static_public("_erg_std_prelude");
        // escaping
        let mut items = vec![(
            Identifier::static_public("contains_operator"),
            Some(Identifier::private("#contains_operator")),
        )];
        items.extend(Self::err_op_items());
        self.emit_global_import_items(erg_std_mod.clone(), items);
        self.emit_import_all_instr(erg_std_mod);
    }

    fn load_record_type(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("collections"),
            vec![(
                Identifier::static_public("namedtuple"),
                Some(Identifier::private("#NamedTuple")),
            )],
        );
    }

    fn load_abc(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("abc"),
            vec![
                (
                    Identifier::static_public("ABCMeta"),
                    Some(Identifier::private("#ABCMeta")),
                ),
                (
                    Identifier::static_public("abstractmethod"),
                    Some(Identifier::private("#abstractmethod")),
                ),
            ],
        );
    }

    fn load_union(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("_erg_type"),
            vec![(
                Identifier::static_public("UnionType"),
                Some(Identifier::private("#UnionType")),
            )],
        );
    }

    fn load_fake_generic(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("_erg_type"),
            vec![(
                Identifier::static_public("FakeGenericAlias"),
                Some(Identifier::private("#FakeGenericAlias")),
            )],
        );
    }

    fn load_generic_alias(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("_erg_type"),
            vec![(
                Identifier::static_public("GenericAlias"),
                Some(Identifier::private("#GenericAlias")),
            )],
        );
    }

    fn load_module_type(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("types"),
            vec![(
                Identifier::static_public("ModuleType"),
                Some(Identifier::private("#ModuleType")),
            )],
        );
    }

    fn load_builtins(&mut self) {
        self.emit_global_import_items(
            Identifier::static_public("_erg_builtins"),
            vec![(
                Identifier::static_public("sum"),
                Some(Identifier::private("#sum")),
            )],
        );
    }

    pub fn emit(&mut self, hir: HIR) -> CodeObj {
        log!(info "the code-generating process has started.{RESET}");
        self.captures = collect_captures(&hir.module);
        self.unit_size += 1;
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            vec![],
            0,
            Str::rc(self.cfg.input.enclosed_name()),
            "<module>",
            1,
            0,
        ));
        self.mut_cur_block().kind = UnitKind::Module;
        if self.opcode_set.is_3_11_plus() {
            self.write_instr(self.opcode_set.resume());
            self.write_arg(0);
        }
        if !self.cfg.no_std && !self.prelude_loaded {
            self.load_prelude();
        }
        for chunk in hir.module.into_iter() {
            self.emit_chunk(chunk);
            // TODO: discard
            if self.stack_len() == 1 {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top(); // 最後の値は戻り値として取っておく
        if self.input().is_repl() {
            if self.stack_len() == 1 {
                self.emit_print_expr();
            }
            self.stack_dec_n(self.stack_len() as usize);
        }
        if self.stack_len() == 0 {
            self.emit_load_const(ValueObj::None);
        } else if self.stack_len() > 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit: invalid stack size");
        }
        self.write_opcode(RETURN_VALUE);
        self.write_arg(0);
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        self.remap_localsplus();
        // end of flagging
        let unit = self.units.pop().unwrap();
        if !self.units.is_empty() {
            let ld = unit.prev_lineno - self.cur_block().prev_lineno;
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += u8::try_from(ld).unwrap();
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        log!(info "the code-generating process has completed.{RESET}");
        unit.codeobj
    }
}
