//! Checks that formatting did not change what the source means.
//!
//! Erg's lexer decides operator fixity from surrounding whitespace -- `x + 1`
//! is an addition, `x +1` applies `x` to `+1` -- so a formatter that adjusts
//! spacing can silently change a program. Rather than trusting the spacing
//! rules to be exhaustive, every result is lexed again and compared against the
//! input. A mismatch means the formatter has a bug, and the caller throws the
//! formatted text away and returns the original.

use erg_common::traits::DequeStream;
use erg_parser::token::{Token, TokenKind, TokenStream};

/// The first place two token streams diverge.
#[derive(Debug, Clone)]
pub struct Mismatch {
    /// Position in the compared (i.e. already filtered) sequence.
    pub index: usize,
    pub before: Option<Token>,
    pub after: Option<Token>,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let show = |tok: &Option<Token>| match tok {
            Some(tok) => format!("{:?} {:?}", tok.kind, tok.content.to_string()),
            None => "<end of stream>".to_string(),
        };
        write!(
            f,
            "token {} differs: before = {}, after = {}",
            self.index,
            show(&self.before),
            show(&self.after)
        )
    }
}

/// Whether a token takes part in the comparison.
///
/// `Newline` is excluded because collapsing runs of blank lines is one of the
/// things the formatter is *for*: the number of these legitimately changes.
fn is_compared(tok: &Token) -> bool {
    tok.kind != TokenKind::Newline
}

/// The part of a token that formatting must not alter.
///
/// `Indent` carries the actual spaces, which change whenever the indent width
/// does, so only its kind is compared -- enough to catch a lost or spurious
/// block, which is the failure that matters. Everything else, comments
/// included, must survive byte for byte.
///
/// The text compared is [`Token::raw_text`], not `content`. `content` has had
/// its escapes decoded, so `"a\tb"` and `"a    b"` share one -- comparing it
/// would let the formatter silently rewrite the first as the second, which is
/// precisely the mistake `raw` exists to prevent.
fn key(tok: &Token) -> (TokenKind, &str) {
    match tok.kind {
        TokenKind::Indent | TokenKind::Dedent => (tok.kind, ""),
        _ => (tok.kind, tok.raw_text()),
    }
}

/// Compares the token stream of the input against that of the output.
///
/// `Ok(())` means formatting preserved the program.
pub fn compare(before: &TokenStream, after: &TokenStream) -> Result<(), Mismatch> {
    let mut befores = before.iter().filter(|tok| is_compared(tok));
    let mut afters = after.iter().filter(|tok| is_compared(tok));
    let mut index = 0;
    loop {
        match (befores.next(), afters.next()) {
            (None, None) => return Ok(()),
            (b, a) if b.map(key) != a.map(key) => {
                return Err(Mismatch {
                    index,
                    before: b.cloned(),
                    after: a.cloned(),
                })
            }
            _ => index += 1,
        }
    }
}

/// [`compare`], as a predicate.
pub fn token_streams_equivalent(before: &TokenStream, after: &TokenStream) -> bool {
    compare(before, after).is_ok()
}
