//! Generates Erg type declarations from Python type stubs (`.pyi`).
//!
//! The entry point is [`convert_pyi_to_decl`], which parses Python stub
//! source with a real Python parser and emits Erg declaration source
//! (the same syntax used by `.d.er` files, without leading-dot visibility
//! markers since all symbols in `.pyi` modules are treated as public).
//!
//! Every generated declaration is validated with the Erg parser; items
//! that fail to parse are dropped instead of poisoning the whole module.
//!
//! Conversion policy:
//! - Functions are declared pure (`->`). Purity cannot be inferred from
//!   stubs; declarations are trusted unconditionally anyway.
//! - Unknown or unsupported annotations map to `Obj`.
//! - `@overload`: the first overload wins.
//! - `if` blocks (e.g. `sys.version_info` guards): the first definition
//!   of a name wins, so the `if` branch takes precedence over `else`.
//! - Private names (leading `_`) are skipped; dunder methods are kept.

use std::collections::{HashMap, HashSet};
use std::fmt;

use erg_parser::parse::SimpleParser;
use rustpython_parser::ast;
use rustpython_parser::Parse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertError(pub String);

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to convert .pyi: {}", self.0)
    }
}

impl std::error::Error for ConvertError {}

/// Convert Python type stub (`.pyi`) source into Erg declaration source.
pub fn convert_pyi_to_decl(src: &str) -> Result<String, ConvertError> {
    let suite = ast::Suite::parse(src, "<pyi>").map_err(|e| ConvertError(e.to_string()))?;
    let mut conv = Converter::default();
    conv.collect(&suite);
    let mut chunks = vec![];
    let mut seen = HashSet::new();
    conv.emit_stmts(&suite, &mut chunks, &mut seen);
    Ok(assemble(chunks))
}

/// One generated declaration (a function/variable line or a whole class block).
struct Chunk {
    text: String,
    /// emitted instead when `text` fails to parse (e.g. a class header
    /// without its members)
    fallback: Option<String>,
}

fn parses_ok(src: &str) -> bool {
    SimpleParser::parse(src.to_string()).is_ok()
}

/// Join chunks, dropping any that the Erg parser rejects.
fn assemble(chunks: Vec<Chunk>) -> String {
    if chunks.is_empty() {
        return String::new();
    }
    let full = chunks.iter().map(|c| c.text.as_str()).collect::<String>();
    if parses_ok(&full) {
        return full;
    }
    let mut out = String::new();
    for chunk in &chunks {
        if parses_ok(&chunk.text) {
            out.push_str(&chunk.text);
        } else if let Some(fb) = &chunk.fallback {
            if parses_ok(fb) {
                out.push_str(fb);
            }
        }
    }
    out
}

enum MethodKind {
    /// module-level function or `@staticmethod`
    Plain,
    /// instance method: first param is dropped and re-emitted as `self: Class`
    Instance,
    /// `@classmethod`: first param is dropped
    Class,
    /// `@property` / `@cached_property`: emitted as an attribute
    Property,
}

struct FuncView<'a> {
    name: &'a str,
    args: &'a ast::Arguments,
    returns: Option<&'a ast::Expr>,
    decorators: &'a [ast::Expr],
    is_async: bool,
}

impl<'a> FuncView<'a> {
    fn from_stmt(stmt: &'a ast::Stmt) -> Option<Self> {
        match stmt {
            ast::Stmt::FunctionDef(d) => Some(FuncView {
                name: d.name.as_str(),
                args: &d.args,
                returns: d.returns.as_deref(),
                decorators: &d.decorator_list,
                is_async: false,
            }),
            ast::Stmt::AsyncFunctionDef(d) => Some(FuncView {
                name: d.name.as_str(),
                args: &d.args,
                returns: d.returns.as_deref(),
                decorators: &d.decorator_list,
                is_async: true,
            }),
            _ => None,
        }
    }
}

/// The trailing name of a decorator expression
/// (`@property` -> "property", `@functools.cached_property` -> "cached_property",
/// `@x.setter` -> "setter", `@overload(...)` -> "overload").
fn deco_name(expr: &ast::Expr) -> &str {
    match expr {
        ast::Expr::Name(n) => n.id.as_str(),
        ast::Expr::Attribute(a) => a.attr.as_str(),
        ast::Expr::Call(c) => deco_name(&c.func),
        _ => "",
    }
}

#[derive(Default)]
struct DecoInfo {
    skip: bool,
    property: bool,
    staticm: bool,
    classm: bool,
}

