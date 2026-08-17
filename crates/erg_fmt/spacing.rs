//! What goes between two adjacent tokens.
//!
//! Keyed on `TokenKind`, never on `TokenCategory`. The lexer decides an
//! operator's fixity from the whitespace around it and records the answer in
//! the kind -- `x + 1` gives `Plus`, `x +1` gives `PrePlus` -- so a rule per
//! kind reproduces the spacing that produced it. A rule per category would
//! merge the two and rewrite one program into the other.
//!
//! Where the token stream does not determine the answer, the source's own
//! spacing is kept ([`Space::AsWritten`]). That covers every adjacency of two
//! operands, because in Erg juxtaposition is application and the space in
//! `f (x)` versus `f(x)`, or its absence in `1.0f64` and `T|Int|`, is meaning
//! the formatter has no business inventing -- and, tokenizing identically, is
//! meaning [`crate::verify`] could not catch it inventing.

use erg_parser::token::{Token, TokenKind};

/// The separator to emit between two tokens on one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// Join them.
    None,
    /// Exactly one space.
    One,
    /// One space if the source had any, none if it had none.
    AsWritten,
    /// The source's own run of spaces, but never fewer than one.
    ///
    /// For trailing comments. Collapsing them to one space would break up a
    /// column of aligned comments, and since the formatter has no alignment
    /// pass it could not put one back -- so the run is information only the
    /// source has.
    Padding,
}

impl Space {
    /// Resolves against the text that separated the two tokens in the source.
    ///
    /// An absent gap -- which means [`crate::spans`] could not locate one of
    /// the tokens -- is read as a space. That is the common case by a wide
    /// margin, and joining two operands that were apart would fuse them into a
    /// different token.
    pub fn resolve(self, gap: Option<&str>) -> &str {
        match self {
            Self::None => "",
            Self::One => " ",
            Self::AsWritten => {
                if gap.is_some_and(str::is_empty) {
                    ""
                } else {
                    " "
                }
            }
            // anything but plain spaces (a line break, an absent gap) is not
            // alignment and gets the default
            Self::Padding => match gap {
                Some(gap) if !gap.is_empty() && gap.bytes().all(|b| b == b' ') => gap,
                _ => " ",
            },
        }
    }
}

/// A token that can stand on its own as a value or a type.
///
/// Two of these in a row is juxtaposition -- application, a suffix, a type
/// application -- and the spacing between them is the author's.
const fn is_operand(kind: TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        kind,
        Symbol
            | NatLit
            | IntLit
            | BinLit
            | OctLit
            | HexLit
            | RatioLit
            | BoolLit
            | StrLit
            | NoneLit
            | EllipsisLit
            | InfLit
            | DocComment
            | UBar
            | StrInterpRight
            | LParen
            | LSqBr
            | LBrace
            | RParen
            | RSqBr
            | RBrace
    )
}

const fn is_opening(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::LParen | TokenKind::LSqBr | TokenKind::LBrace
    )
}

const fn is_closing(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::RParen | TokenKind::RSqBr | TokenKind::RBrace
    )
}

/// `..`, `..<`, `<..`, `<..<` -- written tight, as `1..10`.
const fn is_range(kind: TokenKind) -> bool {
    use TokenKind::*;
    matches!(kind, Closed | RightOpen | LeftOpen | Open)
}

/// Operators that take their argument on the right with nothing in between.
///
/// The kind already encodes that: `PreStar` exists only because `*args` had no
/// space, and putting one back would re-lex it as multiplication.
const fn is_prefix(kind: TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        kind,
        PrePlus | PreMinus | PreStar | PreDblStar | PreBitNot | Mutate
    )
}

/// The infix operators whose fixity the lexer reads off their surroundings.
///
/// `op_fix_of` reads `+ - * **` as prefix on exactly one of the four spacings
/// -- a space before and none after -- and as infix on the other three. So the
/// one thing that must never happen to an infix `+` is losing the space on its
/// right while keeping the one on its left.
///
/// `Pow` belongs to that set but is not listed: it is spaced [`Space::AsWritten`]
/// on *both* sides, which reproduces whichever of the four spacings the source
/// had, and so cannot flip it either.
const fn is_ambiguous_infix(kind: TokenKind) -> bool {
    use TokenKind::*;
    matches!(kind, Plus | Minus | Star)
}

