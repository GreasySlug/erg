//! defines `Token` (The minimum unit in the Erg source code that serves as input to the parser).
//!
//! Token(パーサーへの入力となる、Ergソースコードにおける最小単位)を定義する
use std::collections::VecDeque;
use std::fmt;
use std::hash::{Hash, Hasher};

use erg_common::error::Location;
use erg_common::impl_displayable_deque_stream_for_wrapper;
use erg_common::opcode311::BinOpCode;
use erg_common::str::Str;
use erg_common::traits::{DequeStream, Locational};
// use erg_common::ty::Type;
// use erg_common::typaram::OpKind;
// use erg_common::value::ValueObj;

#[cfg(not(feature = "pylib"))]
use erg_proc_macros::pyclass;
#[cfg(feature = "pylib")]
use pyo3::prelude::*;

/// 意味論的名前と記号自体の名前が混在しているが、Pythonの名残である
#[pyclass]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TokenKind {
    /// e.g. i, p!, $s, T, `+`, `and`, 'd/dx'
    Symbol,
    /// e.g. 0, 1
    NatLit,
    /// e.g. -1, -2
    IntLit,
    /// e.g. 0b101
    BinLit,
    /// e.g. 0o777
    OctLit,
    /// e.g. 0xdeadbeef
    HexLit,
    RatioLit,
    BoolLit,
    StrLit,
    /// e.g. "abc\{
    StrInterpLeft,
    /// e.g. }abc\{
    StrInterpMid,
    /// e.g. }def"
    StrInterpRight,
    NoneLit,
    /// ... (== Ellipsis)
    EllipsisLit,
    InfLit,
    DocComment,
    /// `+` (unary)
    PrePlus,
    /// `-` (unary)
    PreMinus,
    /// ~ (unary)
    PreBitNot,
    // PreAmp,    // & (unary)
    // PreAt,     // @ (unary)
    /// ! (unary)
    Mutate,
    PreStar,    // * (unary)
    PreDblStar, // ** (unary)
    /// ? (postfix)
    Try,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// /
    Slash,
    /// //
    FloorDiv,
    /// **
    Pow,
    /// %
    Mod,
    /// ..
    Closed,
    /// ..<
    RightOpen,
    /// <..
    LeftOpen,
    /// <..<
    Open,
    /// &&
    BitAnd,
    /// ||
    BitOr,
    /// ^^
    BitXor,
    /// <<
    Shl,
    /// >>
    Shr,
    /// <
    Less,
    /// >
    Gre,
    /// <=
    LessEq,
    /// >=
    GreEq,
    /// ==
    DblEq,
    /// !=
    NotEq,
    /// `in`
    InOp,
    /// `notin`
    NotInOp,
    // `contains`
    ContainsOp,
    /// `sub` (subtype of)
    SubOp,
    /// `is!`
    IsOp,
    /// `isnot!`
    IsNotOp,
    /// `and`
    AndOp,
    /// `or`
    OrOp,
    /// `ref` (special unary)
    RefOp,
    /// `ref!` (special unary)
    RefMutOp,
    /// =
    Assign,
    /// <-
    Inclusion,
    /// :=
    Walrus,
    /// ->
    FuncArrow,
    /// =>
    ProcArrow,
    /// (
    LParen,
    /// )
    RParen,
    /// [
    LSqBr,
    /// ]
    RSqBr,
    /// {
    LBrace,
    /// }
    RBrace,
    Indent,
    Dedent,
    /// .
    Dot,
    /// |>
    Pipe,
    /// :
    Colon,
    /// ::
    DblColon,
    /// :>
    SupertypeOf,
    /// <:
    SubtypeOf,
    /// `as`
    As,
    /// ,
    Comma,
    /// ^
    Caret,
    /// &
    Amper,
    /// @
    AtSign,
    /// |
    VBar,
    /// _
    UBar,
    /// \n
    Newline,
    /// ;
    Semi,
    /// `# ...` or `#[ ... ]#`.
    /// Only produced when the lexer was built with `Lexer::keep_comments`;
    /// otherwise comments are consumed and dropped.
    Comment,
    Illegal,
    /// Beginning Of File
    BOF,
    EOF,
}

use TokenKind::*;

#[pyclass]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenCategory {
    Symbol,
    Literal,
    StrInterpLeft,
    StrInterpMid,
    StrInterpRight,
    BinOp,
    UnaryOp,
    /// ? <.. ..
    PostfixOp,
    /// ( [ { Indent
    LEnclosure,
    /// ) } } Dedent
    REnclosure,
    /// , : :: :> <: . |> :=
    SpecialBinOp,
    /// =
    DefOp,
    /// -> =>
    LambdaOp,
    /// \n ;
    Separator,
    /// `#`-comments. Carries no syntax; present only for tools that must
    /// reproduce the source (see `TokenKind::Comment`).
    Ignorable,
    /// ^ &
    Reserved,
    /// @
    AtSign,
    /// |
    VBar,
    /// _
    UBar,
    BOF,
    EOF,
    Illegal,
}