fn analyze_decos(decos: &[ast::Expr]) -> DecoInfo {
    let mut info = DecoInfo::default();
    for d in decos {
        match deco_name(d) {
            // property setters/deleters duplicate the getter declaration
            "setter" | "deleter" => info.skip = true,
            "property" | "cached_property" => info.property = true,
            "staticmethod" => info.staticm = true,
            "classmethod" => info.classm = true,
            _ => {}
        }
    }
    info
}

fn is_dunder(name: &str) -> bool {
    name.len() > 4 && name.starts_with("__") && name.ends_with("__")
}

/// Dunder methods that have special meaning for the converter or no
/// useful Erg counterpart.
fn is_skipped_dunder(name: &str) -> bool {
    matches!(
        name,
        "__init__"
            | "__new__"
            | "__call__"
            | "__init_subclass__"
            | "__class_getitem__"
            | "__getattr__"
            | "__getattribute__"
            | "__setattr__"
            | "__delattr__"
    )
}

fn is_ellipsis(expr: &ast::Expr) -> bool {
    matches!(
        expr,
        ast::Expr::Constant(c) if matches!(c.value, ast::Constant::Ellipsis)
    )
}

/// The type of a literal expression, if inferable.
fn literal_type(expr: &ast::Expr) -> Option<&'static str> {
    let ast::Expr::Constant(c) = expr else {
        return None;
    };
    match &c.value {
        ast::Constant::Bool(_) => Some("Bool"),
        ast::Constant::Int(_) => Some("Int"),
        ast::Constant::Float(_) => Some("Float"),
        ast::Constant::Str(_) => Some("Str"),
        ast::Constant::Bytes(_) => Some("Bytes"),
        _ => None,
    }
}

fn is_simple_str_literal(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\')
}

#[derive(Default)]
struct Converter {
    /// locally defined (public) class names
    classes: HashSet<String>,
    /// TypeVar/ParamSpec name -> optional `bound=` expression
    typevars: HashMap<String, Option<ast::Expr>>,
    /// import alias -> canonical name, for names imported from
    /// typing / typing_extensions / collections.abc / builtins
    known_imports: HashMap<String, String>,
    /// module aliases referring to typing (e.g. `import typing as t`)
    typing_mods: HashSet<String>,
    /// local type aliases: name -> aliased type expression
    aliases: HashMap<String, ast::Expr>,
}

impl Converter {
    /// Pass 1: collect classes, type variables, imports and aliases
    /// (recursing into `if` blocks) so that forward references resolve.
    fn collect(&mut self, stmts: &[ast::Stmt]) {
        for stmt in stmts {
            match stmt {
                ast::Stmt::ClassDef(cls) => {
                    if !cls.name.starts_with('_') {
                        self.classes.insert(cls.name.to_string());
                    }
                }
                ast::Stmt::ImportFrom(imp) => {
                    let module = imp.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                    if matches!(
                        module,
                        "typing" | "typing_extensions" | "collections.abc" | "builtins"
                    ) {
                        for alias in &imp.names {
                            let name = alias.name.as_str();
                            let local = alias.asname.as_ref().map(|n| n.as_str()).unwrap_or(name);
                            self.known_imports
                                .insert(local.to_string(), name.to_string());
                        }
                    }
                }
                ast::Stmt::Import(imp) => {
                    for alias in &imp.names {
                        let name = alias.name.as_str();
                        if matches!(name, "typing" | "typing_extensions") {
                            let local = alias.asname.as_ref().map(|n| n.as_str()).unwrap_or(name);
                            self.typing_mods.insert(local.to_string());
                        }
                    }
                }
                ast::Stmt::Assign(assign) => {
                    let [ast::Expr::Name(target)] = &assign.targets[..] else {
                        continue;
                    };
                    let name = target.id.to_string();
                    if let ast::Expr::Call(call) = assign.value.as_ref() {
                        if matches!(
                            deco_name(&call.func),
                            "TypeVar" | "ParamSpec" | "TypeVarTuple"
                        ) {
                            let bound = call
                                .keywords
                                .iter()
                                .find(|kw| kw.arg.as_ref().is_some_and(|a| a.as_str() == "bound"))
                                .map(|kw| kw.value.clone());
                            self.typevars.insert(name, bound);
                        }
                    } else if matches!(
                        assign.value.as_ref(),
                        ast::Expr::Name(_)
                            | ast::Expr::Subscript(_)
                            | ast::Expr::Attribute(_)
                            | ast::Expr::BinOp(_)
                    ) {
                        // implicit type alias (`Num = Union[int, float]`, `Path = PosixPath`)
                        self.aliases.insert(name, (*assign.value).clone());
                    }
                }
                ast::Stmt::AnnAssign(ann) => {
                    // explicit type alias: `X: TypeAlias = ...`
                    if let (ast::Expr::Name(target), Some(value)) =
                        (ann.target.as_ref(), ann.value.as_deref())
                    {
                        if self.head_canonical(&ann.annotation).as_deref() == Some("TypeAlias") {
                            self.aliases.insert(target.id.to_string(), value.clone());
                        }
                    }
                }
                ast::Stmt::If(if_) => {
                    self.collect(&if_.body);
                    self.collect(&if_.orelse);
                }
                _ => {}
            }
        }
    }

