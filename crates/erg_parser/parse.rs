//! implements `Parser`.
//!
//! パーサーを実装する
//!
use std::cell::Cell;
use std::mem;

use erg_common::config::ErgConfig;
use erg_common::error::Location;
use erg_common::io::{Input, InputKind};
use erg_common::set::Set as HashSet;
use erg_common::str::Str;
use erg_common::traits::{DequeStream, ExitStatus, Locational, New, Runnable, Stream};
use erg_common::{
    debug_power_assert, enum_unwrap, impl_display_for_enum, impl_locational_for_enum, log, set,
    switch_lang, switch_unreachable,
};

use crate::ast::*;
use crate::desugar::Desugarer;
use crate::error::{
    CompleteArtifact, IncompleteArtifact, ParseError, ParseErrors, ParseResult, ParserRunnerError,
    ParserRunnerErrors,
};
use crate::lex::Lexer;
use crate::token::{Token, TokenCategory, TokenKind, TokenStream};

use TokenCategory as TC;
use TokenKind::*;

thread_local! {
    /// How deep the recursive descent is on this thread; the `debug` trace indents by it
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Logs entering the calling parse method and, when the guard it binds is dropped at
/// the end of that method, leaving it (`debug` feature only). Put it first in the body:
///
/// ```text
/// fn try_reduce_ident(&mut self) -> ParseResult<Identifier> {
///     trace!(self);
///     ...
/// }
/// ```
///
/// Every early `return` and every `?` drops the guard, so exits need nothing.
macro_rules! trace {
    ($self:ident) => {
        let _trace = $self.trace(|| erg_common::fn_name!());
    };
}
pub(crate) use trace;

/// The guard `trace!` binds; see `Parser::trace`
#[must_use]
pub(crate) struct Trace(fn() -> &'static str);

impl Trace {
    fn enter(name: fn() -> &'static str, cur: Option<&Token>) -> Self {
        if cfg!(feature = "debug") {
            let depth = DEPTH.with(|d| {
                d.set(d.get() + 1);
                d.get()
            });
            log!(
                c DEBUG_MAIN,
                "\n{} ({depth}) entered {}, cur: {}",
                "･".repeat(depth / 4),
                name(),
                cur.unwrap_or(&Token::DUMMY)
            );
        }
        Self(name)
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        if cfg!(feature = "debug") {
            let depth = DEPTH.with(|d| {
                d.set(d.get() - 1);
                d.get()
            });
            log!(
                c DEBUG_MAIN,
                "\n{} ({depth}) exit {}",
                "･".repeat(depth / 4),
                (self.0)()
            );
        }
    }
}

pub trait Parsable: 'static {
    fn parse(code: String) -> Result<CompleteArtifact, IncompleteArtifact<Module, ParseErrors>>;
}

#[cfg_attr(feature = "pylib", pyo3::pyclass)]
pub struct SimpleParser {}

impl Parsable for SimpleParser {
    fn parse(code: String) -> Result<CompleteArtifact, IncompleteArtifact> {
        let ts = Lexer::from_str(code).lex().map_err(|(_, es)| es)?;
        let mut parser = Parser::new(ts);
        let mut desugarer = Desugarer::new();
        let artifact = parser
            .parse()
            .map_err(|iart| iart.map_mod(|module| desugarer.desugar(module)))?;
        Ok(artifact.map(|module| desugarer.desugar(module)))
    }
}

impl SimpleParser {
    pub fn parse(code: String) -> Result<CompleteArtifact, IncompleteArtifact> {
        <Self as Parsable>::parse(code)
    }
}

/// Check if the given source code is syntactically complete.
/// Used for REPL to determine whether Enter key should execute or insert newline.
///
/// Returns:
/// - `Complete`: Code is valid and can be executed
/// - `MayContinue`: Code is valid, but ends with a class/patch definition or a
///   method block, so more method blocks may belong to the same cell
/// - `Incomplete`: Code needs more input (unclosed brackets, unfinished blocks, etc.)
/// - `SyntaxError`: Code has a syntax error (should be executed to show error message)
pub fn check_code_completeness(src: &str) -> erg_common::stdin::CodeCompleteness {
    use erg_common::error::ErrorKind;
    use erg_common::stdin::CodeCompleteness;

    if src.trim().is_empty() {
        return CodeCompleteness::Complete;
    }
    if has_open_delimiters(src) {
        return CodeCompleteness::Unclosed;
    }
    // Trailing newlines must not affect the result, but the parser reports a
    // plain SyntaxError (instead of ExpectNextLine) for e.g. "f x =\n",
    // because the EOF check after `=` only fires when EOF follows directly
    let src = src.trim_end();
    match SimpleParser::parse(src.to_string()) {
        Ok(artifact) => {
            if ends_with_class_def(&artifact.ast) {
                CodeCompleteness::MayContinue
            } else {
                CodeCompleteness::Complete
            }
        }
        Err(incomplete_artifact) => {
            // `ExpectNextLine` is emitted by the parser when a block is expected
            // at EOF (after `=`, `=>`, a decorator, a class accessor line, ...),
            // and by the lexer for unclosed multi-line constructs
            let expects_more = incomplete_artifact
                .errors
                .iter()
                .any(|e| e.core().kind == ErrorKind::ExpectNextLine);
            if expects_more {
                // a decorator must be followed by a definition at the SAME
                // indent level, not an indented block
                let last_line = src.lines().rev().find(|l| !l.trim().is_empty());
                if last_line.is_some_and(|l| l.trim_start().starts_with('@')) {
                    CodeCompleteness::Continuation
                } else {
                    CodeCompleteness::ExpectsBlock
                }
            } else {
                // Actual syntax error - should be executed to show error
                CodeCompleteness::SyntaxError
            }
        }
    }
}

/// Whether the module ends with a class or patch definition, or with a method
/// block (`C.` / `C::`) of one.
///
/// The method blocks of a class are attached to it while the AST is still
/// whole (the AST linker), so they have to be in the same chunk as the
/// definition -- in the REPL, the same cell. The REPL therefore keeps such a
/// cell open for more method blocks (`CodeCompleteness::MayContinue`). A
/// definition is recognized the way the linker recognizes it: its body is a
/// call to `Class`, `Inherit`, `Inheritable` or `Patch`.
fn ends_with_class_def(module: &Module) -> bool {
    match module.last() {
        Some(Expr::Def(def)) => matches!(
            def.body.block.first(),
            Some(Expr::Call(call))
                if matches!(
                    call.obj.get_name().map(|s| &s[..]),
                    Some("Class" | "Inherit" | "Inheritable" | "Patch")
                )
        ),
        Some(Expr::Methods(_)) => true,
        _ => false,
    }
}

/// Check if the source ends inside an open delimiter: an unclosed bracket,
/// string, or multi-line comment. This is a quick structural scan that runs
/// before full parsing.
fn has_open_delimiters(src: &str) -> bool {
    let mut paren_count = 0i32; // ()
    let mut bracket_count = 0i32; // []
    let mut brace_count = 0i32; // {}
    let mut in_string = false;
    let mut in_triple_string = false;
    let mut comment_depth = 0usize; // #[ ]# (nestable)

    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Inside a multi-line comment: only track nesting
        if comment_depth > 0 {
            if c == '#' && chars.get(i + 1) == Some(&'[') {
                comment_depth += 1;
                i += 2;
            } else if c == ']' && chars.get(i + 1) == Some(&'#') {
                comment_depth -= 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        // Skip escaped characters inside strings (handles `\"` and `\\`)
        if (in_string || in_triple_string) && c == '\\' {
            i += 2;
            continue;
        }

        // Handle triple quotes
        if !in_string && i + 2 < chars.len() && chars[i..i + 3] == ['"', '"', '"'] {
            in_triple_string = !in_triple_string;
            i += 3;
            continue;
        }

        // Handle single-line strings
        if c == '"' && !in_triple_string {
            in_string = !in_string;
            i += 1;
            continue;
        }

        // Skip characters inside strings
        if in_string || in_triple_string {
            i += 1;
            continue;
        }

        // Comments: `#[` opens a multi-line comment, otherwise skip to EOL
        if c == '#' {
            if chars.get(i + 1) == Some(&'[') {
                comment_depth += 1;
                i += 2;
            } else {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            continue;
        }

        // Count brackets
        match c {
            '(' => paren_count += 1,
            ')' => paren_count -= 1,
            '[' => bracket_count += 1,
            ']' => bracket_count -= 1,
            '{' => brace_count += 1,
            '}' => brace_count -= 1,
            _ => {}
        }

        i += 1;
    }

    // If in unclosed string or multi-line comment, it's incomplete
    if in_string || in_triple_string || comment_depth > 0 {
        return true;
    }

    // If any bracket count is positive, we have unclosed brackets
    paren_count > 0 || bracket_count > 0 || brace_count > 0
}

/// Context flags for the expression parsing engine (`try_reduce_expr_prec`).
///
/// Call sites start from one of the constants and switch on what differs:
/// `ExprCtx { winding: true, ..ExprCtx::EXPR }`.
#[derive(Debug, Clone, Copy)]
struct ExprCtx {
    /// statement level: definitions, method blocks and `expr args` calls are allowed
    chunk: bool,
    /// parse paren-less tuples (`1, 2, 3`)
    winding: bool,
    in_type_args: bool,
    /// `:` is a key-value separator, not a type ascription
    in_brace: bool,
    /// the expression can span multiple lines (inside parentheses)
    line_break: bool,
}

impl ExprCtx {
    /// A plain expression: an operand, an argument, an element
    const EXPR: Self = Self {
        chunk: false,
        winding: false,
        in_type_args: false,
        in_brace: false,
        line_break: false,
    };
    /// A statement-level expression (a "chunk"), which may also be a definition
    const CHUNK: Self = Self {
        chunk: true,
        ..Self::EXPR
    };
}

enum ArgKind {
    Pos(PosArg),
    Var(PosArg),
    Kw(KwArg),
    KwVar(PosArg),
}

impl ArgKind {
    /// Adds the argument to `args`, in the slot its kind selects
    fn push_to(self, args: &mut Args) {
        match self {
            Self::Pos(arg) => args.push_pos(arg),
            Self::Var(arg) => args.set_var_args(arg),
            Self::Kw(arg) => args.push_kw(arg),
            Self::KwVar(arg) => args.set_kw_var(arg),
        }
    }
}

impl From<ArgKind> for Args {
    fn from(first: ArgKind) -> Self {
        let mut args = Args::empty();
        first.push_to(&mut args);
        args
    }
}

/// The generators of a comprehension, `x <- xs; y <- ys`, and its optional guard
type Generators = (Vec<(Identifier, Expr)>, Option<Expr>);

pub enum ListInner {
    Normal(Args),
    WithLength(PosArg, Expr),
    Comprehension {
        layout: Option<Expr>,
        generators: Vec<(Identifier, Expr)>,
        guard: Option<Expr>,
    },
}

impl ListInner {
    pub const fn comp(
        layout: Option<Expr>,
        generators: Vec<(Identifier, Expr)>,
        guard: Option<Expr>,
    ) -> Self {
        Self::Comprehension {
            layout,
            generators,
            guard,
        }
    }
}

pub enum BraceContainer {
    Set(Set),
    Dict(Dict),
    Record(Record),
}

impl_locational_for_enum!(BraceContainer; Set, Dict, Record);
impl_display_for_enum!(BraceContainer; Set, Dict, Record);

impl BraceContainer {
    pub const fn kind(&self) -> &str {
        match self {
            BraceContainer::Set(_) => "Set",
            BraceContainer::Dict(_) => "Dict",
            BraceContainer::Record(_) => "Record",
        }
    }
}

impl From<BraceContainer> for Expr {
    fn from(container: BraceContainer) -> Self {
        match container {
            BraceContainer::Set(set) => Expr::Set(set),
            BraceContainer::Dict(dict) => Expr::Dict(dict),
            BraceContainer::Record(record) => Expr::Record(record),
        }
    }
}

pub enum ArgsStyle {
    SingleCommaWithParen,
    SingleCommaNoParen,
    MultiComma, // with parentheses
    Colon,      // with no parentheses
}

impl ArgsStyle {
    pub const fn needs_parens(&self) -> bool {
        match self {
            Self::SingleCommaWithParen | Self::MultiComma => true,
            Self::SingleCommaNoParen | Self::Colon => false,
        }
    }

    pub const fn is_colon(&self) -> bool {
        matches!(self, Self::Colon)
    }

    pub const fn is_multi_comma(&self) -> bool {
        matches!(self, Self::MultiComma)
    }
}

/// Perform recursive descent parsing.
///
/// Every parse method starts with `trace!(self)`, reports errors through `fail`
/// (directly or via the `expect*` / `skip_and_throw_*` helpers) and gives up on
/// the construct with `Err(())`; the caller decides how much to skip.
///
/// To enhance error descriptions, the parsing process will continue as long as it's not fatal.
#[derive(Debug)]
pub struct Parser {
    counter: DefId,
    tokens: TokenStream,
    /// spans that were written inside their own parentheses. The AST keeps no
    /// trace of grouping parens, and `chain_comparison` must not re-group
    /// `(a < b) == c` -- the writer already grouped it.
    parenthesized: HashSet<Location>,
    warns: ParseErrors,
    pub(crate) errs: ParseErrors,
}

impl Parsable for Parser {
    fn parse(code: String) -> Result<CompleteArtifact, IncompleteArtifact<Module, ParseErrors>> {
        let ts = Lexer::from_str(code).lex().map_err(|(_, es)| es)?;
        Parser::new(ts).parse()
    }
}

impl Parser {
    pub fn new(ts: TokenStream) -> Self {
        Self {
            counter: DefId(0),
            tokens: ts,
            parenthesized: HashSet::new(),
            warns: ParseErrors::empty(),
            errs: ParseErrors::empty(),
        }
    }

    #[inline]
    pub fn peek(&self) -> Option<&Token> {
        self.tokens.first()
    }

    /// Whether the source ends here, with only the indentation the lexer has to
    /// close left in between.
    ///
    /// `cur_is(EOF)` is not enough after an operator that introduces a block:
    /// `f x =` at the top level is followed by `EOF` directly, but the same
    /// thing one block in is followed by `Dedent, EOF`, and the REPL needs both
    /// to say "waiting for the block" rather than "syntax error".
    fn at_eof(&self) -> bool {
        self.tokens
            .iter()
            .find(|tk| !tk.is(Newline) && !tk.is(Dedent))
            .is_none_or(|tk| tk.is(EOF))
    }

    pub fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|tok| tok.kind)
    }

    #[inline]
    fn nth(&self, idx: usize) -> Option<&Token> {
        self.tokens.get(idx)
    }

    #[inline]
    fn skip(&mut self) {
        self.tokens.pop_front();
    }

    fn skip_newlines(&mut self) {
        while self.cur_is(Newline) {
            self.skip();
        }
    }

    #[inline]
    fn lpop(&mut self) -> Token {
        self.tokens.pop_front().unwrap()
    }

    fn cur_category_is(&self, category: TokenCategory) -> bool {
        self.peek()
            .map(|t| t.category_is(category))
            .unwrap_or(false)
    }

    fn cur_is(&self, kind: TokenKind) -> bool {
        self.peek().map(|t| t.is(kind)).unwrap_or(false)
    }

    fn nth_is(&self, idx: usize, kind: TokenKind) -> bool {
        self.nth(idx).map(|t| t.is(kind)).unwrap_or(false)
    }

    /// 解析を諦めて次の解析できる要素に移行する
    /// give up parsing and move to the next element that can be parsed
    fn next_expr(&mut self) {
        while let Some(t) = self.peek() {
            match t.category() {
                TC::Separator => {
                    self.skip();
                    return;
                }
                TC::EOF => {
                    return;
                }
                _ => {
                    self.skip();
                }
            }
        }
    }

    fn next_line(&mut self) {
        while let Some(t) = self.peek() {
            match t.kind {
                Newline => {
                    self.skip();
                    return;
                }
                EOF => return,
                _ => {
                    self.skip();
                }
            }
        }
    }

    fn until_dedent(&mut self) {
        let mut nest_cnt = 1;
        while let Some(t) = self.peek() {
            match t.kind {
                Indent => {
                    self.skip();
                    nest_cnt += 1;
                }
                Dedent => {
                    self.skip();
                    nest_cnt -= 1;
                    if nest_cnt <= 0 {
                        return;
                    }
                }
                EOF => return,
                _ => {
                    self.skip();
                }
            }
        }
    }

    /// The `trace!` guard. The name is a closure so that outside the `debug`
    /// feature it is never computed.
    pub(crate) fn trace(&self, name: fn() -> &'static str) -> Trace {
        Trace::enter(name, self.peek())
    }

    /// Records `err` and gives up on the construct being parsed. Every error a parse
    /// method reports goes through here, so the `debug` log names the line that raised it.
    #[track_caller]
    pub(crate) fn fail<T>(&mut self, err: ParseError) -> ParseResult<T> {
        log!(err "error caused by: {}", std::panic::Location::caller());
        self.errs.push(err);
        Err(())
    }

    /// Attaches `hint` to the error the sub-parse that has just failed recorded
    pub(crate) fn hint(&mut self, hint: &str) {
        if let Some(err) = self.errs.last_mut() {
            err.set_hint(hint);
        }
    }

    /// The line of the caller, the error number (`errno`) the parser reports: the
    /// same as writing `line!()` at the call site
    #[track_caller]
    fn errno() -> usize {
        std::panic::Location::caller().line() as usize
    }

    /// The current token is not the `expected` one
    #[track_caller]
    fn unexpected_token_err(&self, expected: impl std::fmt::Display) -> ParseError {
        let loc = self.peek().map(|t| t.loc()).unwrap_or(Location::Unknown);
        let got = self.peek_kind().unwrap_or(EOF);
        ParseError::unexpected_token(Self::errno(), loc, expected, got)
    }

    /// Pops the current token if it is `kind`, otherwise fails
    #[track_caller]
    fn expect(&mut self, kind: TokenKind) -> ParseResult<Token> {
        if self.cur_is(kind) {
            Ok(self.lpop())
        } else {
            self.fail(self.unexpected_token_err(kind))
        }
    }

    /// `expect`, skipping the rest of the line when it fails
    #[track_caller]
    fn expect_or_skip_line(&mut self, kind: TokenKind) -> ParseResult<Token> {
        if self.cur_is(kind) {
            Ok(self.lpop())
        } else {
            let err = self.unexpected_token_err(kind);
            self.next_line();
            self.fail(err)
        }
    }

    /// Pops the current token if it is of `category`, otherwise fails
    #[track_caller]
    fn expect_category(&mut self, category: TokenCategory) -> ParseResult<Token> {
        if self.cur_category_is(category) {
            Ok(self.lpop())
        } else {
            self.fail(self.unexpected_token_err(category))
        }
    }

    /// `peek` gave `None`: the token stream ran out, which its trailing `EOF` token
    /// should make impossible, so this is reported as a parser bug
    #[track_caller]
    fn unexpected_none<T>(&mut self) -> ParseResult<T> {
        let caller = std::panic::Location::caller();
        let err =
            ParseError::invalid_none_match(0, Location::Unknown, caller.file(), caller.line());
        self.fail(err)
    }

    /// A plain syntax error at the current token; the rest of the expression is skipped
    #[track_caller]
    fn skip_and_throw_syntax_err<T>(&mut self) -> ParseResult<T> {
        let loc = self.peek().map(|t| t.loc()).unwrap_or_default();
        self.next_expr();
        self.fail(ParseError::simple_syntax_error(Self::errno(), loc))
    }

    #[track_caller]
    fn skip_and_throw_invalid_unclosed_err<T>(&mut self, closer: &str, ty: &str) -> ParseResult<T> {
        let loc = self.peek().map(|t| t.loc()).unwrap_or_default();
        self.next_expr();
        self.fail(ParseError::unclosed_error(Self::errno(), loc, closer, ty))
    }

    #[track_caller]
    fn skip_and_throw_invalid_seq_err<T>(
        &mut self,
        expected: &[impl std::fmt::Display],
        found: TokenKind,
    ) -> ParseResult<T> {
        let loc = self.peek().map(|t| t.loc()).unwrap_or_default();
        self.next_expr();
        self.fail(ParseError::invalid_seq_elems_error(
            Self::errno(),
            loc,
            expected,
            found,
        ))
    }

    /// Unlike its siblings this only builds the error: a chunk that does not end where
    /// it should is reported, and parsing goes on with the next line
    #[track_caller]
    fn skip_and_throw_invalid_chunk_err(&mut self, loc: Location) -> ParseError {
        log!(err "error caused by: {}", std::panic::Location::caller());
        self.next_line();
        ParseError::invalid_chunk_error(Self::errno(), loc)
    }

    #[track_caller]
    fn skip_and_throw_stream_op_err<T>(&mut self, loc: Location) -> ParseResult<T> {
        self.next_expr();
        self.fail(ParseError::syntax_error(
            Self::errno(),
            loc,
            switch_lang!(
                "japanese" => "パイプ演算子の後には関数・メソッド・サブルーチンのみ呼び出しができます",
                "simplified_chinese" => "流操作符后只能调用函数、方法或子程序",
                "traditional_chinese" => "流操作符後只能調用函數、方法或子程序",
                "english" => "Only a call of function, method or subroutine is available after stream operator",
            ),
            None,
        ))
    }

    /// An operator or expression that would be silently discarded, like the `1 +`
    /// in `1 + x = 2`
    #[track_caller]
    fn extra_operator_err<T>(&mut self, loc: Location) -> ParseResult<T> {
        self.fail(ParseError::syntax_error(
            Self::errno(),
            loc,
            switch_lang!(
                "japanese" => "余分な式・演算子が残っています",
                "simplified_chinese" => "存在多余的表达式或运算符",
                "traditional_chinese" => "存在多餘的表達式或運算符",
                "english" => "extra expression or operator remains",
            ),
            None,
        ))
    }

    /// The comparisons Python chains: `a < b < c` means `a < b and b < c`, not
    /// `(a < b) < c`. `contains` is left out -- it is Erg's own, and the
    /// desugarer generates it.
    const fn is_chainable_comparison(kind: TokenKind) -> bool {
        matches!(
            kind,
            Less | Gre | LessEq | GreEq | DblEq | NotEq | InOp | NotInOp | IsOp | IsNotOp
        )
    }

    /// The operand a chain would compare next: the right-hand side of the
    /// rightmost comparison. The `and` case is one this function built -- `and`
    /// binds looser than a comparison, so a user's own cannot be the left-hand
    /// side here.
    fn chain_tail(expr: &Expr) -> Option<&Expr> {
        match expr {
            Expr::BinOp(bin) if Self::is_chainable_comparison(bin.op.kind) => {
                Some(bin.args[1].as_ref())
            }
            Expr::BinOp(bin) if bin.op.is(AndOp) => Self::chain_tail(bin.args[1].as_ref()),
            _ => None,
        }
    }

    /// Whether `expr` can be written twice without changing what the program
    /// does. The middle operand of a chain is compared against both of its
    /// neighbours, and an expression here has nowhere to bind a temporary.
    fn is_duplicable(expr: &Expr) -> bool {
        match expr {
            Expr::Literal(_) => true,
            Expr::Accessor(Accessor::Ident(_)) => true,
            Expr::Accessor(Accessor::Attr(attr)) => Self::is_duplicable(&attr.obj),
            Expr::UnaryOp(unary) => Self::is_duplicable(unary.args[0].as_ref()),
            _ => false,
        }
    }

    /// `a < b` then `< c` becomes `a < b and b < c`, as it reads and as Python
    /// evaluates it. Without this the comparison is left-associative and
    /// `1 < 3 < 2` quietly compares `True < 2`, which is true.
    fn chain_comparison(&mut self, op: Token, lhs: Expr, rhs: Expr) -> ParseResult<Expr> {
        // `(a < b) == c` is the writer grouping on purpose. Python, too, only
        // chains comparisons that are written bare.
        if self.parenthesized.contains(&lhs.loc()) {
            return Ok(Expr::BinOp(BinOp::new(op, lhs, rhs)));
        }
        let Some(mid) = Self::chain_tail(&lhs) else {
            return Ok(Expr::BinOp(BinOp::new(op, lhs, rhs)));
        };
        if !Self::is_duplicable(mid) {
            return self.fail(ParseError::syntax_error(
                line!() as usize,
                mid.loc(),
                switch_lang!(
                    "japanese" => "連鎖比較の中央の被演算子は二回評価されるため、変数かリテラルでなければなりません",
                    "simplified_chinese" => "链式比较的中间操作数会被求值两次，因此必须是变量或字面量",
                    "traditional_chinese" => "鏈式比較的中間運算元會被求值兩次，因此必須是變數或字面量",
                    "english" => "the middle operand of a chained comparison is evaluated twice, so it must be a variable or a literal",
                ),
                Some(
                    switch_lang!(
                        "japanese" => "一度変数に束縛してから比較してください",
                        "simplified_chinese" => "请先绑定到变量再比较",
                        "traditional_chinese" => "請先繫結到變數再比較",
                        "english" => "bind it to a variable first, then compare",
                    )
                    .to_string(),
                ),
            ));
        }
        let mid = mid.clone();
        let and = Token::new_fake(AndOp, "and", op.lineno, op.col_begin, op.col_end);
        let right = Expr::BinOp(BinOp::new(op, mid, rhs));
        Ok(Expr::BinOp(BinOp::new(and, lhs, right)))
    }

    #[inline]
    fn restore(&mut self, token: Token) {
        self.tokens.push_front(token);
    }
}

#[derive(Debug, Default)]
pub struct ParserRunner {
    cfg: ErgConfig,
}

impl New for ParserRunner {
    #[inline]
    fn new(cfg: ErgConfig) -> Self {
        Self { cfg }
    }
}

impl Runnable for ParserRunner {
    type Err = ParserRunnerError;
    type Errs = ParserRunnerErrors;
    const NAME: &'static str = "Erg parser";

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

    #[inline]
    fn initialize(&mut self) {}

    #[inline]
    fn clear(&mut self) {}

    fn completeness_checker(&self) -> Option<erg_common::stdin::CompletenessChecker> {
        Some(Box::new(crate::parse::check_code_completeness))
    }

    fn exec(&mut self) -> Result<ExitStatus, Self::Errs> {
        let src = self.cfg_mut().input.read();
        let artifact = self.parse(src).map_err(|iart| iart.errors)?;
        println!("{}", artifact.ast);
        Ok(ExitStatus::OK)
    }

    fn eval(&mut self, src: String) -> Result<String, ParserRunnerErrors> {
        let artifact = self.parse(src).map_err(|iart| iart.errors)?;
        Ok(format!("{}", artifact.ast))
    }
}

impl ParserRunner {
    #[inline]
    pub fn new(cfg: ErgConfig) -> Self {
        New::new(cfg)
    }

    pub fn parse_token_stream(
        &mut self,
        ts: TokenStream,
    ) -> Result<CompleteArtifact, IncompleteArtifact<Module, ParserRunnerErrors>> {
        Parser::new(ts)
            .parse()
            .map_err(|iart| iart.map_errs(|errs| ParserRunnerErrors::convert(self.input(), errs)))
    }

    pub fn parse(
        &mut self,
        src: String,
    ) -> Result<CompleteArtifact, IncompleteArtifact<Module, ParserRunnerErrors>> {
        let ts = Lexer::new(Input::new(InputKind::Str(src), self.cfg.input.id()))
            .lex()
            .map_err(|(_, errs)| ParserRunnerErrors::convert(self.input(), errs))?;
        Parser::new(ts)
            .parse()
            .map_err(|iart| iart.map_errs(|errs| ParserRunnerErrors::convert(self.input(), errs)))
    }
}

impl Parser {
    pub fn parse(&mut self) -> Result<CompleteArtifact, IncompleteArtifact> {
        if self.tokens.is_empty() {
            return Ok(CompleteArtifact::new(Module::empty(), ParseErrors::empty()));
        }
        log!(info "the parsing process has started.");
        log!(info "token stream: {}", self.tokens);
        let module = match self.try_reduce_module() {
            Ok(module) => module,
            Err(_) => {
                return Err(IncompleteArtifact::new(
                    None,
                    mem::take(&mut self.warns),
                    mem::take(&mut self.errs),
                ));
            }
        };
        log!(info "the parsing process has completed (errs: {}).", self.errs.len());
        log!(info "AST:\n{module}");
        if self.errs.is_empty() {
            Ok(CompleteArtifact::new(module, mem::take(&mut self.warns)))
        } else {
            Err(IncompleteArtifact::new(
                Some(module),
                mem::take(&mut self.warns),
                mem::take(&mut self.errs),
            ))
        }
    }

    /// Reduce to the largest unit of syntax, the module (this is called only once)
    #[inline]
    fn try_reduce_module(&mut self) -> ParseResult<Module> {
        trace!(self);
        let mut chunks = Module::empty();
        loop {
            match self.peek_kind() {
                Some(Newline | Semi) => {
                    self.skip();
                }
                Some(EOF) => {
                    break;
                }
                Some(_) => {
                    if let Ok(expr) = self.try_reduce_expr(ExprCtx {
                        winding: true,
                        ..ExprCtx::CHUNK
                    }) {
                        if !self.cur_is(EOF) && !self.cur_category_is(TC::Separator) {
                            let err = self.skip_and_throw_invalid_chunk_err(expr.loc());
                            self.errs.push(err);
                        }
                        chunks.push(expr);
                    }
                }
                None => {
                    if self.errs.is_empty() {
                        switch_unreachable!()
                    }
                    let Some(last) = chunks.last() else {
                        return self.unexpected_none();
                    };
                    let err = self.skip_and_throw_invalid_chunk_err(last.loc());
                    self.errs.push(err);
                    break;
                }
            }
        }
        Ok(chunks)
    }

    // expect the block`= ; . -> =>`
    fn try_reduce_block(&mut self) -> ParseResult<Block> {
        trace!(self);
        let mut block = Block::with_capacity(2);
        // single line block
        if !self.cur_is(Newline) {
            let expr = self.try_reduce_expr(ExprCtx {
                winding: true,
                ..ExprCtx::EXPR
            })?;
            block.push(expr);
            if !self.cur_is(Dedent)
                && !self.cur_category_is(TC::Separator)
                && !self.cur_category_is(TC::REnclosure)
            {
                let err = self.skip_and_throw_invalid_chunk_err(block.last().unwrap().loc());
                self.errs.push(err);
            }
            if block.last().unwrap().is_definition() {
                return self.fail(ParseError::invalid_definition_of_last_block(
                    line!() as usize,
                    block.last().unwrap().loc(),
                ));
            } else {
                return Ok(block);
            }
        }
        self.expect(Newline)?;
        self.skip_newlines();
        self.expect(Indent)?;
        loop {
            match self.peek_kind() {
                Some(Newline) if self.nth_is(1, Dedent) => {
                    let nl = self.lpop();
                    self.skip();
                    self.restore(nl);
                    break;
                }
                // last line dedent without newline
                Some(Dedent) => {
                    self.skip();
                    break;
                }
                Some(Newline | Semi) => {
                    self.skip();
                }
                Some(EOF) => {
                    break;
                }
                Some(_) => {
                    if let Ok(expr) = self.try_reduce_expr(ExprCtx {
                        winding: true,
                        ..ExprCtx::CHUNK
                    }) {
                        if !self.cur_is(Dedent) && !self.cur_category_is(TC::Separator) {
                            let err = self.skip_and_throw_invalid_chunk_err(expr.loc());
                            self.errs.push(err);
                        }
                        block.push(expr);
                    }
                }
                None => {
                    let err =
                        ParseError::failed_to_analyze_block(line!() as usize, Location::Unknown);
                    self.errs.push(err);
                    break;
                }
            }
        }
        if block.is_empty() {
            let loc = if let Some(u) = self.peek() {
                u.loc()
            } else {
                Location::Unknown
            };
            self.fail(ParseError::failed_to_analyze_block(line!() as usize, loc))
        } else if block.last().unwrap().is_definition() {
            self.fail(ParseError::invalid_definition_of_last_block(
                line!() as usize,
                block.last().unwrap().loc(),
            ))
        } else {
            Ok(block)
        }
    }

    #[inline]
    fn opt_reduce_decorator(&mut self) -> ParseResult<Option<Decorator>> {
        trace!(self);
        if self.cur_is(TokenKind::AtSign) {
            self.lpop();
            let expr = self.try_reduce_expr(ExprCtx::EXPR).map_err(|_| {
                self.hint(switch_lang!(
                    "japanese" => "予期: デコレータ",
                    "simplified_chinese" => "期望: 装饰器",
                    "traditional_chinese" => "期望: 裝飾器",
                    "english" => "expect: decorator",
                ))
            })?;
            Ok(Some(Decorator::new(expr)))
        } else {
            Ok(None)
        }
    }

    #[inline]
    fn opt_reduce_decorators(&mut self) -> ParseResult<HashSet<Decorator>> {
        trace!(self);
        let mut decs = set![];
        while let Some(deco) = self.opt_reduce_decorator()? {
            // `at_eof`, not `cur_is(EOF)`: a decorator that ends the input one
            // block in (`C.\n    @Override`) is followed by `Dedent, EOF`
            if self.at_eof() {
                let err =
                    ParseError::expect_next_line_error(line!() as usize, deco.0.loc(), "AtMark");
                return self.fail(err);
            }
            decs.insert(deco);
            self.expect_or_skip_line(Newline)?;
        }
        Ok(decs)
    }

    fn try_reduce_type_app_args(&mut self) -> ParseResult<TypeAppArgs> {
        trace!(self);
        let l_vbar = self.expect(VBar)?;
        let args = match self.peek_kind() {
            Some(SubtypeOf) => {
                let op = self.lpop();
                let t_spec_as_expr = self
                    .try_reduce_expr(ExprCtx {
                        in_type_args: true,
                        ..ExprCtx::EXPR
                    })
                    .map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "予期: 型指定",
                            "simplified_chinese" => "期望: 类型规范",
                            "traditional_chinese" => "期望: 類型規範",
                            "english" => "expect: type specification",
                        ))
                    })?;
                match Parser::expr_to_type_spec(t_spec_as_expr.clone()) {
                    Ok(t_spec) => {
                        let t_spec = TypeSpecWithOp::new(op, t_spec, t_spec_as_expr);
                        TypeAppArgsKind::SubtypeOf(Box::new(t_spec))
                    }
                    Err(_) => {
                        return self.fail(ParseError::simple_syntax_error(0, t_spec_as_expr.loc()));
                    }
                }
            }
            _ => {
                let args = self.try_reduce_args(true)?;
                TypeAppArgsKind::Args(args)
            }
        };
        let r_vbar = self.expect(VBar)?;
        Ok(TypeAppArgs::new(l_vbar.loc(), args, r_vbar.loc()))
    }

    /// Parses the guard part of a refinement pattern and desugars the whole
    /// pattern into a set comprehension (i.e. a refinement type).
    /// `X: T | Pred` == `X: {X: T | Pred}`, `X | Pred` == `{X: _ | Pred}`
    /// The caller must ensure that the current token is `|`.
    fn try_reduce_refinement_guard(&mut self, var: Identifier, typ: Expr) -> ParseResult<Expr> {
        trace!(self);
        debug_power_assert!(self.cur_is(VBar));
        let _vbar = self.lpop();
        let pred = self.try_reduce_expr(ExprCtx::EXPR).map_err(|_| {
            self.hint(switch_lang!(
                "japanese" => "予期: 述語式",
                "simplified_chinese" => "期望: 谓词表达式",
                "traditional_chinese" => "期望: 謂詞表達式",
                "english" => "expect: predicate expression",
            ))
        })?;
        let l_brace = Token::new_with_loc(LBrace, "{", var.loc());
        let r_brace = Token::new_with_loc(RBrace, "}", pred.loc());
        let comp = SetComprehension::new(l_brace, r_brace, None, vec![(var, typ)], Some(pred));
        Ok(Expr::Set(Set::Comprehension(comp)))
    }

    fn try_reduce_restriction(&mut self) -> ParseResult<VisRestriction> {
        trace!(self);
        self.expect(LSqBr)?;
        let rest = match self.peek_kind() {
            Some(SubtypeOf) => {
                self.lpop();
                let t_spec_as_expr = self.try_reduce_expr(ExprCtx {
                    in_type_args: true,
                    ..ExprCtx::EXPR
                })?;
                match Parser::expr_to_type_spec(t_spec_as_expr) {
                    Ok(t_spec) => VisRestriction::SubtypeOf(Box::new(t_spec)),
                    Err(err) => {
                        return self.fail(err);
                    }
                }
            }
            _ => {
                // FIXME: reduce namespaces
                let acc = self.try_reduce_acc_lhs()?;
                VisRestriction::Namespaces(Namespaces::new(vec![acc]))
            }
        };
        self.expect(RSqBr)?;
        Ok(rest)
    }

    fn try_reduce_ident(&mut self) -> ParseResult<Identifier> {
        trace!(self);
        let ident = match self.peek_kind() {
            Some(Symbol) => {
                let symbol = self.lpop();
                Identifier::private_from_token(symbol)
            }
            Some(Dot) => {
                let dot = self.lpop();
                let symbol = self.expect(Symbol)?;
                Identifier::public_from_token(dot, symbol)
            }
            _ => {
                return self.skip_and_throw_syntax_err();
            }
        };
        Ok(ident)
    }

    fn try_reduce_acc_lhs(&mut self) -> ParseResult<Accessor> {
        trace!(self);
        let acc = match self.peek_kind() {
            Some(Symbol | UBar) => Accessor::local(self.lpop()),
            Some(Dot) => {
                let dot = self.lpop();
                let maybe_symbol = self.lpop();
                if maybe_symbol.is(Symbol) {
                    Accessor::public(dot.loc(), maybe_symbol)
                } else {
                    return self.skip_and_throw_syntax_err();
                }
            }
            Some(DblColon) => {
                let dbl_colon = self.lpop();
                if let Some(LSqBr) = self.peek_kind() {
                    let rest = self.try_reduce_restriction()?;
                    let symbol = self.expect(Symbol)?;
                    Accessor::restricted(rest, symbol)
                } else {
                    let symbol = self.expect(Symbol)?;
                    Accessor::explicit_local(dbl_colon.loc(), symbol)
                }
            }
            _ => {
                return self.skip_and_throw_syntax_err();
            }
        };
        Ok(acc)
    }

    fn try_reduce_list_elems(&mut self) -> ParseResult<ListInner> {
        trace!(self);
        if self.cur_is(EOF) {
            let tk = self.tokens.last().unwrap();
            return self.fail(ParseError::expect_next_line_error(
                line!() as usize,
                tk.loc(),
                "Collections",
            ));
        }
        if self.cur_category_is(TC::REnclosure) {
            return Ok(ListInner::Normal(Args::empty()));
        }
        let first = self.try_reduce_elem()?;
        let mut elems = Args::single(first);
        match self.peek_kind() {
            Some(Semi) => {
                self.lpop();
                let len = self.try_reduce_expr(ExprCtx::EXPR).map_err(|_| {
                    self.hint(switch_lang!(
                        "japanese" => "予期: Nat型",
                        "simplified_chinese" => "期望: Nat类型",
                        "traditional_chinese" => "期望: Nat類型",
                        "english" => "expect: Nat type",
                    ))
                })?;
                return Ok(ListInner::WithLength(elems.remove_pos(0), len));
            }
            Some(PreStar) => {
                self.lpop();
                let rest = self.try_reduce_expr(ExprCtx::EXPR)?;
                elems.set_var_args(PosArg::new(rest));
                return Ok(ListInner::Normal(elems));
            }
            Some(Inclusion) => {
                self.lpop();
                let Expr::Accessor(Accessor::Ident(var)) = elems.remove_pos(0).expr else {
                    return self.skip_and_throw_invalid_seq_err(&["identifier"], Inclusion);
                };
                let (generators, guard) = self.try_reduce_guarded_generator(var)?;
                return Ok(ListInner::comp(None, generators, guard));
            }
            Some(VBar) => {
                self.lpop();
                let layout = elems.remove_pos(0).expr;
                let (generators, guard) = self.try_reduce_generators()?;
                return Ok(ListInner::comp(Some(layout), generators, guard));
            }
            Some(RParen | RSqBr | RBrace | Dedent | Comma) => {}
            Some(_) => {
                return self.skip_and_throw_invalid_unclosed_err("]", "array");
            }
            None => {
                return self.unexpected_none();
            }
        }
        loop {
            match self.peek_kind() {
                Some(Comma) => {
                    self.skip();
                    match self.peek_kind() {
                        Some(Comma) => {
                            return self.skip_and_throw_invalid_seq_err(&["]", "element"], Comma);
                        }
                        Some(RParen | RSqBr | RBrace | Dedent) => {
                            break;
                        }
                        Some(PreStar) => {
                            self.lpop();
                            let rest = self.try_reduce_expr(ExprCtx::EXPR)?;
                            elems.set_var_args(PosArg::new(rest));
                            break;
                        }
                        _ => {}
                    }
                    elems.push_pos(self.try_reduce_elem()?);
                }
                Some(RParen | RSqBr | RBrace | Dedent) => {
                    break;
                }
                Some(_other) => {
                    return self.skip_and_throw_invalid_unclosed_err("]", "array");
                }
                None => {
                    return self.unexpected_none();
                }
            }
        }
        Ok(ListInner::Normal(elems))
    }

    fn try_reduce_elem(&mut self) -> ParseResult<PosArg> {
        trace!(self);
        match self.peek() {
            Some(_) => Ok(PosArg::new(self.try_reduce_expr(ExprCtx::EXPR)?)),
            None => self.unexpected_none(),
        }
    }

    fn opt_reduce_args(&mut self, in_type_args: bool) -> Option<ParseResult<Args>> {
        trace!(self);
        match self.peek() {
            Some(t)
                if t.category_is(TC::Literal)
                    || t.is(StrInterpLeft)
                    || t.is(Symbol)
                    || t.category_is(TC::UnaryOp)
                    || t.is(LParen)
                    || t.is(LSqBr)
                    || t.is(LBrace)
                    || t.is(UBar) =>
            {
                Some(self.try_reduce_args(in_type_args))
            }
            Some(t)
                if (t.is(Dot) || t.is(DblColon))
                    && !self.nth_is(1, Newline)
                    && !self.nth_is(1, LBrace)
                    && !self.nth_is(1, LSqBr) =>
            {
                Some(self.try_reduce_args(in_type_args))
            }
            _ => None,
        }
    }

    /// 引数はインデントで区切ることができる(ただしコンマに戻すことはできない)
    ///
    /// ```erg
    /// x = if True, 1, 2
    /// # is equal to
    /// x = if True:
    ///     1
    ///     2
    /// ```
    fn try_reduce_args(&mut self, in_type_args: bool) -> ParseResult<Args> {
        trace!(self);
        let mut lp = None;
        if self.cur_is(LParen) {
            lp = Some(self.lpop());
        }
        let mut style = if lp.is_some() {
            ArgsStyle::SingleCommaWithParen
        } else {
            ArgsStyle::SingleCommaNoParen
        };
        match self.peek_kind() {
            Some(RParen) => {
                if let Some(lp) = lp {
                    let rp = self.lpop();
                    return Ok(Args::pos_only(vec![], Some((lp.loc(), rp.loc()))));
                }
                // no `(` was consumed; this `)` belongs to an outer expression
                return Ok(Args::empty());
            }
            Some(RBrace | RSqBr | Dedent) => {
                return Ok(Args::empty());
            }
            Some(Newline) if style.needs_parens() => {
                self.skip();
                if self.cur_is(Indent) {
                    self.skip();
                }
                style = ArgsStyle::MultiComma;
            }
            _ => {}
        }
        let mut args = Args::from(self.try_reduce_arg(in_type_args)?);
        loop {
            match self.peek_kind() {
                Some(Colon) if style.is_colon() || lp.is_some() => {
                    self.skip();
                    return self.skip_and_throw_syntax_err();
                }
                Some(Colon) => {
                    self.skip();
                    style = ArgsStyle::Colon;
                    self.skip_newlines();
                    self.expect_or_skip_line(Indent)?;
                }
                Some(Comma) => {
                    self.skip();
                    if style.is_colon() || self.cur_is(Comma) {
                        let loc = self.peek().map(|t| t.loc()).unwrap_or_default();
                        self.until_dedent();
                        return self.fail(ParseError::invalid_colon_style(line!() as usize, loc));
                    }
                    if style.is_multi_comma() {
                        self.skip_newlines();
                        if self.cur_is(Dedent) {
                            self.skip();
                        }
                    }
                    if style.needs_parens() && self.cur_is(RParen) {
                        let rp = self.lpop();
                        args.set_parens((lp.unwrap().loc(), rp.loc()));
                        break;
                    }
                    self.push_next_arg(&mut args, in_type_args)?;
                }
                Some(RParen) => {
                    if let Some(lp) = lp {
                        let rp = self.lpop();
                        args.set_parens((lp.loc(), rp.loc()));
                    } else {
                        // e.g. f(g 1)
                        let (pos_args, var_args, kw_args, kw_var, _) = args.deconstruct();
                        args = Args::new(pos_args, var_args, kw_args, kw_var, None);
                    }
                    break;
                }
                Some(Newline) => {
                    if !style.is_colon() {
                        if style.needs_parens() && !style.is_multi_comma() {
                            return self.skip_and_throw_invalid_seq_err(&[")"], Newline);
                        }
                        if style.is_multi_comma() {
                            self.skip();
                            while self.cur_is(Dedent) {
                                self.skip();
                            }
                            let rp = self.expect_or_skip_line(RParen)?;
                            args.set_parens((lp.unwrap().loc(), rp.loc()));
                        }
                        break;
                    }
                    let last = self.lpop();
                    if self.cur_is(Dedent) {
                        self.skip();
                        self.restore(last);
                        break;
                    }
                }
                Some(Dedent) if style.is_colon() => {
                    self.skip();
                    break;
                }
                Some(_) if style.is_colon() => {
                    self.push_next_arg(&mut args, in_type_args)?;
                }
                None => {
                    return self.unexpected_none();
                }
                _ => break,
            }
        }
        Ok(args)
    }

    /// Parse the next argument and append it to `args`, routing to keyword-argument
    /// parsing once any keyword argument has appeared. Shared by the `Comma` and
    /// colon-style arms of `try_reduce_args`.
    fn push_next_arg(&mut self, args: &mut Args, in_type_args: bool) -> ParseResult<()> {
        trace!(self);
        if !args.kw_is_empty() && !self.cur_is(PreDblStar) {
            let kw = self.try_reduce_kw_arg(in_type_args)?;
            args.push_kw(kw);
        } else {
            self.try_reduce_arg(in_type_args)?.push_to(args);
        }
        Ok(())
    }

    fn try_reduce_arg(&mut self, in_type_args: bool) -> ParseResult<ArgKind> {
        trace!(self);
        match self.peek_kind() {
            // `k := v`
            Some(Symbol) if self.nth_is(1, Walrus) => {
                self.try_reduce_kw_arg(in_type_args).map(ArgKind::Kw)
            }
            Some(Symbol) => {
                let expr = self
                    .try_reduce_expr(ExprCtx {
                        in_type_args,
                        ..ExprCtx::EXPR
                    })
                    .map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "予期: 型指定",
                            "simplified_chinese" => "期望: 类型规范",
                            "traditional_chinese" => "期望: 類型規範",
                            "english" => "expect: type specification",
                        ))
                    })?;
                // refinement pattern with an omitted type (e.g. `List(T, N | N >= 1)`)
                let expr = match &expr {
                    Expr::Accessor(Accessor::Ident(ident))
                        if !in_type_args && self.cur_is(VBar) && !self.nth_is(2, Inclusion) =>
                    {
                        let var = ident.clone();
                        let infer = Expr::Accessor(Accessor::Ident(Identifier::private_with_loc(
                            Str::ever("_"),
                            var.loc(),
                        )));
                        self.try_reduce_refinement_guard(var, infer)?
                    }
                    _ => expr,
                };
                if !self.cur_is(Walrus) {
                    return Ok(ArgKind::Pos(PosArg::new(expr)));
                }
                // `k: T := v`
                self.skip();
                let (kw, t_spec) = match expr {
                    Expr::Accessor(Accessor::Ident(n)) => (n.name.into_token(), None),
                    Expr::TypeAscription(tasc) => {
                        if let Expr::Accessor(Accessor::Ident(n)) = *tasc.expr {
                            (n.name.into_token(), Some(tasc.t_spec))
                        } else {
                            return self.skip_and_throw_invalid_seq_err(
                                &["right enclosure", "element"],
                                Comma,
                            );
                        }
                    }
                    _ => {
                        let err = ParseError::expect_keyword(line!() as usize, expr.loc());
                        self.next_expr();
                        return self.fail(err);
                    }
                };
                let expr = self
                    .try_reduce_expr(ExprCtx {
                        in_type_args,
                        ..ExprCtx::EXPR
                    })
                    .map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "予期: 型指定",
                            "simplified_chinese" => "期望: 类型规范",
                            "traditional_chinese" => "期望: 類型規範",
                            "english" => "expect: type specification",
                        ))
                    })?;
                Ok(ArgKind::Kw(KwArg::new(kw, t_spec, expr)))
            }
            Some(star @ (PreStar | PreDblStar)) => {
                self.skip();
                let expr = self
                    .try_reduce_expr(ExprCtx {
                        in_type_args,
                        ..ExprCtx::EXPR
                    })
                    .map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "予期: 型指定",
                            "simplified_chinese" => "期望: 类型规范",
                            "traditional_chinese" => "期望: 類型規範",
                            "english" => "expect: type specification",
                        ))
                    })?;
                if star == PreStar {
                    Ok(ArgKind::Var(PosArg::new(expr)))
                } else {
                    Ok(ArgKind::KwVar(PosArg::new(expr)))
                }
            }
            Some(_) => {
                let expr = self.try_reduce_expr(ExprCtx {
                    in_type_args,
                    ..ExprCtx::EXPR
                })?;
                Ok(ArgKind::Pos(PosArg::new(expr)))
            }
            None => self.unexpected_none(),
        }
    }

    /// The keyword of a keyword argument, the `k` of `k := v`: a plain identifier
    fn keyword_of(&mut self, acc: Accessor) -> ParseResult<Token> {
        match acc {
            Accessor::Ident(ident) => Ok(ident.name.into_token()),
            other => {
                let err = ParseError::expect_keyword(line!() as usize, other.loc());
                self.next_expr();
                self.fail(err)
            }
        }
    }

    fn try_reduce_kw_arg(&mut self, in_type_args: bool) -> ParseResult<KwArg> {
        trace!(self);
        match self.peek() {
            Some(t) if t.is(Symbol) => {
                if self.nth_is(1, Walrus) {
                    let acc = self.try_reduce_acc_lhs()?;
                    debug_power_assert!(self.cur_is(Walrus));
                    self.skip();
                    let keyword = self.keyword_of(acc)?;
                    let expr = self
                        .try_reduce_expr(ExprCtx {
                            in_type_args,
                            ..ExprCtx::EXPR
                        })
                        .map_err(|_| {
                            self.hint(switch_lang!(
                                "japanese" => "予期: 引数",
                                "simplified_chinese" => "期望: 参数",
                                "traditional_chinese" => "期望: 參數",
                                "english" => "expect: an argument",
                            ))
                        })?;
                    Ok(KwArg::new(keyword, None, expr))
                } else if self.nth_is(1, Colon) {
                    let acc = self.try_reduce_acc_lhs()?;
                    let colon = self.expect(Colon)?;
                    let t_spec_as_expr = self.try_reduce_expr(ExprCtx {
                        in_type_args: true,
                        ..ExprCtx::EXPR
                    })?;
                    let t_spec = match Parser::expr_to_type_spec(t_spec_as_expr.clone()) {
                        Ok(t_spec) => TypeSpecWithOp::new(colon, t_spec, t_spec_as_expr),
                        Err(err) => {
                            return self.fail(err);
                        }
                    };
                    debug_power_assert!(self.cur_is(Walrus));
                    self.skip();
                    let keyword = self.keyword_of(acc)?;
                    let expr = self
                        .try_reduce_expr(ExprCtx {
                            in_type_args,
                            ..ExprCtx::EXPR
                        })
                        .map_err(|_| {
                            self.hint(switch_lang!(
                                "japanese" => "予期: 引数",
                                "simplified_chinese" => "期望: 参数",
                                "traditional_chinese" => "期望: 參數",
                                "english" => "expect: an argument",
                            ))
                        })?;
                    Ok(KwArg::new(keyword, Some(t_spec), expr))
                } else {
                    let err = ParseError::invalid_non_default_parameter(line!() as usize, t.loc());
                    self.next_expr();
                    self.fail(err)
                }
            }
            Some(lit) if lit.category_is(TC::Literal) => {
                let err = ParseError::invalid_non_default_parameter(line!() as usize, lit.loc());
                self.next_expr();
                self.fail(err)
            }
            Some(other) => {
                let err = ParseError::expect_keyword(line!() as usize, other.loc());
                self.next_expr();
                self.fail(err)
            }
            None => self.unexpected_none(),
        }
    }

    fn try_reduce_class_attr_defs(
        &mut self,
        class: Expr,
        vis: VisModifierSpec,
    ) -> ParseResult<Methods> {
        trace!(self);
        self.expect_or_skip_line(Indent)?;
        self.skip_newlines();
        let first = self.try_reduce_expr(ExprCtx::CHUNK).map_err(|_| {
            self.hint(switch_lang!(
                "japanese" => "メソッドか属性のみ定義できます",
                "simplified_chinese" => "只能定义方法或属性",
                "traditional_chinese" => "只能定義方法或屬性",
                "english" => "only a method or attribute can be defined",
            ))
        })?;
        let first = match first {
            Expr::Def(def) => ClassAttr::Def(def),
            Expr::TypeAscription(tasc) => ClassAttr::Decl(tasc),
            Expr::Literal(lit) if lit.is_doc_comment() => ClassAttr::Doc(lit),
            other => {
                let hint = switch_lang!(
                    "japanese" => "メソッドか属性のみ定義できます",
                    "simplified_chinese" => "只能定义方法或属性",
                    "traditional_chinese" => "只能定義方法或屬性",
                    "english" => "only a method or attribute can be defined",
                )
                .to_string();
                self.until_dedent();
                return self.fail(ParseError::syntax_error(
                    line!() as usize,
                    other.loc(),
                    switch_lang!(
                        "japanese" => "クラス属性を定義するのに失敗しました",
                        "simplified_chinese" => "定义类属性失败",
                        "traditional_chinese" => "定義類屬性失敗",
                        "english" => "failed to define a class attribute",
                    ),
                    Some(hint),
                ));
            }
        };
        let mut attrs = vec![first];
        loop {
            match self.peek() {
                Some(t) if t.is(Newline) && self.nth_is(1, Dedent) => {
                    let nl = self.lpop();
                    self.skip();
                    self.restore(nl);
                    break;
                }
                Some(t) if t.is(Dedent) => {
                    self.skip();
                    break;
                }
                Some(t) if t.category_is(TC::Separator) => {
                    self.skip();
                }
                Some(_) => {
                    let def = self.try_reduce_expr(ExprCtx::CHUNK).map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "クラス属性かメソッドを定義してください",
                            "simplified_chinese" => "应声明类属性或方法",
                            "traditional_chinese" => "應聲明類屬性或方法",
                            "english" => "class attribute or method should be declared",
                        ))
                    })?;
                    match def {
                        Expr::Def(def) => {
                            attrs.push(ClassAttr::Def(def));
                        }
                        Expr::TypeAscription(tasc) => {
                            attrs.push(ClassAttr::Decl(tasc));
                        }
                        Expr::Literal(lit) if lit.is_doc_comment() => {
                            attrs.push(ClassAttr::Doc(lit));
                        }
                        other => {
                            self.next_expr();
                            return self.fail(ParseError::syntax_error(
                                line!() as usize,
                                other.loc(),
                                switch_lang!(
                                    "japanese" => "クラス属性を定義するのに失敗しました",
                                    "simplified_chinese" => "定义类属性失败",
                                    "traditional_chinese" => "定義類屬性失敗",
                                    "english" => "failed to define a class attribute",
                                ),
                                None,
                            ));
                        }
                    }
                    match self.peek() {
                        Some(t) if !t.is(Dedent) && !t.category_is(TC::Separator) => {
                            let err = self.skip_and_throw_invalid_chunk_err(t.loc());
                            return self.fail(err);
                        }
                        Some(_) => {}
                        None => {
                            return self.unexpected_none();
                        }
                    }
                }
                None => {
                    return self.unexpected_none();
                }
            }
        }
        let attrs = ClassAttrs::from(attrs);
        let t_spec = Self::expr_to_type_spec(class.clone()).map_err(|e| self.errs.push(e))?;
        self.counter.inc();
        Ok(Methods::new(self.counter, t_spec, class, vis, attrs))
    }

    fn try_reduce_do_block(&mut self) -> ParseResult<Lambda> {
        trace!(self);
        let do_symbol = self.lpop();
        let sig = LambdaSignature::do_sig(&do_symbol);
        let op = match &do_symbol.inspect()[..] {
            "do" => Token::from_str(FuncArrow, "->"),
            "do!" => Token::from_str(ProcArrow, "=>"),
            _ => unreachable!(),
        };
        if self.cur_is(Colon) {
            self.lpop();
            if self.at_eof() {
                return self.fail(ParseError::expect_next_line_error(
                    line!() as usize,
                    op.loc(),
                    "Lambda",
                ));
            }
            // On one line, `do:` is still a lambda, and a lambda binds tighter
            // than `,` (precedence: `->` > `,`). Reading the body as a block
            // would let it take a paren-less tuple, so `if c, do: a, do: b` came
            // out as `if(c, do: (a, do: b))` -- one branch holding both.
            let body = if self.cur_is(Newline) {
                self.try_reduce_block()?
            } else {
                let expr = self.try_reduce_expr(ExprCtx::EXPR)?;
                Block::new(vec![expr])
            };
            self.counter.inc();
            Ok(Lambda::new(sig, op, body, self.counter))
        } else {
            let expr = self.try_reduce_expr(ExprCtx::EXPR).map_err(|_| {
                self.hint(switch_lang!(
                    "japanese" => "予期: 式",
                    "simplified_chinese" => "期望: 表达",
                    "traditional_chinese" => "期望: 表達",
                    "english" => "expect: expression",
                ))
            })?;
            let block = Block::new(vec![expr]);
            Ok(Lambda::new(sig, op, block, self.counter))
        }
    }

    /// Detect and parse the `import` / `pyimport` syntactic sugar at statement level.
    ///
    /// Returns `None` when the current chunk is not an import statement, so the
    /// caller falls through to ordinary expression parsing. This keeps the
    /// historical call form (`foo = import "foo"`) working unchanged, since there
    /// the leading token is the bound name rather than `import`.
    ///
    /// Sugar forms (all desugar to the existing call form):
    /// - `import foo`                => `foo = import "foo"`
    /// - `import foo as f`           => `f = import "foo"`
    /// - `import foo/bar`            => `bar = import "foo/bar"`
    /// - `pyimport numpy`            => `numpy = pyimport "numpy"`
    /// - `pyimport numpy as np`      => `np = pyimport "numpy"`
    /// - `from foo import a, b`      => `{a; b} = import "foo"`
    /// - `from foo import a as x, b` => `{a as x; b} = import "foo"`
    /// - `pyfrom typing import T, U` => `{T; U} = pyimport "typing"`
    fn opt_reduce_import_sugar(&mut self) -> Option<ParseResult<Expr>> {
        let head = self.peek()?;
        if !head.is(Symbol) {
            return None;
        }
        // (underlying function name, whether this is a selective `from`-style import)
        let (func, selective) = match &head.content[..] {
            "import" => ("import", false),
            "pyimport" => ("pyimport", false),
            "from" => ("import", true),
            "pyfrom" => ("pyimport", true),
            _ => return None,
        };
        // A module path (bare name or string literal) must follow; otherwise this is
        // an ordinary use of the identifier (e.g. `import = ...`) and we bail out.
        match self.nth(1) {
            Some(t) if t.is(Symbol) || t.is(StrLit) => {}
            _ => return None,
        }
        Some(self.reduce_import_sugar(func, selective))
    }

    fn reduce_import_sugar(&mut self, func: &'static str, selective: bool) -> ParseResult<Expr> {
        trace!(self);
        let kw = self.lpop(); // `import` / `pyimport` / `from` / `pyfrom`
        let start = kw.loc();
        let (path, last, path_loc) = self.reduce_module_path();
        // build the underlying call `<func> "<path>"`
        let func_tok = Token::new_with_loc(Symbol, Str::ever(func), start);
        let str_tok = Token::new_with_loc(StrLit, Str::from(format!("\"{path}\"")), path_loc);
        let call =
            Expr::Accessor(Accessor::local(func_tok)).call1(Expr::Literal(Literal::new(str_tok)));
        let sig = if selective {
            // a literal `import` keyword separates the module from the imported names
            match self.peek() {
                Some(t) if t.is(Symbol) && &t.content[..] == "import" => {
                    self.skip();
                }
                _ => {
                    let loc = self.peek().map(|t| t.loc()).unwrap_or(path_loc);
                    let got = self.peek_kind().unwrap_or(EOF);
                    return self.fail(ParseError::unexpected_token(
                        line!() as usize,
                        loc,
                        "import",
                        got,
                    ));
                }
            }
            let attrs = self.reduce_import_names()?;
            let end = attrs.last().map(|a| a.loc()).unwrap_or(path_loc);
            let braces = Location::concat(&start, &end);
            let pat = VarRecordPattern::new(braces, VarRecordAttrs::new(attrs));
            Signature::Var(VarSignature::new(VarPattern::Record(pat), None, None))
        } else {
            // optional `as <name>` rebinds the module to a different name
            let name_tok = if self.cur_is(As) {
                self.skip();
                match self.peek() {
                    Some(t) if t.is(Symbol) => self.lpop(),
                    _ => {
                        let loc = self.peek().map(|t| t.loc()).unwrap_or(path_loc);
                        let got = self.peek_kind().unwrap_or(EOF);
                        let err =
                            ParseError::unexpected_token(line!() as usize, loc, "identifier", got);
                        return self.fail(err);
                    }
                }
            } else {
                Token::new_with_loc(Symbol, last, path_loc)
            };
            let ident = Identifier::private_from_token(name_tok);
            Signature::Var(VarSignature::new(VarPattern::Ident(ident), None, None))
        };
        self.counter.inc();
        let body = DefBody::new(crate::token::EQUAL, Block::new(vec![call]), self.counter);
        Ok(Expr::Def(Def::new(sig, body)))
    }

    /// Parse a module path: either a string literal (`"foo/bar"`) or a slash-separated
    /// chain of identifiers (`foo/bar`). Returns the joined path (without quotes), the
    /// last path component (used as the default bound name), and the covered location.
    fn reduce_module_path(&mut self) -> (Str, Str, Location) {
        if self.cur_is(StrLit) {
            let tok = self.lpop();
            let raw = tok.content.replace('\"', "");
            let last = raw.rsplit('/').next().unwrap_or("").to_string();
            (Str::from(raw), Str::from(last), tok.loc())
        } else {
            let first = self.lpop(); // guaranteed to be a Symbol by the caller
            let start = first.loc();
            let mut path = first.content.to_string();
            let mut last = first.content.to_string();
            let mut end = start;
            while self.cur_is(Slash) && self.nth_is(1, Symbol) {
                self.skip(); // `/`
                let seg = self.lpop();
                path.push('/');
                path.push_str(&seg.content);
                last = seg.content.to_string();
                end = seg.loc();
            }
            (
                Str::from(path),
                Str::from(last),
                Location::concat(&start, &end),
            )
        }
    }

    /// Parse the comma-separated name list of a `from ... import a, b as c` statement
    /// into record-destructuring attributes (`a` => `a = a`, `b as c` => `b = c`).
    fn reduce_import_names(&mut self) -> ParseResult<Vec<VarRecordAttr>> {
        trace!(self);
        let mut attrs = vec![];
        loop {
            let attr_tok = match self.peek() {
                Some(t) if t.is(Symbol) => self.lpop(),
                _ => {
                    let loc = self.peek().map(|t| t.loc()).unwrap_or(Location::Unknown);
                    let got = self.peek_kind().unwrap_or(EOF);
                    let err =
                        ParseError::unexpected_token(line!() as usize, loc, "identifier", got);
                    return self.fail(err);
                }
            };
            let local_tok = if self.cur_is(As) {
                self.skip();
                match self.peek() {
                    Some(t) if t.is(Symbol) => self.lpop(),
                    _ => {
                        let loc = self
                            .peek()
                            .map(|t| t.loc())
                            .unwrap_or_else(|| attr_tok.loc());
                        let got = self.peek_kind().unwrap_or(EOF);
                        let err =
                            ParseError::unexpected_token(line!() as usize, loc, "identifier", got);
                        return self.fail(err);
                    }
                }
            } else {
                attr_tok.clone()
            };
            let lhs = Identifier::private_from_token(attr_tok);
            let rhs = VarSignature::new(
                VarPattern::Ident(Identifier::private_from_token(local_tok)),
                None,
                None,
            );
            attrs.push(VarRecordAttr::new(lhs, rhs));
            if self.cur_is(Comma) {
                self.skip();
                // tolerate a trailing comma before the end of the statement
                if !self.cur_is(Symbol) {
                    break;
                }
                continue;
            }
            break;
        }
        Ok(attrs)
    }

    /// Parses one expression in the context `ctx`: an operand or an argument
    /// (`ExprCtx::EXPR`), or a statement, which may also be a definition
    /// (`ExprCtx::CHUNK`).
    fn try_reduce_expr(&mut self, ctx: ExprCtx) -> ParseResult<Expr> {
        // `import` / `pyimport` syntactic sugar is only recognized at statement level
        // (top-level chunks and block bodies, which are parsed with `winding == true`).
        if ctx.chunk && ctx.winding {
            if let Some(res) = self.opt_reduce_import_sugar() {
                return res;
            }
        }
        self.try_reduce_expr_prec(0, ctx)
    }

    /// The core expression parser (precedence climbing).
    ///
    /// A binary operator with precedence lower than `min_prec` terminates this level
    /// and is handled by an outer call, which makes all binary operators
    /// left-associative without an explicit operator stack.
    /// Postfix-like constructs (accessors, lambdas, type ascriptions, paren-less tuples)
    /// always bind to the nearest operand, so they are handled at any level.
    fn try_reduce_expr_prec(&mut self, min_prec: usize, ctx: ExprCtx) -> ParseResult<Expr> {
        trace!(self);
        let mut lhs = self.try_reduce_bin_lhs(ctx.in_type_args, ctx.in_brace)?;
        loop {
            match self.peek() {
                // expr + args is a function call (e.g. `(x -> x) 1`), only at statement level
                Some(t) if ctx.chunk && (t.is(Symbol) || t.category_is(TC::Literal)) => {
                    let args = self.try_reduce_args(false)?;
                    lhs = lhs.call_expr(args);
                }
                Some(op) if ctx.chunk && op.category_is(TC::DefOp) => {
                    let op = self.lpop();
                    if self.at_eof() {
                        return self.fail(ParseError::expect_next_line_error(
                            line!() as usize,
                            op.loc(),
                            "Assignment",
                        ));
                    }
                    if min_prec > 0 {
                        // e.g. `a + b = ...`: the lhs of `=` must be a sole expression
                        return self.extra_operator_err(op.loc());
                    }
                    let is_multiline_block = self.cur_is(Newline);
                    let sig = self.convert_rhs_to_sig(lhs)?;
                    self.counter.inc();
                    let block = if is_multiline_block {
                        self.try_reduce_block()?
                    } else {
                        // precedence: `=` < `,`
                        let expr = self
                            .try_reduce_expr(ExprCtx {
                                winding: true,
                                ..ExprCtx::EXPR
                            })
                            .map_err(|_| {
                                self.hint(switch_lang!(
                                    "japanese" => "予期: 式",
                                    "simplified_chinese" => "期望: 表达",
                                    "traditional_chinese" => "期望: 表達",
                                    "english" => "expect: expression",
                                ))
                            })?;
                        Block::new(vec![expr])
                    };
                    let body = DefBody::new(op, block, self.counter);
                    return Ok(Expr::Def(Def::new(sig, body)));
                }
                Some(op) if op.category_is(TC::LambdaOp) => {
                    let op = self.lpop();
                    if self.at_eof() {
                        return self.fail(ParseError::expect_next_line_error(
                            line!() as usize,
                            op.loc(),
                            "Lambda",
                        ));
                    }
                    let is_multiline_block = self.cur_is(Newline);
                    let sig = self.convert_rhs_to_lambda_sig(lhs)?;
                    self.counter.inc();
                    let block = if is_multiline_block {
                        self.try_reduce_block()?
                    } else {
                        // precedence: `->` > `,`
                        let expr = self.try_reduce_expr(ExprCtx::EXPR).map_err(|_| {
                            self.hint(switch_lang!(
                                "japanese" => "予期: 式",
                                "simplified_chinese" => "期望: 表达",
                                "traditional_chinese" => "期望: 表達",
                                "english" => "expect: expression",
                            ))
                        })?;
                        Block::new(vec![expr])
                    };
                    lhs = Expr::Lambda(Lambda::new(sig, op, block, self.counter));
                }
                // type ascription
                Some(op)
                    if (op.is(Colon) && !self.nth_is(1, Newline))
                        || (op.is(SubtypeOf) || op.is(SupertypeOf) || op.is(As)) =>
                {
                    // "a": 1 (key-value pair)
                    if ctx.in_brace {
                        break;
                    }
                    let op = self.lpop();
                    if self.at_eof() {
                        return self.fail(ParseError::expect_next_line_error(
                            line!() as usize,
                            op.loc(),
                            "TypeAscription",
                        ));
                    }
                    let (in_type_args, in_brace) = if ctx.chunk {
                        (false, false)
                    } else {
                        (ctx.in_type_args, ctx.in_brace)
                    };
                    let t_spec_as_expr = self
                        .try_reduce_expr(ExprCtx {
                            in_type_args,
                            in_brace,
                            ..ExprCtx::EXPR
                        })
                        .map(Desugarer::desugar_simple_expr)?;
                    // refinement pattern (e.g. `X: T | Pred` == `X: {X: T | Pred}`)
                    let t_spec_as_expr = match &lhs {
                        Expr::Accessor(Accessor::Ident(ident))
                            if !ctx.in_type_args
                                && self.cur_is(VBar)
                                && !self.nth_is(2, Inclusion) =>
                        {
                            self.try_reduce_refinement_guard(ident.clone(), t_spec_as_expr)
                                .map(Desugarer::desugar_simple_expr)?
                        }
                        _ => t_spec_as_expr,
                    };
                    let t_spec = Self::expr_to_type_spec(t_spec_as_expr.clone())
                        .map_err(|e| self.errs.push(e))?;
                    let t_spec_op = TypeSpecWithOp::new(op, t_spec, t_spec_as_expr);
                    lhs = lhs.type_asc_expr(t_spec_op);
                }
                // error propagation operator (e.g. `f()?`)
                // binds tighter than any binary operator, like `.attr` or `[i]`
                Some(t) if t.is(Try) => {
                    let op = self.lpop();
                    lhs = Expr::UnaryOp(UnaryOp::new(op, lhs));
                }
                Some(op) if op.category_is(TC::BinOp) => {
                    let op_prec = op.kind.precedence().unwrap_or(0);
                    if op_prec < min_prec {
                        break;
                    }
                    let op = self.lpop();
                    // parse the rhs operand together with all tighter-binding operators
                    let rhs = self.try_reduce_expr_prec(op_prec + 1, ctx).map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "予期: 式、被演算子",
                            "simplified_chinese" => "期望：表达式或操作数",
                            "traditional_chinese" => "期望：表達式或操作數",
                            "english" => "expect: expression or operand",
                        ))
                    })?;
                    lhs = if Self::is_chainable_comparison(op.kind) {
                        self.chain_comparison(op, lhs, rhs)?
                    } else {
                        Expr::BinOp(BinOp::new(op, lhs, rhs))
                    };
                }
                Some(t) if t.is(DblColon) && ctx.chunk => {
                    let dcolon = self.lpop();
                    let token = self.lpop();
                    match token.kind {
                        Symbol => {
                            let ident = Identifier::private_from_token(token);
                            if let Some(args) = self.opt_reduce_args(false).transpose()? {
                                lhs = Expr::Call(Call::new(lhs, Some(ident), args));
                            } else {
                                lhs = lhs.attr_expr(ident);
                            }
                        }
                        Newline => {
                            if min_prec > 0 {
                                return self.extra_operator_err(dcolon.loc());
                            }
                            let vis = VisModifierSpec::ExplicitPrivate(dcolon.loc());
                            return Ok(Expr::Methods(self.try_reduce_class_attr_defs(lhs, vis)?));
                        }
                        LSqBr => {
                            self.restore(token);
                            let restriction = self.try_reduce_restriction()?;
                            let vis = VisModifierSpec::Restricted(restriction);
                            self.expect(Newline)?;
                            if min_prec > 0 {
                                return self.extra_operator_err(dcolon.loc());
                            }
                            return Ok(Expr::Methods(self.try_reduce_class_attr_defs(lhs, vis)?));
                        }
                        LBrace => {
                            let vis = VisModifierSpec::ExplicitPrivate(dcolon.loc());
                            self.restore(token);
                            let container = self.try_reduce_brace_container()?;
                            match container {
                                BraceContainer::Record(args) => {
                                    lhs = Expr::DataPack(DataPack::new(lhs, vis, args));
                                }
                                other => {
                                    return self.fail(ParseError::invalid_data_pack_definition(
                                        line!() as usize,
                                        other.loc(),
                                        other.kind(),
                                    ));
                                }
                            }
                        }
                        _ => {
                            self.restore(token);
                            return self.skip_and_throw_syntax_err();
                        }
                    }
                }
                Some(t) if t.is(Dot) => {
                    let dot = self.lpop();
                    let token = self.lpop();
                    match token.kind {
                        Symbol => {
                            let ident = Identifier::public_from_token(dot, token);
                            if let Some(args) =
                                self.opt_reduce_args(ctx.in_type_args).transpose()?
                            {
                                lhs = Expr::Call(Call::new(lhs, Some(ident), args));
                            } else {
                                lhs = lhs.attr_expr(ident);
                            }
                        }
                        Newline if ctx.chunk => {
                            if min_prec > 0 {
                                return self.extra_operator_err(dot.loc());
                            }
                            let vis = VisModifierSpec::Public(dot.loc());
                            return Ok(Expr::Methods(self.try_reduce_class_attr_defs(lhs, vis)?));
                        }
                        _ => {
                            self.restore(token);
                            return self.skip_and_throw_syntax_err();
                        }
                    }
                }
                Some(t) if t.is(LSqBr) => {
                    self.skip(); // l_sqbr
                    let index = self
                        .try_reduce_expr(ExprCtx {
                            in_brace: ctx.in_brace,
                            ..ExprCtx::EXPR
                        })
                        .map_err(|_| {
                            self.hint(switch_lang!(
                                "japanese" => "予期: Nat型",
                                "simplified_chinese" => "期望: Nat类型",
                                "traditional_chinese" => "期望: Nat類型",
                                "english" => "expect: Nat type",
                            ))
                        })?;
                    let r_sqbr = self.expect_or_skip_line(RSqBr)?;
                    lhs = Expr::Accessor(Accessor::subscr(lhs, index, r_sqbr));
                }
                Some(t) if t.is(Comma) && ctx.winding => {
                    let first_elem = ArgKind::Pos(PosArg::new(lhs));
                    let tup = self.try_reduce_nonempty_tuple(first_elem, ctx.line_break)?;
                    lhs = Expr::Tuple(tup);
                }
                Some(t) if t.is(Walrus) && ctx.winding => {
                    let tuple = self.try_reduce_default_parameters(lhs, ctx.in_brace)?;
                    lhs = Expr::Tuple(tuple);
                }
                Some(t) if t.is(Pipe) => {
                    // `|>` binds loosest; only the outermost level may consume it
                    if min_prec > 0 {
                        break;
                    }
                    lhs = self.try_reduce_stream_operator(lhs)?;
                }
                Some(t) if t.category_is(TC::Reserved) => {
                    return self.skip_and_throw_syntax_err();
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    #[inline]
    fn try_reduce_default_parameters(
        &mut self,
        first_elem: Expr,
        in_brace: bool,
    ) -> ParseResult<Tuple> {
        trace!(self);
        let (keyword, t_spec) = match first_elem {
            Expr::Accessor(Accessor::Ident(ident)) => (ident.name.into_token(), None),
            Expr::TypeAscription(tasc) => {
                if let Expr::Accessor(Accessor::Ident(ident)) = *tasc.expr {
                    (ident.name.into_token(), Some(tasc.t_spec))
                } else {
                    self.next_expr();
                    return self.fail(ParseError::expect_keyword(line!() as usize, tasc.loc()));
                }
            }
            // foo, arg/*args/**kwargs | := rhs
            Expr::Tuple(Tuple::Normal(mut tuple)) => {
                let rhs = self.try_reduce_default_value(in_brace)?;
                if tuple.elems.kw_var_args.is_some() {
                    tuple.elems.set_kw_var(PosArg::new(rhs));
                } else if tuple.elems.var_args.is_some() {
                    tuple.elems.set_var_args(PosArg::new(rhs));
                } else {
                    let (kw, t_spec) = match tuple.elems.pos_args.pop().unwrap().expr {
                        Expr::Accessor(Accessor::Ident(ident)) => (ident.name.into_token(), None),
                        Expr::TypeAscription(tasc) => {
                            let kw = if let Expr::Accessor(Accessor::Ident(ident)) = *tasc.expr {
                                ident.name.into_token()
                            } else {
                                Token::symbol("_")
                            };
                            (kw, Some(tasc.t_spec))
                        }
                        _ => (Token::symbol("_"), None),
                    };
                    tuple.elems.push_kw(KwArg::new(kw, t_spec, rhs));
                }
                return Ok(Tuple::Normal(tuple));
            }
            other => {
                self.next_expr();
                return self.fail(ParseError::expect_keyword(line!() as usize, other.loc()));
            }
        };
        let rhs = self.try_reduce_default_value(in_brace)?;
        let first_elem = ArgKind::Kw(KwArg::new(keyword, t_spec, rhs));
        self.try_reduce_nonempty_tuple(first_elem, self.nth_is(1, Newline))
    }

    /// The `:= value` of a default parameter; the cursor is at the `:=`
    fn try_reduce_default_value(&mut self, in_brace: bool) -> ParseResult<Expr> {
        self.skip(); // :=
        self.try_reduce_expr(ExprCtx {
            in_brace,
            ..ExprCtx::EXPR
        })
        .map_err(|_| {
            self.hint(switch_lang!(
                "japanese" => "予期: デフォルト引数",
                "simplified_chinese" => "期望: 默认参数",
                "traditional_chinese" => "期望: 默認參數",
                "english" => "expect: default parameter",
            ))
        })
    }

    /// Build the `Float(<expr>)` conversion that the `f64` / `f32` literal suffix
    /// desugars to. Decimal literals (`2.5`) are `Ratio`, so the suffix is the
    /// canonical way to construct an actual `Float` value in source.
    fn float_suffix(expr: Expr) -> Expr {
        let loc = expr.loc();
        let float_tok = Token::new_with_loc(Symbol, Str::ever("Float"), loc);
        Expr::Accessor(Accessor::local(float_tok)).call1(expr)
    }

    /// "LHS" is the smallest unit that can be the left-hand side of a BinOp,
    /// e.g. Call, Name, UnaryOp, Lambda
    fn try_reduce_bin_lhs(&mut self, in_type_args: bool, in_brace: bool) -> ParseResult<Expr> {
        trace!(self);
        match self.peek() {
            Some(t) if &t.inspect()[..] == "do" || &t.inspect()[..] == "do!" => {
                Ok(Expr::Lambda(self.try_reduce_do_block()?))
            }
            Some(t) if t.category_is(TC::Literal) => {
                let lit = self.try_reduce_lit()?;
                if let Some(tk) = self.peek() {
                    if tk.is(Mutate) {
                        self.skip();
                        let main_msg = switch_lang!(
                            "japanese" => "可変演算子は、後置演算子ではなく前置演算子です ",
                            "simplified_chinese" => "突变运算符是前缀运算符，不是后缀运算符",
                            "traditional_chinese" => "突變運算符是前綴運算符，不是後綴運算符",
                            "english" => "the mutation operator is a prefix operator, not a postfix operator ",
                        );
                        let lit_loc = lit.loc();
                        let lit = lit.token.inspect();
                        return self.fail(ParseError::invalid_token_error(
                            line!() as usize,
                            lit_loc,
                            main_msg,
                            &format!("!{lit}"),
                            &format!("{lit}!"),
                        ));
                    } else if lit.is_number()
                        && tk.is(Symbol)
                        && matches!(&tk.inspect()[..], "f64" | "f32")
                        && lit.col_end() == tk.col_begin()
                    {
                        // Float literal suffix: `2.5f64` / `15f32` => `Float(2.5)`.
                        // Decimal literals are `Ratio`; the adjacent `f64` / `f32`
                        // unit suffix constructs an actual `Float`
                        // (see doc/EN/syntax/01_literal.md).
                        self.skip(); // `f64` / `f32`
                        return Ok(Self::float_suffix(Expr::Literal(lit)));
                    } else if lit.is_number() && tk.is(Symbol) {
                        // *-less multiplication (e.g. 3x, 3x.y)
                        let rhs = self.try_reduce_call_or_acc(false)?;
                        let lhs = Expr::Literal(lit);
                        let op = Token::dummy(Star, "*");
                        return Ok(Expr::BinOp(lhs.bin_op(op, rhs)));
                    } else if lit.is_number() && tk.is(LParen) {
                        // *-less multiplication (e.g. 3(4+1), 3(4+1).y)
                        self.lpop();
                        let rhs = self.try_reduce_expr(ExprCtx::EXPR)?;
                        self.expect(RParen)?;
                        let lhs = Expr::Literal(lit);
                        let op = Token::dummy(Star, "*");
                        return Ok(Expr::BinOp(lhs.bin_op(op, rhs)));
                    }
                }
                Ok(Expr::Literal(lit))
            }
            Some(t) if t.is(StrInterpLeft) => self.try_reduce_string_interpolation(),
            Some(t) if t.is(AtSign) => {
                let decos = self.opt_reduce_decorators()?;
                let expr = self
                    .try_reduce_expr(ExprCtx {
                        in_brace,
                        ..ExprCtx::CHUNK
                    })
                    .map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "期待: デコレータ",
                            "simplified_chinese" => "期望: 装饰器",
                            "traditional_chinese" => "期望: 裝飾器",
                            "english" => "expect: decorator",
                        ))
                    })?;
                let Expr::Def(mut def) = expr else {
                    // self.restore(other);
                    return self.skip_and_throw_syntax_err();
                };
                match def.sig {
                    Signature::Subr(mut subr) => {
                        subr.decorators = decos;
                        Ok(Expr::Def(Def::new(Signature::Subr(subr), def.body)))
                    }
                    Signature::Var(var) => {
                        let mut last = def.body.block.pop().unwrap();
                        for deco in decos.into_iter() {
                            last = deco.into_expr().call_expr(Args::single(PosArg::new(last)));
                        }
                        def.body.block.push(last);
                        Ok(Expr::Def(Def::new(Signature::Var(var), def.body)))
                    }
                }
            }
            Some(t) if t.is(Symbol) || t.is(Dot) || t.is(DblColon) || t.is(UBar) => {
                self.try_reduce_call_or_acc(in_type_args)
            }
            // REVIEW: correct?
            Some(t) if t.is(PreStar) || t.is(PreDblStar) => {
                let kind = t.kind;
                self.skip();
                let expr = self
                    .try_reduce_expr(ExprCtx {
                        in_type_args,
                        in_brace,
                        ..ExprCtx::EXPR
                    })
                    .map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "期待: 可変長引数",
                            "simplified_chinese" => "期望: 可变长度参数",
                            "traditional_chinese" => "期望: 可變長度參數",
                            "english" => "expect: variable-length arguments",
                        ))
                    })?;
                let arg = match kind {
                    PreStar => ArgKind::Var(PosArg::new(expr)),
                    PreDblStar => ArgKind::KwVar(PosArg::new(expr)),
                    _ => switch_unreachable!(),
                };
                Ok(Expr::Tuple(self.try_reduce_nonempty_tuple(arg, false)?))
            }
            Some(t) if t.category_is(TC::UnaryOp) => Ok(Expr::UnaryOp(self.try_reduce_unary()?)),
            Some(t) if t.is(LParen) => {
                let lparen = self.lpop();
                self.skip_newlines();
                let line_break = if self.cur_is(Indent) {
                    self.skip();
                    true
                } else {
                    false
                };
                if self.cur_is(RParen) {
                    let rparen = self.lpop();
                    let args = Args::pos_only(vec![], Some((lparen.loc(), rparen.loc())));
                    return Ok(Expr::Tuple(Tuple::Normal(NormalTuple::new(args))));
                }
                let mut expr = self
                    .try_reduce_expr(ExprCtx {
                        winding: true,
                        line_break,
                        ..ExprCtx::EXPR
                    })
                    .map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "期待: 要素",
                            "simplified_chinese" => "期望: 元素",
                            "traditional_chinese" => "期望: 元素",
                            "english" => "expect: an element",
                        ))
                    })?;
                // tuple comprehension: `(expr | x <- xs)`, `(x <- xs | guard)`
                if self.cur_is(Inclusion) || (self.cur_is(VBar) && self.nth_is(2, Inclusion)) {
                    let tup = self.try_reduce_tuple_comprehension(lparen, expr, line_break)?;
                    return Ok(Expr::Tuple(Tuple::Comprehension(tup)));
                }
                if line_break {
                    self.skip_newlines();
                    if self.cur_is(Dedent) {
                        self.skip();
                    }
                }
                let rparen = match self.peek_kind() {
                    Some(RParen) => self.lpop(),
                    Some(_) => {
                        return self.skip_and_throw_invalid_unclosed_err(")", "tuple");
                    }
                    None => {
                        return self.unexpected_none();
                    }
                };
                if let Expr::Tuple(Tuple::Normal(tup)) = &mut expr {
                    tup.elems.paren = Some((lparen.loc(), rparen.loc()));
                } else {
                    self.parenthesized.insert(expr.loc());
                }
                // Float suffix on a parenthesized expression: `(1/2)f64` => `Float(1/2)`
                if let Some(tk) = self.peek() {
                    if tk.is(Symbol)
                        && matches!(&tk.inspect()[..], "f64" | "f32")
                        && rparen.col_end() == tk.col_begin()
                    {
                        self.skip(); // `f64` / `f32`
                        expr = Self::float_suffix(expr);
                    }
                }
                Ok(expr)
            }
            Some(t) if t.is(LSqBr) => Ok(Expr::List(self.try_reduce_list()?)),
            Some(t) if t.is(LBrace) => Ok(Expr::from(self.try_reduce_brace_container()?)),
            Some(t) if t.is(VBar) => {
                let type_args = self.try_reduce_type_app_args()?;
                let bounds = self.convert_type_args_to_bounds(type_args)?;
                let args = self.try_reduce_args(false)?;
                let params = self.convert_args_to_params(args)?;
                let sig = LambdaSignature::new(params, None, bounds);
                let op = self.expect_category(TC::LambdaOp)?;
                let block = self.try_reduce_block()?;
                self.counter.inc();
                Ok(Expr::Lambda(Lambda::new(sig, op, block, self.counter)))
            }
            Some(t) if t.is(UBar) => {
                let token = self.lpop();
                self.fail(ParseError::feature_error(
                    line!() as usize,
                    token.loc(),
                    "discard pattern",
                ))
            }
            Some(_other) => self.skip_and_throw_syntax_err(),
            None => self.unexpected_none(),
        }
    }

    #[inline]
    fn try_reduce_call_or_acc(&mut self, in_type_args: bool) -> ParseResult<Expr> {
        trace!(self);
        let acc = self.try_reduce_acc_lhs()?;
        let mut call_or_acc = self.try_reduce_acc_chain(acc, in_type_args)?;
        while let Some(res) = self.opt_reduce_args(in_type_args) {
            let args = res?;
            let call = call_or_acc.call(args);
            call_or_acc = Expr::Call(call);
        }
        Ok(call_or_acc)
    }

    /// [y], .0, .attr, .method(...), (...)
    #[inline]
    fn try_reduce_acc_chain(&mut self, acc: Accessor, in_type_args: bool) -> ParseResult<Expr> {
        trace!(self);
        let mut obj = Expr::Accessor(acc);
        loop {
            match self.peek() {
                Some(t) if t.is(LSqBr) && obj.col_end() == t.col_begin() => {
                    let _l_sqbr = self.lpop();
                    let index = self
                        .try_reduce_expr(ExprCtx {
                            winding: true,
                            ..ExprCtx::EXPR
                        })
                        .map_err(|_| {
                            self.hint(switch_lang!(
                                "japanese" => "期待: Nat型",
                                "simplified_chinese" => "期望: Nat类型",
                                "traditional_chinese" => "期望: Nat類型",
                                "english" => "expect: Nat type",
                            ))
                        })?;
                    let r_sqbr = self.expect_or_skip_line(RSqBr)?;
                    obj = Expr::Accessor(Accessor::subscr(obj, index, r_sqbr));
                }
                Some(t) if t.is(Dot) && obj.col_end() == t.col_begin() => {
                    let vis = self.lpop();
                    let token = self.lpop();
                    match token.kind {
                        Symbol => {
                            let ident = Identifier::new(
                                VisModifierSpec::Public(vis.loc()),
                                VarName::new(token),
                            );
                            obj = obj.attr_expr(ident);
                        }
                        NatLit => {
                            let index = Literal::from(token);
                            obj = obj.tuple_attr_expr(index);
                        }
                        Newline => {
                            self.restore(token);
                            self.restore(vis);
                            break;
                        }
                        EOF => {
                            let err = ParseError::expect_next_line_error(
                                line!() as usize,
                                token.loc(),
                                "ClassPub",
                            );
                            self.restore(token);
                            return self.fail(err);
                        }
                        _ => {
                            return self.fail(ParseError::invalid_acc_chain(
                                line!() as usize,
                                token.loc(),
                                &token.inspect()[..],
                            ));
                        }
                    }
                }
                // e.g. l[0].0
                Some(t) if t.is(RatioLit) && obj.col_end() == t.col_begin() => {
                    let mut token = self.lpop();
                    token.content = Str::rc(&token.content[1..]);
                    token.kind = NatLit;
                    token.col_begin += 1;
                    obj = obj.tuple_attr_expr(Literal::from(token));
                }
                Some(t) if t.is(DblColon) && obj.col_end() == t.col_begin() => {
                    let vis = self.lpop();
                    let token = self.lpop();
                    match token.kind {
                        Symbol => {
                            let ident = Identifier::new(
                                VisModifierSpec::ExplicitPrivate(vis.loc()),
                                VarName::new(token),
                            );
                            obj = obj.attr_expr(ident);
                        }
                        LBrace => {
                            self.restore(token);
                            let args = self.try_reduce_brace_container()?;
                            match args {
                                BraceContainer::Record(args) => {
                                    let vis = VisModifierSpec::ExplicitPrivate(vis.loc());
                                    obj = Expr::DataPack(DataPack::new(obj, vis, args));
                                }
                                other => {
                                    return self.fail(ParseError::invalid_data_pack_definition(
                                        line!() as usize,
                                        other.loc(),
                                        other.kind(),
                                    ));
                                }
                            }
                        }
                        // MethodDefs
                        Newline | LSqBr => {
                            self.restore(token);
                            self.restore(vis);
                            break;
                        }
                        EOF => {
                            let err = ParseError::expect_next_line_error(
                                line!() as usize,
                                token.loc(),
                                "ClassPriv",
                            );
                            self.restore(token);
                            return self.fail(err);
                        }
                        _ => {
                            self.restore(token);
                            return self.skip_and_throw_syntax_err();
                        }
                    }
                }
                Some(t) if t.is(LParen) && obj.col_end() == t.col_begin() => {
                    let args = self.try_reduce_args(false)?;
                    let (receiver, attr_name) = match obj {
                        Expr::Accessor(Accessor::Attr(attr)) => (*attr.obj, Some(attr.ident)),
                        other => (other, None),
                    };
                    let call = Call::new(receiver, attr_name, args);
                    obj = Expr::Call(call);
                }
                // a type application `T|...|` requires that `|` directly follows `T`
                // (a spaced `|` is a refinement pattern guard, e.g. `N: Nat | N >= 1`)
                Some(t)
                    if t.is(VBar)
                        && obj.col_end() == t.col_begin()
                        && !in_type_args
                        && !self.nth_is(2, Inclusion) =>
                {
                    let type_args = self.try_reduce_type_app_args()?;
                    obj = Expr::Accessor(Accessor::TypeApp(TypeApp::new(obj, type_args)));
                }
                _ => {
                    break;
                }
            }
        }
        Ok(obj)
    }

    #[inline]
    fn try_reduce_unary(&mut self) -> ParseResult<UnaryOp> {
        trace!(self);
        let op = self.lpop();
        let expr = self.try_reduce_expr(ExprCtx::EXPR).map_err(|_| {
            self.hint(switch_lang!(
                "japanese" => "予期: 式",
                "simplified_chinese" => "期待：表达式",
                "traditional_chinese" => "期待：表達式",
                "english" => "expect: expression",
            ))
        })?;
        Ok(UnaryOp::new(op, expr))
    }

    #[inline]
    fn try_reduce_list(&mut self) -> ParseResult<List> {
        trace!(self);
        let l_sqbr = self.expect_or_skip_line(LSqBr)?;
        let inner = self.try_reduce_list_elems()?;
        let r_sqbr = self.expect_or_skip_line(RSqBr)?;
        let lis = match inner {
            ListInner::Normal(mut elems) => {
                let elems = if elems
                    .pos_args()
                    .first()
                    .map(|pos| match &pos.expr {
                        Expr::Tuple(tup) => tup.paren().is_none(),
                        _ => false,
                    })
                    .unwrap_or(false)
                {
                    enum_unwrap!(elems.remove_pos(0).expr, Expr::Tuple:(Tuple::Normal:(_))).elems
                } else {
                    elems
                };
                List::Normal(NormalList::new(l_sqbr, r_sqbr, elems))
            }
            ListInner::WithLength(elem, len) => {
                List::WithLength(ListWithLength::new(l_sqbr, r_sqbr, elem, len))
            }
            ListInner::Comprehension {
                layout,
                generators,
                guard,
            } => List::Comprehension(ListComprehension::new(
                l_sqbr, r_sqbr, layout, generators, guard,
            )),
        };
        Ok(lis)
    }

    /// tuple comprehension: `(expr | x <- xs)`, `(x <- xs | guard)`
    /// the l_paren has already been popped, and the cursor is at `|` or `<-`
    fn try_reduce_tuple_comprehension(
        &mut self,
        l_paren: Token,
        first: Expr,
        line_break: bool,
    ) -> ParseResult<TupleComprehension> {
        trace!(self);
        let (layout, (generators, guard)) = match self.peek_kind() {
            Some(Inclusion) => {
                self.lpop();
                let Expr::Accessor(Accessor::Ident(var)) = first else {
                    return self.skip_and_throw_invalid_seq_err(&["identifier"], Inclusion);
                };
                (None, self.try_reduce_guarded_generator(var)?)
            }
            Some(VBar) => {
                self.lpop();
                (Some(first), self.try_reduce_generators()?)
            }
            _ => {
                return self.unexpected_none();
            }
        };
        if line_break {
            self.skip_newlines();
            if self.cur_is(Dedent) {
                self.skip();
            }
        }
        let r_paren = match self.peek_kind() {
            Some(RParen) => self.lpop(),
            Some(_) => {
                return self.skip_and_throw_invalid_unclosed_err(")", "tuple");
            }
            None => {
                return self.unexpected_none();
            }
        };
        Ok(TupleComprehension::new(
            l_paren, r_paren, layout, generators, guard,
        ))
    }

    /// The generators of a comprehension, `x <- xs; y <- ys`, and its optional
    /// `| guard`; the cursor is just past the `|` that opens them
    fn try_reduce_generators(&mut self) -> ParseResult<Generators> {
        trace!(self);
        let mut generators = vec![];
        loop {
            let var = self.try_reduce_ident()?;
            self.expect_or_skip_line(Inclusion)?;
            let iter = self.try_reduce_expr(ExprCtx::EXPR)?;
            generators.push((var, iter));
            if !self.cur_is(Semi) {
                break;
            }
            self.skip();
        }
        let guard = if self.cur_is(VBar) {
            self.skip();
            Some(self.try_reduce_expr(ExprCtx::EXPR)?)
        } else {
            None
        };
        Ok((generators, guard))
    }

    /// The rest of a comprehension without a layout, `[x <- xs | guard]`: `var` is
    /// its `x`, and the cursor is just past the `<-`
    fn try_reduce_guarded_generator(&mut self, var: Identifier) -> ParseResult<Generators> {
        trace!(self);
        let iter = self.try_reduce_expr(ExprCtx::EXPR)?;
        self.expect(VBar)?;
        let guard = self.try_reduce_expr(ExprCtx::EXPR)?;
        Ok((vec![(var, iter)], Some(guard)))
    }

    /// Set, Dict, Record
    fn try_reduce_brace_container(&mut self) -> ParseResult<BraceContainer> {
        trace!(self);
        let l_brace = self.expect_or_skip_line(LBrace)?;
        if self.cur_is(EOF) {
            let err =
                ParseError::expect_next_line_error(line!() as usize, l_brace.loc(), "Collections");
            return self.fail(err);
        }
        // Empty brace literals
        match self.peek_kind() {
            Some(RBrace) => {
                let r_brace = self.lpop();
                let arg = Args::empty();
                let set = NormalSet::new(l_brace, r_brace, arg);
                return Ok(BraceContainer::Set(Set::Normal(set)));
            }
            Some(Assign) => {
                let _eq = self.lpop();
                if let Some(t) = self.peek() {
                    if t.is(RBrace) {
                        let r_brace = self.lpop();
                        return Ok(BraceContainer::Record(Record::empty(l_brace, r_brace)));
                    }
                } else {
                    return self.unexpected_none();
                }
                let t = self.lpop();
                let mut err = ParseError::invalid_token_error(
                    line!() as usize,
                    t.loc(),
                    switch_lang!(
                        "japanese" => "無効なレコードの宣言です",
                        "simplified_chinese" => "无效的Record定义",
                        "traditional_chinese" => "無效的Record定義",
                        "english" => "invalid record",
                    ),
                    "}",
                    &t.inspect()[..],
                );
                err.set_hint(switch_lang!(
                    "japanese" => "空のレコードが期待されています: {=}",
                    "simplified_chinese" => "期望空Record: {=}",
                    "traditional_chinese" => "期望空Record: {=}",
                    "english" => "expect empty record: {=}",
                ));
                return self.fail(err);
            }
            Some(Colon) => {
                let _colon = self.lpop();
                if let Some(t) = self.peek() {
                    if t.is(RBrace) {
                        let r_brace = self.lpop();
                        let dict = NormalDict::new(l_brace, r_brace, vec![]);
                        return Ok(BraceContainer::Dict(Dict::Normal(dict)));
                    }
                } else {
                    return self.unexpected_none();
                }
                let t = self.lpop();
                let mut err = ParseError::invalid_token_error(
                    line!() as usize,
                    t.loc(),
                    switch_lang!(
                        "japanese" => "無効な辞書の宣言です",
                        "simplified_chinese" => "无效的字典定义",
                        "traditional_chinese" => "無效的字典定義",
                        "english" => "invalid dict",
                    ),
                    "}",
                    &t.inspect()[..],
                );
                err.set_hint(switch_lang!(
                    "japanese" => "空の辞書が期待されています: {:}",
                    "simplified_chinese" => "期望空字典: {:}",
                    "traditional_chinese" => "期望空字典: {:}",
                    "english" => "expect empty dict: {:}",
                ));
                return self.fail(err);
            }
            _ => {}
        }

        let first = self
            .try_reduce_expr(ExprCtx {
                in_brace: true,
                ..ExprCtx::CHUNK
            })
            .map_err(|_| {
                self.hint(switch_lang!(
                    "japanese" => "期待: 要素",
                    "simplified_chinese" => "期望: 元素",
                    "traditional_chinese" => "期望: 元素",
                    "english" => "expect: an element",
                ))
            })?;
        match first {
            Expr::Def(def) => {
                let attr = RecordAttrOrIdent::Attr(def);
                Ok(BraceContainer::Record(
                    self.try_reduce_record(l_brace, attr)?,
                ))
            }
            // TODO: {X; Y} will conflict with Set
            Expr::Accessor(acc)
                if self.cur_is(Semi)
                    && !self.nth_is(1, TokenKind::NatLit)
                    && !self.nth_is(1, UBar) =>
            {
                let ident = match acc {
                    Accessor::Ident(ident) => ident,
                    other => {
                        let err =
                            ParseError::invalid_record_element_err(line!() as usize, other.loc());
                        self.next_expr();
                        return self.fail(err);
                    }
                };
                let attr = RecordAttrOrIdent::Ident(ident);
                Ok(BraceContainer::Record(
                    self.try_reduce_record(l_brace, attr)?,
                ))
            }
            other => {
                match self.peek_kind() {
                    Some(Colon) => {
                        return self.try_reduce_normal_dict_or_refine_type(l_brace, other);
                    }
                    Some(Inclusion) => {
                        self.skip();
                        let Expr::Accessor(Accessor::Ident(var)) = other else {
                            let err = ParseError::invalid_record_element_err(
                                line!() as usize,
                                other.loc(),
                            );
                            self.next_expr();
                            return self.fail(err);
                        };
                        let (generators, guard) = self.try_reduce_guarded_generator(var)?;
                        let r_brace = self.expect_or_skip_line(RBrace)?;
                        let comp = SetComprehension::new(l_brace, r_brace, None, generators, guard);
                        return Ok(BraceContainer::Set(Set::Comprehension(comp)));
                    }
                    Some(VBar) => {
                        self.skip();
                        let (generators, guard) = self.try_reduce_generators()?;
                        let r_brace = self.expect_or_skip_line(RBrace)?;
                        let comp =
                            SetComprehension::new(l_brace, r_brace, Some(other), generators, guard);
                        return Ok(BraceContainer::Set(Set::Comprehension(comp)));
                    }
                    Some(RBrace) => {
                        let arg = Args::new(vec![PosArg::new(other)], None, vec![], None, None);
                        let r_brace = self.lpop();
                        return Ok(BraceContainer::Set(Set::Normal(NormalSet::new(
                            l_brace, r_brace, arg,
                        ))));
                    }
                    _ => {}
                }
                Ok(BraceContainer::Set(self.try_reduce_set(l_brace, other)?))
            }
        }
    }

    // Note that this accepts:
    //  - {x=expr;y=expr;...}
    //  - {x;y}
    //  - {x;y=expr} (shorthand/normal mixed)
    fn try_reduce_record(
        &mut self,
        l_brace: Token,
        first_attr: RecordAttrOrIdent,
    ) -> ParseResult<Record> {
        trace!(self);
        let mut attrs = vec![first_attr];
        loop {
            match self.peek_kind() {
                Some(Newline | Semi) => {
                    self.skip();
                    match self.peek() {
                        Some(t) if t.is(Semi) => {
                            return self.skip_and_throw_invalid_seq_err(&["}", "element"], Semi);
                        }
                        Some(_) => {}
                        None => {
                            return self.unexpected_none();
                        }
                    }
                }
                Some(Dedent) => {
                    self.skip();
                    let r_brace = self.expect_or_skip_line(RBrace)?;
                    return Ok(Record::new_mixed(l_brace, r_brace, attrs));
                }
                Some(RBrace) => {
                    let r_brace = self.lpop();
                    return Ok(Record::new_mixed(l_brace, r_brace, attrs));
                }
                Some(_) => {
                    let next = self.try_reduce_expr(ExprCtx::CHUNK).map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "予期: 属性",
                            "simplified_chinese" => "期望: 属性",
                            "traditional_chinese" => "期望: 屬性",
                            "english" => "expect: an attribute",
                        ))
                    })?;
                    match next {
                        Expr::Def(def) => {
                            if attrs.iter().any(|attr| {
                                attr.ident()
                                    .zip(def.sig.ident())
                                    .is_some_and(|(l, r)| l == r)
                            }) {
                                self.warns.push(ParseError::duplicate_elem_warning(
                                    line!() as usize,
                                    def.sig.loc(),
                                    def.sig.to_string(),
                                ));
                            }
                            attrs.push(RecordAttrOrIdent::Attr(def));
                        }
                        Expr::Accessor(acc) => {
                            let ident = match acc {
                                Accessor::Ident(ident) => ident,
                                other => {
                                    return self.fail(ParseError::invalid_record_element_err(
                                        line!() as usize,
                                        other.loc(),
                                    ));
                                }
                            };
                            attrs.push(RecordAttrOrIdent::Ident(ident));
                        }
                        other => {
                            self.next_expr();
                            return self.fail(ParseError::invalid_record_element_err(
                                line!() as usize,
                                other.loc(),
                            ));
                        }
                    }
                }
                None => {
                    return self.unexpected_none();
                }
            }
        }
    }

    fn try_reduce_normal_dict_or_refine_type(
        &mut self,
        l_brace: Token,
        lhs: Expr,
    ) -> ParseResult<BraceContainer> {
        trace!(self);
        let _colon = self.expect_or_skip_line(Colon)?;
        let rhs = self.try_reduce_expr(ExprCtx {
            in_type_args: true,
            ..ExprCtx::EXPR
        })?;
        // dict comprehension: `{k: v | x <- xs}`, `{k: v | x <- xs | guard}`
        if self.cur_is(VBar) && self.nth_is(2, Inclusion) {
            self.skip();
            let (generators, guard) = self.try_reduce_generators()?;
            let r_brace = self.expect_or_skip_line(RBrace)?;
            let comp = DictComprehension::new(
                l_brace,
                r_brace,
                KeyValue::new(lhs, rhs),
                generators,
                guard,
            );
            return Ok(BraceContainer::Dict(Dict::Comprehension(comp)));
        }
        if self.cur_is(VBar) {
            self.skip();
            let Expr::Accessor(Accessor::Ident(var)) = lhs else {
                return self.fail(ParseError::simple_syntax_error(line!() as usize, lhs.loc()));
            };
            let generators = vec![(var, rhs)];
            let guard = self.try_reduce_expr(ExprCtx::CHUNK)?;
            let r_brace = self.expect_or_skip_line(RBrace)?;
            let set_comp = SetComprehension::new(l_brace, r_brace, None, generators, Some(guard));
            Ok(BraceContainer::Set(Set::Comprehension(set_comp)))
        } else {
            let dict = self.try_reduce_normal_dict(l_brace, lhs, rhs)?;
            Ok(BraceContainer::Dict(Dict::Normal(dict)))
        }
    }

    fn try_reduce_normal_dict(
        &mut self,
        l_brace: Token,
        first_key: Expr,
        value: Expr,
    ) -> ParseResult<NormalDict> {
        trace!(self);
        let mut kvs = vec![KeyValue::new(first_key, value)];
        loop {
            match self.peek_kind() {
                Some(Comma) => {
                    self.skip();
                    match self.peek_kind() {
                        Some(Comma) => {
                            return self.skip_and_throw_invalid_seq_err(&["}", "element"], Comma);
                        }
                        Some(RBrace) => {
                            return Ok(NormalDict::new(l_brace, self.lpop(), kvs));
                        }
                        Some(Newline) => {
                            self.skip();
                        }
                        _ => {}
                    }
                    let key = self.try_reduce_expr(ExprCtx {
                        in_brace: true,
                        ..ExprCtx::EXPR
                    })?;
                    self.expect_or_skip_line(Colon)?;
                    let value = self.try_reduce_expr(ExprCtx::CHUNK).map_err(|_| {
                        self.hint(switch_lang!(
                            "japanese" => "予期: キー",
                            "simplified_chinese" => "期望: 关键",
                            "traditional_chinese" => "期望: 關鍵",
                            "english" => "expect: key",
                        ))
                    })?;
                    kvs.push(KeyValue::new(key, value));
                }
                Some(Newline | Indent | Dedent) => {
                    self.skip();
                }
                Some(RBrace) => {
                    return Ok(NormalDict::new(l_brace, self.lpop(), kvs));
                }
                Some(_) => {
                    let loc = self.lpop().loc();
                    return self.fail(ParseError::unclosed_error(
                        line!() as usize,
                        loc,
                        "}",
                        "dict",
                    ));
                }
                None => return self.unexpected_none(),
            }
        }
    }

    fn try_reduce_set(&mut self, l_brace: Token, first_elem: Expr) -> ParseResult<Set> {
        trace!(self);
        if self.cur_is(Semi) {
            match first_elem {
                Expr::Accessor(_) => {}
                other => {
                    return self.fail(ParseError::expect_type_specified(
                        line!() as usize,
                        other.loc(),
                    ));
                }
            }
            self.skip();
            let len = self.try_reduce_expr(ExprCtx::EXPR).map_err(|_| {
                self.hint(switch_lang!(
                    "japanese" => "予期: }か要素",
                    "simplified_chinese" => "期望: }或元素",
                    "traditional_chinese" => "期望: }或元素",
                    "english" => "expect: } or element",
                ))
            })?;
            let r_brace = self.lpop();
            if !r_brace.is(RBrace) {
                return self.skip_and_throw_invalid_unclosed_err("}", "set type specification");
            }
            return Ok(Set::WithLength(SetWithLength::new(
                l_brace,
                r_brace,
                PosArg::new(first_elem),
                len,
            )));
        }
        let mut args = Args::single(PosArg::new(first_elem));
        loop {
            match self.peek_kind() {
                Some(Comma) => {
                    self.skip();
                    match self.peek_kind() {
                        Some(Comma) => {
                            return self.skip_and_throw_invalid_seq_err(&["}", "element"], Comma);
                        }
                        Some(RBrace) => {
                            return Ok(Set::Normal(NormalSet::new(l_brace, self.lpop(), args)));
                        }
                        Some(Newline | Indent | Dedent) => {
                            self.skip();
                        }
                        _ => {}
                    }
                    match self.try_reduce_arg(false)? {
                        ArgKind::Pos(arg) => match arg.expr {
                            Expr::Set(Set::Normal(set)) if set.elems.paren.is_none() => {
                                args.extend_pos(set.elems.into_iters().0);
                            }
                            other => {
                                let pos = PosArg::new(other);
                                if !args.has_pos_arg(&pos) {
                                    args.push_pos(pos);
                                }
                            }
                        },
                        ArgKind::Var(var) | ArgKind::KwVar(var) => {
                            return self.fail(ParseError::simple_syntax_error(
                                line!() as usize,
                                var.loc(),
                            ));
                        }
                        ArgKind::Kw(arg) => {
                            return self.fail(ParseError::simple_syntax_error(
                                line!() as usize,
                                arg.loc(),
                            ));
                        }
                    }
                }
                Some(Newline | Indent | Dedent) => {
                    self.skip();
                }
                Some(RBrace) => {
                    return Ok(Set::Normal(NormalSet::new(l_brace, self.lpop(), args)));
                }
                Some(other) => {
                    return self.skip_and_throw_invalid_seq_err(&["}", "element"], other);
                }
                None => {
                    return self.unexpected_none();
                }
            }
        }
    }

    fn try_reduce_nonempty_tuple(
        &mut self,
        first_elem: ArgKind,
        line_break: bool,
    ) -> ParseResult<Tuple> {
        trace!(self);
        let mut args = Args::from(first_elem);
        #[allow(clippy::while_let_loop)]
        loop {
            match self.peek_kind() {
                Some(Comma) => {
                    self.skip();
                    while self.cur_is(Newline) && line_break {
                        self.skip();
                    }
                    if self.cur_is(Comma) {
                        return self.skip_and_throw_invalid_seq_err(&[")", "element"], Comma);
                    } else if self.cur_is(Dedent) || self.cur_is(RParen) {
                        break;
                    }
                    match self.try_reduce_arg(false)? {
                        ArgKind::Pos(arg) if args.kw_is_empty() && args.var_args.is_none() => {
                            match arg.expr {
                                Expr::Tuple(Tuple::Normal(tup)) if tup.elems.paren.is_none() => {
                                    args.extend_pos(tup.elems.into_iters().0);
                                }
                                other => {
                                    args.push_pos(PosArg::new(other));
                                }
                            }
                        }
                        ArgKind::Var(var) => {
                            args.set_var_args(var);
                        }
                        ArgKind::Pos(arg) => {
                            self.next_expr();
                            return self.fail(ParseError::syntax_error(
                                line!() as usize,
                                arg.loc(),
                                switch_lang!(
                                    "japanese" => "非デフォルト引数はデフォルト引数の後に指定できません",
                                    "simplified_chinese" => "不能在默认参数之后指定非默认参数",
                                    "traditional_chinese" => "不能在默認參數之後指定非默認參數",
                                    "english" => "Non-default arguments cannot be specified after default arguments",
                                ),
                                None,
                            ));
                        }
                        // e.g. (x, y:=1) -> ...
                        // Syntax error will occur when trying to use it as a tuple
                        ArgKind::Kw(arg) => {
                            args.push_kw(arg);
                        }
                        ArgKind::KwVar(arg) => {
                            args.set_kw_var(arg);
                        }
                    }
                }
                Some(_other) => {
                    break;
                }
                None => {
                    return self.unexpected_none();
                }
            }
        }
        Ok(Tuple::Normal(NormalTuple::new(args)))
    }

    #[inline]
    fn try_reduce_lit(&mut self) -> ParseResult<Literal> {
        trace!(self);
        match self.peek() {
            Some(t) if t.category_is(TC::Literal) => Ok(Literal::from(self.lpop())),
            Some(other) => self.fail(ParseError::unexpected_token_error(
                line!() as usize,
                other.loc(),
                &other.inspect()[..],
            )),
            None => self.unexpected_none(),
        }
    }

    /// "...\{, expr, }..." ==> "..." + str(expr) + "..."
    /// "...\{, expr, }..." ==> "..." + str(expr) + "..."
    fn try_reduce_string_interpolation(&mut self) -> ParseResult<Expr> {
        trace!(self);
        let mut expr = Self::str_piece(self.lpop());
        loop {
            match self.peek() {
                Some(l) if l.is(StrInterpRight) => {
                    let right = Self::str_piece(self.lpop());
                    return Ok(Self::concat(expr, right));
                }
                Some(t) if t.is(EOF) => {
                    return self.fail(ParseError::syntax_error(
                        line!() as usize,
                        expr.loc(),
                        switch_lang!(
                            "japanese" => "文字列補間の終わりが見つかりませんでした",
                            "simplified_chinese" => "未找到字符串的结束插值",
                            "traditional_chinese" => "未找到字符串的結束插值",
                            "english" => "end of a string interpolation not found",
                        ),
                        None,
                    ));
                }
                Some(_) => {
                    let mid_expr = self.try_reduce_expr(ExprCtx {
                        winding: true,
                        ..ExprCtx::EXPR
                    })?;
                    let str_func = Expr::local(
                        "str",
                        mid_expr.ln_begin().unwrap_or(0),
                        mid_expr.col_begin().unwrap_or(0),
                        mid_expr.col_end().unwrap_or(0),
                    );
                    let call = Call::new(str_func, None, Args::single(PosArg::new(mid_expr)));
                    expr = Self::concat(expr, Expr::Call(call));
                    if self.cur_is(StrInterpMid) {
                        let mid = Self::str_piece(self.lpop());
                        expr = Self::concat(expr, mid);
                    }
                }
                None => {
                    return self.unexpected_none();
                }
            }
        }
    }

    /// A literal piece of an interpolated string. Exactly one `\{` / `}` marker is
    /// stripped at each end -- a literal brace may sit next to it, as in `"\{x}}"` --
    /// and the quotes it stood in for are put back.
    fn str_piece(mut tok: Token) -> Expr {
        let content = &tok.content[..];
        let quoted = match tok.kind {
            // `"abc\{`
            StrInterpLeft => format!("{}\"", content.strip_suffix("\\{").unwrap_or(content)),
            // `}abc\{`
            StrInterpMid => {
                let content = content.strip_prefix('}').unwrap_or(content);
                format!("\"{}\"", content.strip_suffix("\\{").unwrap_or(content))
            }
            // `}abc"`
            _ => format!("\"{}", content.strip_prefix('}').unwrap_or(content)),
        };
        tok.content = Str::from(quoted);
        tok.kind = StrLit;
        Expr::Literal(Literal::from(tok))
    }

    /// `lhs + rhs`, the `+` placed at `rhs`
    fn concat(lhs: Expr, rhs: Expr) -> Expr {
        let op = Token::new_fake(
            Plus,
            "+",
            rhs.ln_begin().unwrap_or(0),
            rhs.col_begin().unwrap_or(0),
            rhs.col_end().unwrap_or(0),
        );
        Expr::BinOp(BinOp::new(op, lhs, rhs))
    }

    /// x |> f() => f(x)
    fn try_reduce_stream_operator(&mut self, first_arg: Expr) -> ParseResult<Expr> {
        trace!(self);
        let _op = self.lpop();
        if matches!(self.peek_kind(), Some(Dot)) {
            // obj |> .method(...)
            let vis = self.lpop();
            match self.lpop() {
                symbol if symbol.is(Symbol) => {
                    if let Some(args) = self.opt_reduce_args(false).transpose()? {
                        let ident = Identifier::new(
                            VisModifierSpec::Public(vis.loc()),
                            VarName::new(symbol),
                        );
                        let mut call = Expr::Call(Call::new(first_arg, Some(ident), args));
                        while let Some(res) = self.opt_reduce_args(false) {
                            let args = res?;
                            call = call.call_expr(args);
                        }
                        Ok(call)
                    } else {
                        self.skip_and_throw_stream_op_err(first_arg.loc())
                    }
                }
                other => self.fail(ParseError::expect_method_error(
                    line!() as usize,
                    other.loc(),
                )),
            }
        } else {
            let expect_call = self.try_reduce_call_or_acc(false)?;
            let Expr::Call(mut call) = expect_call else {
                return self.skip_and_throw_stream_op_err(expect_call.loc());
            };
            call.args.insert_pos(0, PosArg::new(first_arg));
            Ok(Expr::Call(call))
        }
    }
}