impl fmt::Display for TokenCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl TokenCategory {
    pub const fn is_block_op(&self) -> bool {
        matches!(self, Self::DefOp | Self::LambdaOp)
    }
}

impl TokenKind {
    pub const fn category(&self) -> TokenCategory {
        match self {
            Symbol => TokenCategory::Symbol,
            NatLit | BinLit | OctLit | HexLit | IntLit | RatioLit | StrLit | BoolLit | NoneLit
            | EllipsisLit | InfLit | DocComment => TokenCategory::Literal,
            StrInterpLeft => TokenCategory::StrInterpLeft,
            StrInterpMid => TokenCategory::StrInterpMid,
            StrInterpRight => TokenCategory::StrInterpRight,
            PrePlus | PreMinus | PreBitNot | Mutate | PreStar | PreDblStar | RefOp | RefMutOp => {
                TokenCategory::UnaryOp
            }
            Try => TokenCategory::PostfixOp,
            Comma | Colon | DblColon | SupertypeOf | SubtypeOf | As | Dot | Pipe | Walrus
            | Inclusion => TokenCategory::SpecialBinOp,
            Assign => TokenCategory::DefOp,
            FuncArrow | ProcArrow => TokenCategory::LambdaOp,
            Semi | Newline => TokenCategory::Separator,
            Comment => TokenCategory::Ignorable,
            LParen | LBrace | LSqBr | Indent => TokenCategory::LEnclosure,
            RParen | RBrace | RSqBr | Dedent => TokenCategory::REnclosure,
            Caret | Amper => TokenCategory::Reserved,
            AtSign => TokenCategory::AtSign,
            VBar => TokenCategory::VBar,
            UBar => TokenCategory::UBar,
            BOF => TokenCategory::BOF,
            EOF => TokenCategory::EOF,
            Illegal => TokenCategory::Illegal,
            _ => TokenCategory::BinOp,
        }
    }

    pub const fn precedence(&self) -> Option<usize> {
        let prec = match self {
            Dot | DblColon => 200,                                    // .
            Pow => 190,                                               // **
            PrePlus | PreMinus | PreBitNot | RefOp | RefMutOp => 180, // (unary) + - * ~ ref ref!
            Star | Slash | FloorDiv | Mod => 170,                     // * / // %
            Plus | Minus => 160,                                      // + -
            Shl | Shr => 150,                                         // << >>
            BitAnd => 140,                                            // &&
            BitXor => 130,                                            // ^^
            BitOr => 120,                                             // ||
            Closed | LeftOpen | RightOpen | Open => 100,              // range operators
            Less | Gre | LessEq | GreEq | DblEq | NotEq | InOp | NotInOp | ContainsOp | IsOp
            | IsNotOp => 90, // < > <= >= == != in notin contains is isnot
            AndOp => 80,                                              // and
            OrOp => 70,                                               // or
            FuncArrow | ProcArrow | Inclusion => 60,                  // -> => <-
            Colon | SupertypeOf | SubtypeOf | As => 50,               // : :> <: as
            Comma => 40,                                              // ,
            Assign | Walrus => 20,                                    // = :=
            Newline | Semi => 10,                                     // \n ;
            LParen | LBrace | LSqBr | Indent => 0,                    // ( { [ Indent
            _ => return None,
        };
        Some(prec)
    }

    pub const fn is_right_associative(&self) -> bool {
        matches!(
            self,
            FuncArrow | ProcArrow | Assign /* | PreDollar | PreAt */
        )
    }

    pub const fn is_range_op(&self) -> bool {
        matches!(self, Closed | LeftOpen | RightOpen | Open)
    }
}

