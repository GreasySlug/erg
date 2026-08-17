//! Checks that formatting did not change what the source means.
//!
//! Erg's lexer decides operator fixity from surrounding whitespace -- `x + 1`
//! is an addition, `x +1` applies `x` to `+1` -- so a formatter that adjusts
//! spacing can silently change a program. Rather than trusting the spacing
//! rules to be exhaustive, every result is lexed again and compared against the
//! input. A mismatch means the formatter has a bug, and the caller throws the
//! formatted text away and returns the original.
//!
//! # What this cannot catch
//!
//! Only differences that reach the token stream. Whitespace that changes how a
//! call is read without changing any token is invisible here: `f(1)` and
//! `f (1)` tokenize identically, yet in Erg they need not mean the same thing.
//! Cases like that are the spacing rules' responsibility -- the formatter
//! preserves the original spacing before an opening bracket rather than relying
//! on this check to catch it (design §5.3).

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
        // the line is what makes this actionable; `index` counts filtered
        // tokens and corresponds to nothing the reader can see
        let show = |tok: &Option<Token>| match tok {
            Some(tok) => format!("{:?} {:?} (line {})", tok.kind, tok.raw_text(), tok.lineno),
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

/// The sequence the comparison actually runs over.
///
/// Runs of `Newline` collapse to one, and a leading run drops entirely. That
/// is what the formatter is allowed to do to blank lines (design §5.4): change
/// how many there are, and remove them from the top of the file.
///
/// What it must *not* do is change whether there is a line break at all.
/// Excluding `Newline` outright -- the first thing this did -- let
/// `x = 1` / `y = 2` be joined into `x = 1 y = 2` without the comparison
/// noticing, because the remaining tokens are identical.
fn comparable(ts: &TokenStream) -> Vec<&Token> {
    let mut out: Vec<&Token> = Vec::new();
    for tok in ts.iter() {
        let redundant_newline = tok.kind == TokenKind::Newline
            && (out.is_empty()
                || out
                    .last()
                    .is_some_and(|prev| prev.kind == TokenKind::Newline));
        if !redundant_newline {
            out.push(tok);
        }
    }
    // The file's own final newline is the formatter's to add or remove -- a
    // source not ending in one is given one. Only the last newline before the
    // end qualifies; any other still separates two statements.
    let tail = out.len() - usize::from(out.last().is_some_and(|t| t.kind == TokenKind::EOF));
    if tail > 0 && out[tail - 1].kind == TokenKind::Newline {
        out.remove(tail - 1);
    }
    out
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
    let before = comparable(before);
    let after = comparable(after);
    for index in 0..before.len().max(after.len()) {
        let b = before.get(index).copied();
        let a = after.get(index).copied();
        if b.map(key) != a.map(key) {
            return Err(Mismatch {
                index,
                before: b.cloned(),
                after: a.cloned(),
            });
        }
    }
    Ok(())
}

/// [`compare`], as a predicate.
pub fn token_streams_equivalent(before: &TokenStream, after: &TokenStream) -> bool {
    compare(before, after).is_ok()
}