/// Operators written both ways often enough that neither is "the" convention.
///
/// `x**2` and `x ** 2`; `f x:=1` and `f(x := 1)`. Measured across this
/// repository, both spellings of each are in the hundreds. A formatter that
/// picks a side on a genuinely split convention produces a diff nobody asked
/// for, so these keep whatever they had.
const fn is_unsettled(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Pow | TokenKind::Walrus)
}

/// Whether `kind` can be applied to what follows it.
///
/// Operands, because juxtaposition is application; and `ref`/`ref!`, which take
/// their argument the same way -- `ref!(x)` is a call, and spacing it out to
/// `ref! (x)` changes what it is called on just as `f(x)` versus `f (x)` does.
const fn takes_argument(kind: TokenKind) -> bool {
    is_operand(kind) || matches!(kind, TokenKind::RefOp | TokenKind::RefMutOp)
}

/// The separator to put between `prev` and `next`, both on the same line.
pub fn space_between(prev: &Token, next: &Token) -> Space {
    use TokenKind::*;
    let (p, n) = (prev.kind, next.kind);
    match () {
        // a trailing comment keeps the gap that may be aligning it
        _ if n == Comment => Space::Padding,
        // Nothing may take the space on an infix `+ - * **`'s right while
        // leaving the one on its left: that is the spelling of prefix. This
        // outranks every rule below, all of which would otherwise close a gap.
        _ if is_ambiguous_infix(p) => Space::One,
        _ if is_unsettled(p) || is_unsettled(n) => Space::AsWritten,
        // A brace is a record (`{ .x = Int }`), a set (`{1, 2}`), a dict
        // (`{"a": 1}`) or a refinement (`{x: Int | x > 0}`), and the padded
        // spelling is the convention for the first and not for the rest. The
        // token stream does not say which one this is, so neither do we --
        // unlike `(` and `[`, which hug in every use.
        _ if p == LBrace || n == RBrace => Space::AsWritten,
        // brackets hug their contents
        _ if is_opening(p) || is_closing(n) => Space::None,
        // `f(a, b)`, `x: Int`, `x?`
        _ if matches!(n, Comma | Semi | Colon | Try) => Space::None,
        // A leading `.` is a member of something, and what that something is
        // depends on the space in front: `discard .Name.parse(x)` calls
        // `discard`, `discard.Name.parse(x)` reaches into it. Same tokens both
        // ways, so `verify` cannot referee it -- the source decides.
        _ if matches!(n, Dot | DblColon) => Space::AsWritten,
        _ if matches!(p, Dot | DblColon | AtSign) => Space::None,
        // `+1`, `*args`, `!x` -- see `is_prefix`
        _ if is_prefix(p) => Space::None,
        // `1..10`
        _ if is_range(p) || is_range(n) => Space::None,
        // `"pre\{x}post"`: the interpolation braces belong to the literal
        _ if matches!(p, StrInterpLeft | StrInterpMid) => Space::None,
        _ if matches!(n, StrInterpMid | StrInterpRight) => Space::None,
        // `|` is a refinement separator in `{x: Int | x > 0}` but glues a type
        // application together in `T|Int|`, and `^`/`&` are reserved with no
        // settled convention. The stream does not say which, so neither do we.
        _ if matches!(p, VBar | Caret | Amper) || matches!(n, VBar | Caret | Amper) => {
            Space::AsWritten
        }
        // juxtaposition -- `f x`, `f (x)`, `1.0f64`, `ref!(x)`
        _ if takes_argument(p) && is_operand(n) => Space::AsWritten,
        _ => Space::One,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use erg_common::traits::DequeStream;
    use erg_parser::lex::Lexer;
    use erg_parser::token::TokenStream;

    fn lex(src: &str) -> TokenStream {
        Lexer::from_str(src.to_string())
            .keep_comments()
            .lex()
            .unwrap_or_else(|(_, errs)| panic!("lexing {src:?} failed:\n{errs}"))
    }

    /// Re-renders one source line by applying the rules to every adjacency,
    /// which is what the renderer does.
    fn respace(src: &str) -> String {
        let tokens = lex(src);
        let spans = crate::spans::Spans::new(src, &tokens);
        let real: Vec<(usize, &Token)> = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                !matches!(
                    t.kind,
                    TokenKind::Newline
                        | TokenKind::Indent
                        | TokenKind::Dedent
                        | TokenKind::BOF
                        | TokenKind::EOF
                )
            })
            .collect();
        let mut out = String::new();
        for (nth, (i, tok)) in real.iter().enumerate() {
            if nth > 0 {
                let (prev_i, prev) = real[nth - 1];
                let gap = spans.gap(src, prev_i, *i);
                out.push_str(space_between(prev, tok).resolve(gap));
            }
            out.push_str(tok.raw_text());
        }
        out
    }

    #[test]
    fn infix_operators_get_one_space_on_each_side() {
        assert_eq!(respace("_ = 1+2\n"), "_ = 1 + 2");
        assert_eq!(respace("_ = 1  *  2\n"), "_ = 1 * 2");
        assert_eq!(respace("_ = a and b or not c\n"), "_ = a and b or not c");
        assert_eq!(respace("_ = a<=b\n"), "_ = a <= b");
    }

    /// The rule the whole design turns on: `+1` is an argument, not an addend.
    #[test]
    fn prefix_operators_stay_glued_to_their_argument() {
        assert_eq!(respace("_ = f +1\n"), "_ = f +1");
        assert_eq!(
            respace("f(*args, **kwargs) = None\n"),
            "f(*args, **kwargs) = None"
        );
        assert_eq!(respace("_ = !x\n"), "_ = !x");
    }

    #[test]
    fn brackets_hug_their_contents() {
        assert_eq!(respace("_ = f( 1 , 2 )\n"), "_ = f(1, 2)");
        assert_eq!(respace("_ = [ 1 , 2 ]\n"), "_ = [1, 2]");
        assert_eq!(respace("_ = ( 1 , )\n"), "_ = (1,)");
        assert_eq!(respace("_ = {}\n"), "_ = {}");
    }

    #[test]
    fn separators_take_a_space_after_and_none_before() {
        assert_eq!(respace("_ = [1 ; 3]\n"), "_ = [1; 3]");
        assert_eq!(respace("f x : Int = x\n"), "f x: Int = x");
    }

    #[test]
    fn access_operators_take_no_space() {
        assert_eq!(respace("_ = a.b.c()\n"), "_ = a.b.c()");
        assert_eq!(respace("_ = x ?.y\n"), "_ = x?.y");
        assert_eq!(respace("@ Deco\n"), "@Deco");
    }

    /// `discard .Name.parse(x)` calls `discard`; `discard.Name.parse(x)`
    /// reaches into it. The two tokenize identically, so closing that gap would
    /// be a silent rewrite that the verification could not catch.
    #[test]
    fn the_space_before_a_leading_dot_is_load_bearing() {
        assert_eq!(
            respace("discard .Name.parse(x)\n"),
            "discard .Name.parse(x)"
        );
        assert_eq!(respace("discard.Name.parse(x)\n"), "discard.Name.parse(x)");
    }

    /// The same trap one operator over: `+` followed by a member path needs the
    /// space on its right, or it is read as a prefix `+`.
    #[test]
    fn an_infix_operator_keeps_the_space_before_a_leading_dot() {
        assert_eq!(respace(".f x = x + .i\n"), ".f x = x + .i");
    }

    /// `ref!(x)` is a call. Spacing it out changes what is called, exactly as
    /// it would for `f(x)`.
    #[test]
    fn ref_takes_its_argument_the_way_a_function_does() {
        assert_eq!(respace("_ = ref!(x)\n"), "_ = ref!(x)");
        assert_eq!(respace("_ = ref! self\n"), "_ = ref! self");
    }

    /// Neither spelling is the convention, so neither is imposed.
    #[test]
    fn unsettled_operators_keep_the_spacing_they_had() {
        assert_eq!(respace("_ = x**2\n"), "_ = x**2");
        assert_eq!(respace("_ = x ** 2\n"), "_ = x ** 2");
        assert_eq!(respace("f x:=1\n"), "f x:=1");
        assert_eq!(respace("f(x := 1) = x\n"), "f(x := 1) = x");
    }

    /// A brace is a record, a set, a dict or a refinement, and only the first
    /// is conventionally padded.
    #[test]
    fn brace_padding_is_left_to_the_author() {
        assert_eq!(respace("_ = { .x = 1 }\n"), "_ = { .x = 1 }");
        assert_eq!(respace("_ = {\"a\": 1}\n"), "_ = {\"a\": 1}");
        // ...while `(` and `[` hug in every use
        assert_eq!(respace("_ = ( 1 )\n"), "_ = (1)");
    }

    /// A column of aligned comments is information the formatter cannot
    /// reconstruct, so it does not destroy it.
    #[test]
    fn a_trailing_comment_keeps_the_gap_that_aligns_it() {
        assert_eq!(respace("_ = 1     # c\n"), "_ = 1     # c");
        assert_eq!(respace("_ = 1# c\n"), "_ = 1 # c", "but never none");
    }

    #[test]
    fn definition_operators_take_one_space() {
        assert_eq!(respace("f=(x)->x\n"), "f = (x) -> x");
        assert_eq!(respace("f=(x)=>x\n"), "f = (x) => x");
        assert_eq!(respace("f: (T<:Eq) -> T\n"), "f: (T <: Eq) -> T");
    }

    #[test]
    fn ranges_are_written_tight() {
        assert_eq!(respace("_ = 1 .. 10\n"), "_ = 1..10");
        assert_eq!(respace("_ = 1 ..< 10\n"), "_ = 1..<10");
    }

    /// Juxtaposition is application, so the author's spacing stands. `f (x)`
    /// and `f(x)` tokenize identically, and in Erg they need not agree.
    #[test]
    fn juxtaposition_keeps_the_spacing_it_had() {
        assert_eq!(respace("_ = f(1)\n"), "_ = f(1)");
        assert_eq!(respace("_ = f (1)\n"), "_ = f (1)");
        assert_eq!(respace("_ = f  (1)\n"), "_ = f (1)", "but runs collapse");
        assert_eq!(respace("print! \"hi\"\n"), "print! \"hi\"");
    }

    /// A suffix is a suffix only while it touches the literal, and the two
    /// tokens are the same either way -- so this must not be normalized.
    #[test]
    fn an_adjacent_suffix_is_not_pulled_apart() {
        assert_eq!(respace("_ = 1.0f64\n"), "_ = 1.0f64");
        assert_eq!(respace("f|T|(x: T) = x\n"), "f|T|(x: T) = x");
    }

    /// ...and the same bar, spaced, separates a refinement predicate.
    #[test]
    fn a_refinement_bar_keeps_its_spaces() {
        assert_eq!(respace("_ = {x: Int | x > 0}\n"), "_ = {x: Int | x > 0}");
    }

    #[test]
    fn interpolation_braces_belong_to_the_literal() {
        assert_eq!(respace("_ = \"pre\\{x}post\"\n"), "_ = \"pre\\{x}post\"");
        assert_eq!(
            respace("_ = \"pre\\{ a + b }post\"\n"),
            "_ = \"pre\\{a + b}post\""
        );
    }

    #[test]
    fn a_leading_dot_is_a_public_member_not_an_access() {
        assert_eq!(respace("_ = {.x = 1}\n"), "_ = {.x = 1}");
    }

    #[test]
    fn word_operators_keep_the_spaces_that_keep_them_words() {
        assert_eq!(respace("_ = ref x\n"), "_ = ref x");
        assert_eq!(respace("_ = a as Int\n"), "_ = a as Int");
        assert_eq!(respace("_ = x in [1]\n"), "_ = x in [1]");
    }
}