impl fmt::Display for TokenKind {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl From<TokenKind> for BinOpCode {
    fn from(tk: TokenKind) -> Self {
        match tk {
            Plus => BinOpCode::Add,
            Minus => BinOpCode::Subtract,
            Star => BinOpCode::Multiply,
            Slash => BinOpCode::TrueDivide,
            FloorDiv => BinOpCode::FloorDiv,
            Mod => BinOpCode::Remainder,
            Pow => BinOpCode::Power,
            BitAnd => BinOpCode::And,
            BitOr => BinOpCode::Or,
            BitXor => BinOpCode::Xor,
            Shl => BinOpCode::LShift,
            Shr => BinOpCode::RShift,
            _ => panic!("invalid token kind for binop"),
        }
    }
}

#[pyclass(get_all, set_all)]
#[derive(Clone, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub content: Str,
    /// The original source text of this token, set only when it differs from
    /// `content`. `None` (the usual case) means `content` is already verbatim.
    ///
    /// String literals are the reason this exists: the lexer decodes escape
    /// sequences into `content` (`"a\nb"` holds a real newline, `"a\tb"` holds
    /// four spaces), so `content` cannot be used to reproduce the source.
    /// Consumers that must round-trip the source (e.g. a formatter) should read
    /// [`Token::raw_text`] instead of `content`.
    ///
    /// Ignored by `PartialEq`/`Hash`/[`Token::deep_eq`], like `lineno`/`col_*`.
    ///
    /// Boxed because it is `None` for all but string literals: an inline
    /// `Option<Str>` costs 24 bytes on every token (`Token` 40 -> 64,
    /// `Expr` 368 -> 416), while the pointer costs 8 and moves the payload
    /// off to the rare token that actually needs it.
    pub raw: Option<Box<Str>>,
    /// 1 origin
    // TODO: 複数行文字列リテラルもあるのでタプルにするのが妥当?
    pub lineno: u32,
    /// A pointer from which the token starts (0 origin)
    pub col_begin: u32,
    /// A pointer to the end position of the token.
    /// `col_end - col_start` does not necessarily equal `content.len()`
    pub col_end: u32,
}

pub const COLON: Token = Token::dummy(TokenKind::Colon, ":");
pub const AS: Token = Token::dummy(TokenKind::As, "as");
pub const DOT: Token = Token::dummy(TokenKind::Dot, ".");
pub const EQUAL: Token = Token::dummy(TokenKind::Assign, "=");

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Token");
        s.field("kind", &self.kind)
            .field("content", &self.content.replace('\n', "\\n"));
        // only shown when set, so the output is unchanged for ordinary tokens
        if let Some(raw) = &self.raw {
            s.field("raw", &raw.replace('\n', "\\n"));
        }
        s.field("lineno", &self.lineno)
            .field("col_begin", &self.col_begin)
            .field("col_end", &self.col_end)
            .finish()
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {}", self.kind, self.content.replace('\n', "\\n"))
    }
}

// the values of lineno and col are not relevant for comparison
// use `deep_eq` if you want to compare them
impl PartialEq for Token {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.is(other.kind) && self.content == other.content
    }
}

impl Hash for Token {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.content.hash(state);
    }
}

impl Locational for Token {
    fn loc(&self) -> Location {
        if self.lineno == 0 {
            Location::Unknown
        } else {
            Location::range(self.lineno, self.col_begin, self.lineno, self.col_end)
        }
    }

    /// The stored `col_end` counts chars, like `col_begin` and `loc()`.
    /// `content.len()` would count bytes and put the end of any non-ASCII
    /// token past where it is.
    #[inline]
    fn col_end(&self) -> Option<u32> {
        Some(self.col_end)
    }
}

impl Token {
    pub const DUMMY: Token = Token {
        kind: TokenKind::Illegal,
        content: Str::ever("DUMMY"),
        raw: None,
        lineno: 0,
        col_begin: 0,
        col_end: 0,
    };

    pub const fn dummy(kind: TokenKind, content: &'static str) -> Self {
        Self {
            kind,
            content: Str::ever(content),
            raw: None,
            lineno: 0,
            col_begin: 0,
            col_end: 0,
        }
    }

    #[inline]
    pub fn new<S: Into<Str>>(kind: TokenKind, cont: S, lineno: u32, col_begin: u32) -> Self {
        let content = cont.into();
        let col_end = col_begin + content.chars().count() as u32;
        Token {
            kind,
            content,
            raw: None,
            lineno,
            col_begin,
            col_end,
        }
    }

    #[inline]
    pub fn new_fake<S: Into<Str>>(
        kind: TokenKind,
        cont: S,
        lineno: u32,
        col_begin: u32,
        col_end: u32,
    ) -> Self {
        Token {
            kind,
            content: cont.into(),
            raw: None,
            lineno,
            col_begin,
            col_end,
        }
    }

    pub fn new_with_loc(kind: TokenKind, cont: impl Into<Str>, loc: Location) -> Self {
        Token {
            kind,
            content: cont.into(),
            raw: None,
            lineno: loc.ln_begin().unwrap_or(0),
            col_begin: loc.col_begin().unwrap_or(0),
            col_end: loc.col_end().unwrap_or(1),
        }
    }

    #[inline]
    pub fn from_str(kind: TokenKind, cont: &str) -> Self {
        Token {
            kind,
            content: Str::rc(cont),
            raw: None,
            lineno: 0,
            col_begin: 0,
            col_end: 0,
        }
    }