    /// The canonical name of an annotation head: resolves import aliases
    /// and `typing.X` / `collections.abc.X` attribute accesses.
    fn head_canonical(&self, expr: &ast::Expr) -> Option<String> {
        match expr {
            ast::Expr::Name(n) => {
                let id = n.id.as_str();
                Some(
                    self.known_imports
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| id.to_string()),
                )
            }
            ast::Expr::Attribute(a) => {
                if let ast::Expr::Name(base) = a.value.as_ref() {
                    let base_id = base.id.as_str();
                    if self.typing_mods.contains(base_id)
                        || matches!(base_id, "typing" | "abc" | "builtins")
                    {
                        return Some(a.attr.to_string());
                    }
                }
                // collections.abc.X
                if let ast::Expr::Attribute(inner) = a.value.as_ref() {
                    if let ast::Expr::Name(base) = inner.value.as_ref() {
                        if base.id.as_str() == "collections" && inner.attr.as_str() == "abc" {
                            return Some(a.attr.to_string());
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Map a Python annotation expression to an Erg type expression.
    /// Type variables encountered are recorded in `used` (for quantifiers).
    fn map_type(
        &self,
        expr: &ast::Expr,
        used: &mut Vec<String>,
        ctx: Option<&str>,
        depth: usize,
    ) -> String {
        if depth > 16 {
            return "Obj".into();
        }
        match expr {
            ast::Expr::Name(n) => self.map_name(n.id.as_str(), used, ctx, depth),
            ast::Expr::Attribute(_) => match self.head_canonical(expr) {
                Some(c) => self.map_bare(&c, ctx),
                None => "Obj".into(),
            },
            ast::Expr::Constant(c) => match &c.value {
                ast::Constant::None => "NoneType".into(),
                // forward reference, e.g. `-> "Foo"`
                ast::Constant::Str(s)
                    if !s.is_empty()
                        && s.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') =>
                {
                    self.map_name(s, used, ctx, depth + 1)
                }
                _ => "Obj".into(),
            },
            ast::Expr::BinOp(b) if matches!(b.op, ast::Operator::BitOr) => {
                let l = self.map_type(&b.left, used, ctx, depth + 1);
                let r = self.map_type(&b.right, used, ctx, depth + 1);
                format!("{l} or {r}")
            }
            ast::Expr::Subscript(sub) => self.map_subscript(sub, used, ctx, depth),
            _ => "Obj".into(),
        }
    }

    fn map_name(
        &self,
        id: &str,
        used: &mut Vec<String>,
        ctx: Option<&str>,
        depth: usize,
    ) -> String {
        if self.typevars.contains_key(id) {
            if !used.iter().any(|u| u == id) {
                used.push(id.to_string());
            }
            return id.to_string();
        }
        if self.classes.contains(id) {
            return id.to_string();
        }
        if let Some(alias) = self.aliases.get(id) {
            return self.map_type(alias, used, ctx, depth + 1);
        }
        let canonical = self.known_imports.get(id).map(|s| s.as_str()).unwrap_or(id);
        self.map_bare(canonical, ctx)
    }

    /// Map a canonical (already alias-resolved) bare name.
    fn map_bare(&self, canonical: &str, ctx: Option<&str>) -> String {
        match canonical {
            "int" => "Int",
            "float" => "Float",
            "bool" => "Bool",
            "str" | "Text" | "AnyStr" | "LiteralString" => "Str",
            "bytes" | "ByteString" => "Bytes",
            "complex" => "Complex",
            "object" | "Any" => "Obj",
            "None" | "NoneType" => "NoneType",
            "NoReturn" | "Never" => "Never",
            "type" | "Type" => "ClassType",
            "Callable" => "GenericCallable",
            "list" | "List" => "[Obj; _]",
            "dict" | "Dict" => "{Obj: Obj}",
            "set" | "Set" | "MutableSet" | "AbstractSet" => "Set(Obj, _)",
            "frozenset" | "FrozenSet" => "FrozenSet(Obj)",
            "Iterable" | "Collection" | "Container" | "Reversible" => "Iterable(Obj)",
            "Iterator" => "Iterator(Obj)",
            "Sequence" | "MutableSequence" => "Sequence(Obj)",
            "Mapping" | "MutableMapping" => "Mapping(Obj, Obj)",
            "Generator" => "Generator(Obj)",
            "Self" => return ctx.unwrap_or("Obj").to_string(),
            _ => "Obj",
        }
        .to_string()
    }

    fn map_subscript(
        &self,
        sub: &ast::ExprSubscript,
        used: &mut Vec<String>,
        ctx: Option<&str>,
        depth: usize,
    ) -> String {
        let Some(head) = self.head_canonical(&sub.value) else {
            return "Obj".into();
        };
        // a subscripted local class or alias: drop the type arguments
        // (generated class declarations are not generic)
        if self.classes.contains(&head) {
            return head;
        }
        if self.aliases.contains_key(&head) {
            return self.map_name(&head, used, ctx, depth + 1);
        }
        let args: Vec<&ast::Expr> = match sub.slice.as_ref() {
            ast::Expr::Tuple(t) => t.elts.iter().collect(),
            other => vec![other],
        };
        match head.as_str() {
            "Optional" => format!(
                "{} or NoneType",
                self.map_type(args[0], used, ctx, depth + 1)
            ),
            "Union" => args
                .iter()
                .map(|a| self.map_type(a, used, ctx, depth + 1))
                .collect::<Vec<_>>()
                .join(" or "),
            "list" | "List" => format!("[{}; _]", self.map_type(args[0], used, ctx, depth + 1)),
            "dict" | "Dict" | "OrderedDict" | "DefaultDict" | "defaultdict" if args.len() >= 2 => {
                let k = self.map_type(args[0], used, ctx, depth + 1);
                let v = self.map_type(args[1], used, ctx, depth + 1);
                format!("{{{k}: {v}}}")
            }
            "Mapping" | "MutableMapping" if args.len() >= 2 => {
                let k = self.map_type(args[0], used, ctx, depth + 1);
                let v = self.map_type(args[1], used, ctx, depth + 1);
                format!("Mapping({k}, {v})")
            }
            "set" | "Set" | "MutableSet" | "AbstractSet" => {
                format!("Set({}, _)", self.map_type(args[0], used, ctx, depth + 1))
            }
            "frozenset" | "FrozenSet" => format!(
                "FrozenSet({})",
                self.map_type(args[0], used, ctx, depth + 1)
            ),
            "tuple" | "Tuple" => {
                if args.len() == 2 && is_ellipsis(args[1]) {
                    format!(
                        "HomogenousTuple({})",
                        self.map_type(args[0], used, ctx, depth + 1)
                    )
                } else if args.len() == 1 {
                    format!("({},)", self.map_type(args[0], used, ctx, depth + 1))
                } else {
                    let elems = args
                        .iter()
                        .map(|a| self.map_type(a, used, ctx, depth + 1))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({elems})")
                }
            }
            "Iterable" | "Collection" | "Container" | "Reversible" => {
                format!("Iterable({})", self.map_type(args[0], used, ctx, depth + 1))
            }
            "Iterator" => format!("Iterator({})", self.map_type(args[0], used, ctx, depth + 1)),
            "Sequence" | "MutableSequence" => {
                format!("Sequence({})", self.map_type(args[0], used, ctx, depth + 1))
            }
            "Generator" => format!(
                "Generator({})",
                self.map_type(args[0], used, ctx, depth + 1)
            ),
            "Callable" if args.len() == 2 => {
                let ret = self.map_type(args[1], used, ctx, depth + 1);
                match args[0] {
                    ast::Expr::List(params) => {
                        let ps = params
                            .elts
                            .iter()
                            .map(|p| self.map_type(p, used, ctx, depth + 1))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("(({ps}) -> {ret})")
                    }
                    // Callable[..., R] / Callable[P, R] (ParamSpec)
                    _ => format!("((*args: Obj) -> {ret})"),
                }
            }
            "Literal" => self.map_literal(&args),
            "ClassVar" | "Final" | "Annotated" | "Required" | "NotRequired" | "ReadOnly" => {
                self.map_type(args[0], used, ctx, depth + 1)
            }
            "type" | "Type" => "ClassType".into(),
            _ => "Obj".into(),
        }
    }

    fn map_literal(&self, args: &[&ast::Expr]) -> String {
        let mut parts = vec![];
        let mut has_none = false;
        for arg in args {
            let ast::Expr::Constant(c) = arg else {
                return "Obj".into();
            };
            match &c.value {
                ast::Constant::Int(i) => parts.push(i.to_string()),
                ast::Constant::Bool(b) => parts.push(if *b { "True" } else { "False" }.into()),
                ast::Constant::Str(s) if is_simple_str_literal(s) => {
                    parts.push(format!("\"{s}\""));
                }
                ast::Constant::None => has_none = true,
                _ => return "Obj".into(),
            }
        }
        match (parts.is_empty(), has_none) {
            (true, _) => "NoneType".into(),
            (false, false) => format!("{{{}}}", parts.join(", ")),
            (false, true) => format!("{{{}}} or NoneType", parts.join(", ")),
        }
    }

    /// Render a parameter list. `skip_first` drops Python's `self`/`cls`;
    /// `self_param` re-emits an explicit `self: Class` parameter.
    fn render_params(
        &self,
        args: &ast::Arguments,
        skip_first: bool,
        self_param: Option<&str>,
        used: &mut Vec<String>,
        ctx: Option<&str>,
    ) -> String {
        let mut parts: Vec<String> = vec![];
        if let Some(class_name) = self_param {
            parts.push(format!("self: {class_name}"));
        }
        let mut pos = args.posonlyargs.iter().chain(args.args.iter());
        if skip_first {
            pos.next();
        }
        // once a parameter has a default, all subsequent ones are declared
        // optional too (Erg disallows required params after optional ones)
        let mut optional = false;
        for arg in pos {
            let name = arg.def.arg.as_str();
            let ty = arg
                .def
                .annotation
                .as_deref()
                .map(|t| self.map_type(t, used, ctx, 0))
                .unwrap_or_else(|| "Obj".into());
            if arg.default.is_some() {
                optional = true;
            }
            if optional {
                parts.push(format!("{name} := {ty}"));
            } else {
                parts.push(format!("{name}: {ty}"));
            }
        }
        if let Some(vararg) = &args.vararg {
            let ty = vararg
                .annotation
                .as_deref()
                .map(|t| self.map_type(t, used, ctx, 0))
                .unwrap_or_else(|| "Obj".into());
            parts.push(format!("*{}: {}", vararg.arg.as_str(), ty));
        }
        for arg in &args.kwonlyargs {
            let name = arg.def.arg.as_str();
            let ty = arg
                .def
                .annotation
                .as_deref()
                .map(|t| self.map_type(t, used, ctx, 0))
                .unwrap_or_else(|| "Obj".into());
            // keyword-only params are declared optional: Erg cannot express
            // required keyword-only arguments
            parts.push(format!("{name} := {ty}"));
        }
        if let Some(kwarg) = &args.kwarg {
            let ty = kwarg
                .annotation
                .as_deref()
                .map(|t| self.map_type(t, used, ctx, 0))
                .unwrap_or_else(|| "Obj".into());
            parts.push(format!("**{}: {}", kwarg.arg.as_str(), ty));
        }
        parts.join(", ")
    }

    /// Render the type-variable quantifier (e.g. `|T: Type, U <: Int|`).
    fn render_quant(&self, used: &[String]) -> String {
        if used.is_empty() {
            return String::new();
        }
        let mut scratch = vec![];
        let items = used
            .iter()
            .map(|tv| match self.typevars.get(tv).and_then(|b| b.as_ref()) {
                Some(bound) => {
                    format!("{tv} <: {}", self.map_type(bound, &mut scratch, None, 0))
                }
                None => format!("{tv}: Type"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("|{items}|")
    }

    /// Render a function/method declaration line (without indent/newline).
    fn render_func(&self, f: &FuncView, ctx: Option<&str>, kind: MethodKind) -> String {
        let mut used = vec![];
        if let MethodKind::Property = kind {
            let mut ret = f
                .returns
                .map(|r| self.map_type(r, &mut used, ctx, 0))
                .unwrap_or_else(|| "Obj".into());
            if !used.is_empty() {
                // an attribute declaration cannot be quantified
                ret = "Obj".into();
            }
            return format!("{}: {ret}", f.name);
        }
        let (skip_first, self_param) = match kind {
            MethodKind::Instance => (true, ctx),
            MethodKind::Class => (true, None),
            _ => (false, None),
        };
        let params = self.render_params(f.args, skip_first, self_param, &mut used, ctx);
        let ret = if f.is_async {
            // async functions return coroutine objects
            "Obj".into()
        } else {
            f.returns
                .map(|r| self.map_type(r, &mut used, ctx, 0))
                .unwrap_or_else(|| "NoneType".into())
        };
        let quant = self.render_quant(&used);
        format!("{}: {quant}({params}) -> {ret}", f.name)
    }

    /// Pass 2 (module level): emit declarations in source order.
    /// `seen` implements first-definition-wins deduplication.
    fn emit_stmts(&self, stmts: &[ast::Stmt], chunks: &mut Vec<Chunk>, seen: &mut HashSet<String>) {
        for stmt in stmts {
            if let Some(view) = FuncView::from_stmt(stmt) {
                if view.name.starts_with('_') || seen.contains(view.name) {
                    continue;
                }
                let info = analyze_decos(view.decorators);
                if info.skip {
                    continue;
                }
                let line = self.render_func(&view, None, MethodKind::Plain);
                seen.insert(view.name.to_string());
                chunks.push(Chunk {
                    text: format!("{line}\n"),
                    fallback: None,
                });
                continue;
            }
            match stmt {
                ast::Stmt::ClassDef(cls) => {
                    if cls.name.starts_with('_') || seen.contains(cls.name.as_str()) {
                        continue;
                    }
                    if let Some(chunk) = self.render_class(cls) {
                        seen.insert(cls.name.to_string());
                        chunks.push(chunk);
                    }
                }
                ast::Stmt::AnnAssign(ann) => {
                    let ast::Expr::Name(target) = ann.target.as_ref() else {
                        continue;
                    };
                    let name = target.id.as_str();
                    if name.starts_with('_') || seen.contains(name) {
                        continue;
                    }
                    // type aliases are resolved inline, not re-declared
                    if self.head_canonical(&ann.annotation).as_deref() == Some("TypeAlias") {
                        continue;
                    }
                    let mut used = vec![];
                    let ty = self.map_type(&ann.annotation, &mut used, None, 0);
                    if !used.is_empty() {
                        continue;
                    }
                    seen.insert(name.to_string());
                    chunks.push(Chunk {
                        text: format!("{name}: {ty}\n"),
                        fallback: None,
                    });
                }
                ast::Stmt::Assign(assign) => {
                    let [ast::Expr::Name(target)] = &assign.targets[..] else {
                        continue;
                    };
                    let name = target.id.as_str();
                    if name.starts_with('_') || seen.contains(name) {
                        continue;
                    }
                    // TypeVars and aliases were collected in pass 1
                    if self.typevars.contains_key(name) || self.aliases.contains_key(name) {
                        continue;
                    }
                    let Some(ty) = literal_type(&assign.value) else {
                        continue;
                    };
                    seen.insert(name.to_string());
                    chunks.push(Chunk {
                        text: format!("{name}: {ty}\n"),
                        fallback: None,
                    });
                }
                ast::Stmt::If(if_) => {
                    self.emit_stmts(&if_.body, chunks, seen);
                    self.emit_stmts(&if_.orelse, chunks, seen);
                }
                _ => {}
            }
        }
    }

    fn render_class(&self, cls: &ast::StmtClassDef) -> Option<Chunk> {
        let name = cls.name.as_str();
        let mut members: Vec<String> = vec![];
        let mut seen = HashSet::new();
        let mut init: Option<String> = None;
        let mut new_: Option<String> = None;
        self.walk_class_body(
            &cls.body,
            name,
            &mut members,
            &mut seen,
            &mut init,
            &mut new_,
        );
        let ctor = init
            .or(new_)
            .unwrap_or_else(|| format!("__call__: (*args: Obj) -> {name}"));
        let mut text = format!("{name}: ClassType\n{name}.\n    {ctor}\n");
        for member in &members {
            text.push_str("    ");
            text.push_str(member);
            text.push('\n');
        }
        Some(Chunk {
            text,
            fallback: Some(format!("{name}: ClassType\n")),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_class_body(
        &self,
        body: &[ast::Stmt],
        class_name: &str,
        members: &mut Vec<String>,
        seen: &mut HashSet<String>,
        init: &mut Option<String>,
        new_: &mut Option<String>,
    ) {
        let ctx = Some(class_name);
        for stmt in body {
            if let Some(view) = FuncView::from_stmt(stmt) {
                let name = view.name;
                let info = analyze_decos(view.decorators);
                if info.skip {
                    continue;
                }
                if name == "__init__" || name == "__new__" {
                    let slot = if name == "__init__" {
                        &mut *init
                    } else {
                        &mut *new_
                    };
                    if slot.is_some() {
                        continue;
                    }
                    let mut used = vec![];
                    let params = self.render_params(view.args, true, None, &mut used, ctx);
                    let quant = self.render_quant(&used);
                    *slot = Some(format!("__call__: {quant}({params}) -> {class_name}"));
                    continue;
                }
                if is_skipped_dunder(name)
                    || (name.starts_with('_') && !is_dunder(name))
                    || seen.contains(name)
                {
                    continue;
                }
                let kind = if info.property {
                    MethodKind::Property
                } else if info.staticm {
                    MethodKind::Plain
                } else if info.classm {
                    MethodKind::Class
                } else {
                    MethodKind::Instance
                };
                seen.insert(name.to_string());
                members.push(self.render_func(&view, ctx, kind));
                continue;
            }
            match stmt {
                ast::Stmt::AnnAssign(ann) => {
                    let ast::Expr::Name(target) = ann.target.as_ref() else {
                        continue;
                    };
                    let name = target.id.as_str();
                    if name.starts_with('_') || seen.contains(name) {
                        continue;
                    }
                    let mut used = vec![];
                    let ty = self.map_type(&ann.annotation, &mut used, ctx, 0);
                    if !used.is_empty() {
                        continue;
                    }
                    seen.insert(name.to_string());
                    members.push(format!("{name}: {ty}"));
                }
                ast::Stmt::Assign(assign) => {
                    let [ast::Expr::Name(target)] = &assign.targets[..] else {
                        continue;
                    };
                    let name = target.id.as_str();
                    if name.starts_with('_') || seen.contains(name) {
                        continue;
                    }
                    let Some(ty) = literal_type(&assign.value) else {
                        continue;
                    };
                    seen.insert(name.to_string());
                    members.push(format!("{name}: {ty}"));
                }
                ast::Stmt::If(if_) => {
                    self.walk_class_body(&if_.body, class_name, members, seen, init, new_);
                    self.walk_class_body(&if_.orelse, class_name, members, seen, init, new_);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(src: &str) -> String {
        convert_pyi_to_decl(src).unwrap()
    }

    #[test]
    fn simple_function() {
        assert_eq!(
            conv("def add(a: int, b: int) -> int: ..."),
            "add: (a: Int, b: Int) -> Int\n"
        );
    }

    #[test]
    fn none_return_defaults() {
        assert_eq!(conv("def f(x: int): ..."), "f: (x: Int) -> NoneType\n");
        assert_eq!(
            conv("def f(x: int) -> None: ..."),
            "f: (x: Int) -> NoneType\n"
        );
    }

    #[test]
    fn optional_and_union() {
        let src = "\
from typing import Optional, Union
def f(x: Optional[int], y: Union[int, str]) -> int | None: ...
";
        assert_eq!(
            conv(src),
            "f: (x: Int or NoneType, y: Int or Str) -> Int or NoneType\n"
        );
    }

    #[test]
    fn containers() {
        let src =
            "def f(xs: list[int], d: dict[str, int], t: tuple[int, str], s: set[str]) -> list[str]: ...";
        assert_eq!(
            conv(src),
            "f: (xs: [Int; _], d: {Str: Int}, t: (Int, Str), s: Set(Str, _)) -> [Str; _]\n"
        );
    }

    #[test]
    fn homogenous_tuple() {
        assert_eq!(
            conv("def f(t: tuple[int, ...]) -> int: ..."),
            "f: (t: HomogenousTuple(Int)) -> Int\n"
        );
    }

    #[test]
    fn default_params() {
        assert_eq!(
            conv("def f(a: int, b: str = \"x\", c: float = 1.0) -> None: ..."),
            "f: (a: Int, b := Str, c := Float) -> NoneType\n"
        );
    }

    #[test]
    fn varargs_kwargs() {
        assert_eq!(
            conv("def f(*args: int, **kwargs: str) -> None: ..."),
            "f: (*args: Int, **kwargs: Str) -> NoneType\n"
        );
    }

    #[test]
    fn class_basic() {
        let src = "\
class Foo:
    x: int
    def __init__(self, x: int) -> None: ...
    def get(self) -> int: ...
    @property
    def size(self) -> int: ...
    @staticmethod
    def make() -> \"Foo\": ...
";
        let expected = "\
Foo: ClassType
Foo.
    __call__: (x: Int) -> Foo
    x: Int
    get: (self: Foo) -> Int
    size: Int
    make: () -> Foo
";
        assert_eq!(conv(src), expected);
    }

    #[test]
    fn class_without_init() {
        let src = "\
class Err(Exception):
    code: int
";
        let expected = "\
Err: ClassType
Err.
    __call__: (*args: Obj) -> Err
    code: Int
";
        assert_eq!(conv(src), expected);
    }

    #[test]
    fn typevar_generic() {
        let src = "\
from typing import TypeVar, Sequence
T = TypeVar(\"T\")
def first(xs: Sequence[T]) -> T: ...
";
        assert_eq!(conv(src), "first: |T: Type|(xs: Sequence(T)) -> T\n");
    }

    #[test]
    fn typevar_bound() {
        let src = "\
from typing import TypeVar
T = TypeVar(\"T\", bound=int)
def f(x: T) -> T: ...
";
        assert_eq!(conv(src), "f: |T <: Int|(x: T) -> T\n");
    }

    #[test]
    fn overload_first_wins() {
        let src = "\
from typing import overload
@overload
def f(x: int) -> int: ...
@overload
def f(x: str) -> str: ...
";
        assert_eq!(conv(src), "f: (x: Int) -> Int\n");
    }

    #[test]
    fn version_guard_if_branch_wins() {
        let src = "\
import sys
if sys.version_info >= (3, 12):
    def f(x: int) -> int: ...
else:
    def f(x: str) -> str: ...
";
        assert_eq!(conv(src), "f: (x: Int) -> Int\n");
    }

    #[test]
    fn literal_types() {
        let src = "\
from typing import Literal
def f(mode: Literal[\"r\", \"w\"]) -> Literal[0, 1]: ...
";
        assert_eq!(conv(src), "f: (mode: {\"r\", \"w\"}) -> {0, 1}\n");
    }

    #[test]
    fn callable() {
        let src = "\
from typing import Callable
def f(cb: Callable[[int, str], bool]) -> Callable[..., int]: ...
";
        assert_eq!(
            conv(src),
            "f: (cb: ((Int, Str) -> Bool)) -> ((*args: Obj) -> Int)\n"
        );
    }

    #[test]
    fn type_alias() {
        let src = "\
from typing import Union
Num = Union[int, float]
def f(x: Num) -> Num: ...
";
        assert_eq!(conv(src), "f: (x: Int or Float) -> Int or Float\n");
    }

    #[test]
    fn module_vars() {
        let src = "\
VERSION: str
DEBUG: bool = False
MAX = 100
";
        assert_eq!(conv(src), "VERSION: Str\nDEBUG: Bool\nMAX: Int\n");
    }

    #[test]
    fn private_names_skipped() {
        let src = "\
def _private() -> None: ...
class _Hidden: ...
_secret: int
x: int
";
        assert_eq!(conv(src), "x: Int\n");
    }

    #[test]
    fn imports_skipped() {
        let src = "\
import os
from collections.abc import Iterable
def f(xs: Iterable[int]) -> int: ...
";
        assert_eq!(conv(src), "f: (xs: Iterable(Int)) -> Int\n");
    }

    #[test]
    fn forward_reference() {
        let src = "\
def make() -> \"Widget\": ...
class Widget:
    def __init__(self) -> None: ...
";
        let expected = "\
make: () -> Widget
Widget: ClassType
Widget.
    __call__: () -> Widget
";
        assert_eq!(conv(src), expected);
    }

    #[test]
    fn unknown_types_become_obj() {
        let src = "\
from os import PathLike
def f(p: PathLike) -> SomeUnknown: ...
";
        assert_eq!(conv(src), "f: (p: Obj) -> Obj\n");
    }

    #[test]
    fn output_is_valid_erg() {
        // every emitted chunk must parse; otherwise assemble() drops it
        let src = "\
from typing import TypeVar, Optional
T = TypeVar(\"T\")
class Conn:
    timeout: float
    def __init__(self, host: str, port: int = 80) -> None: ...
    def send(self, data: bytes) -> Optional[int]: ...
def connect(host: str, *, retries: int = 3) -> Conn: ...
";
        let out = conv(src);
        assert!(parses_ok(&out), "generated decl does not parse:\n{out}");
        assert!(out.contains("connect: (host: Str, retries := Int) -> Conn"));
        assert!(out.contains("__call__: (host: Str, port := Int) -> Conn"));
        assert!(out.contains("send: (self: Conn, data: Bytes) -> Int or NoneType"));
    }
}