    #[inline]
    pub fn symbol(cont: &str) -> Self {
        Self::from_str(TokenKind::Symbol, cont)
    }

    #[inline]
    pub fn symbol_with_line(cont: &str, lineno: u32) -> Self {
        Token {
            kind: TokenKind::Symbol,
            content: Str::rc(cont),
            raw: None,
            lineno,
            col_begin: 0,
            col_end: 1,
        }
    }

    pub fn symbol_with_loc<S: Into<Str>>(cont: S, loc: Location) -> Self {
        Token {
            kind: TokenKind::Symbol,
            content: cont.into(),
            raw: None,
            lineno: loc.ln_begin().unwrap_or(0),
            col_begin: loc.col_begin().unwrap_or(0),
            col_end: loc.col_end().unwrap_or(1),
        }
    }

    pub const fn static_symbol(s: &'static str) -> Self {
        Token {
            kind: TokenKind::Symbol,
            content: Str::ever(s),
            raw: None,
            lineno: 0,
            col_begin: 0,
            col_end: 1,
        }
    }

    pub fn deep_eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.content == other.content
            && self.lineno == other.lineno
            && self.col_begin == other.col_begin
    }

    pub fn loc(&self) -> Location {
        Locational::loc(self)
    }

    pub const fn category(&self) -> TokenCategory {
        self.kind.category()
    }

    pub fn category_is(&self, category: TokenCategory) -> bool {
        self.kind.category() == category
    }

    pub fn is(&self, kind: TokenKind) -> bool {
        self.kind == kind
    }

    pub fn is_number(&self) -> bool {
        matches!(
            self.kind,
            NatLit | IntLit | BinLit | OctLit | HexLit | RatioLit | InfLit
        )
    }

    pub fn is_str(&self) -> bool {
        matches!(self.kind, StrLit | DocComment)
    }

    pub const fn is_block_op(&self) -> bool {
        self.category().is_block_op()
    }

    pub const fn inspect(&self) -> &Str {
        &self.content
    }

    /// The verbatim source text of this token.
    ///
    /// Equal to `content` except for tokens whose `content` is a decoded form
    /// of the source (string literals; see [`Token::raw`]). Use this, not
    /// `content`, whenever the output has to reproduce the input byte for byte.
    pub fn raw_text(&self) -> &str {
        match &self.raw {
            Some(raw) => raw,
            None => &self.content,
        }
    }

    /// Records the verbatim source text, for tokens whose `content` is decoded.
    ///
    /// Storing a `raw` equal to `content` is pointless, so that case is
    /// normalized back to `None`: it keeps `Debug` quiet and, more importantly,
    /// avoids an allocation for every string literal that has no escapes.
    pub fn with_raw<S: Into<Str>>(mut self, raw: S) -> Self {
        let raw = raw.into();
        self.raw = (raw != self.content).then(|| Box::new(raw));
        self
    }

    pub fn is_procedural(&self) -> bool {
        self.inspect().ends_with('!')
    }

    pub fn is_const(&self) -> bool {
        self.inspect().is_uppercase()
    }
}

#[pyclass]
#[derive(Debug, Clone)]
pub struct TokenStream(VecDeque<Token>);

impl_displayable_deque_stream_for_wrapper!(TokenStream, Token);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_text_falls_back_to_content() {
        let tok = Token::new(TokenKind::Symbol, "x", 1, 0);
        assert_eq!(tok.raw, None);
        assert_eq!(tok.raw_text(), "x");
    }

    #[test]
    fn with_raw_keeps_the_source_text() {
        // what the lexer will do for `"a\nb"`: content holds the decoded form,
        // raw holds the two characters actually written in the source
        let tok = Token::new(TokenKind::StrLit, "\"a\nb\"", 1, 0).with_raw("\"a\\nb\"");
        assert_eq!(tok.content, "\"a\nb\"");
        assert_eq!(tok.raw_text(), "\"a\\nb\"");
    }

    #[test]
    fn with_raw_is_none_when_it_matches_content() {
        let tok = Token::new(TokenKind::StrLit, "\"ab\"", 1, 0).with_raw("\"ab\"");
        assert_eq!(tok.raw, None);
        assert_eq!(tok.raw_text(), "\"ab\"");
    }

    /// `raw` must stay out of equality and hashing, like `lineno`/`col_*`.
    #[test]
    fn raw_does_not_affect_eq() {
        let plain = Token::new(TokenKind::StrLit, "\"a\nb\"", 1, 0);
        let with_raw = plain.clone().with_raw("\"a\\nb\"");
        assert!(with_raw.raw.is_some());
        assert_eq!(plain, with_raw);
    }
}
