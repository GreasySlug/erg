//! Locating each token in the source text.
//!
//! The formatter has to know how much whitespace separated two tokens, because
//! there are adjacencies it must not invent or remove: `1.0f64` is a suffixed
//! literal only while the suffix touches the literal, and `T|Int|` is a type
//! application only while the bar touches the name. Both lex as two ordinary
//! tokens, so [`crate::verify`] cannot tell the spaced and unspaced forms apart
//! -- the gap has to be measured and preserved instead.
//!
//! `Token::col_begin` cannot supply it. The lexer advances columns by the
//! *decoded* length of each token, so one string containing an escape shifts
//! every column after it on the line. What is exact is [`Token::raw_text`]: it
//! is a slice of the source. So the tokens are walked in order against the
//! source, each one matched where it must appear, and the text in between is
//! the gap.

use std::ops::Range;

use erg_common::traits::DequeStream;
use erg_parser::token::{Token, TokenKind, TokenStream};

/// Where each token of a stream sits in the source it was lexed from.
///
/// The source must be the one the stream came from, after newline
/// normalization -- the lexer normalizes before it reads, so `raw_text` holds
/// LF even for a CRLF file (see [`erg_common::normalize_newline`]).
#[derive(Debug, Clone)]
pub struct Spans {
    spans: Vec<Option<Range<usize>>>,
    /// Byte offset at which each source line begins.
    line_starts: Vec<usize>,
}

/// Tokens the lexer synthesizes rather than reads.
///
/// `Newline` and `Indent` do correspond to source text, but that text is
/// whitespace, which is exactly what the scan skips over; giving them spans
/// would mean tracking them separately for no gain, since the formatter emits
/// its own line breaks and indentation. `Dedent`, `BOF` and `EOF` have no text
/// at all.
fn is_layout(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Newline
            | TokenKind::Indent
            | TokenKind::Dedent
            | TokenKind::BOF
            | TokenKind::EOF
    )
}

impl Spans {
    pub fn new(src: &str, tokens: &TokenStream) -> Self {
        let mut spans = Vec::with_capacity(tokens.len());
        let mut cursor = 0usize;
        for tok in tokens.iter() {
            if is_layout(tok.kind) {
                spans.push(None);
                continue;
            }
            let raw = tok.raw_text();
            let from = src[cursor..]
                .find(|c: char| !c.is_whitespace())
                .map_or(src.len(), |off| cursor + off);
            // Every real token's text is a slice of the source, and only
            // whitespace separates one from the next, so it must start exactly
            // here. `find` is the resync path: if the lexer and this scan ever
            // disagree, keep going from wherever the token really is rather
            // than mislabelling every gap after it.
            let at = if src[from..].starts_with(raw) {
                Some(from)
            } else {
                src[from..].find(raw).map(|off| from + off)
            };
            match at {
                Some(at) => {
                    spans.push(Some(at..at + raw.len()));
                    cursor = at + raw.len();
                }
                None => spans.push(None),
            }
        }
        let line_starts = std::iter::once(0)
            .chain(src.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Self { spans, line_starts }
    }

    /// The byte range of the token at `index`, if it was located.
    pub fn get(&self, index: usize) -> Option<Range<usize>> {
        self.spans.get(index).cloned().flatten()
    }

    /// The 1-origin line a byte offset falls on.
    pub fn line_of(&self, offset: usize) -> u32 {
        self.line_starts.partition_point(|&start| start <= offset) as u32
    }

    /// The first and last source line a token occupies, 1-origin.
    ///
    /// Preferred over `Token::lineno`, which is not consistently the token's
    /// start: the lexer stamps it after consuming the token, so the multi-line
    /// halves of an interpolated string (`}\n\{`) report the line they *end*
    /// on. Reading that as a start makes the token look like it begins after
    /// the token that precedes it ends, which would put a line break in the
    /// middle of a string literal.
    pub fn line_span(&self, index: usize) -> Option<(u32, u32)> {
        let span = self.get(index)?;
        // the last byte *of* the token, not the one after it, or a token
        // ending at a line break would be credited with the next line
        let last = span.end.saturating_sub(1).max(span.start);
        Some((self.line_of(span.start), self.line_of(last)))
    }

    /// The source text between two tokens, if both were located.
    ///
    /// `None` also when they are out of order, which a failed resync can cause.
    pub fn gap<'a>(&self, src: &'a str, before: usize, after: usize) -> Option<&'a str> {
        let end = self.get(before)?.end;
        let start = self.get(after)?.start;
        src.get(end..start)
    }

    /// Whether every token was located -- i.e. whether the gaps can be trusted.
    ///
    /// The scan is exact by construction, so this holds for any source the
    /// lexer accepted. It exists to be asserted on.
    pub fn is_complete(&self, tokens: &TokenStream) -> bool {
        tokens
            .iter()
            .enumerate()
            .all(|(i, tok)| is_layout(tok.kind) || self.get(i).is_some())
    }
}

/// The last source line a token occupies, 1-origin.
///
/// `Token` records only where it starts, but a `#[ ]#` comment or a `"""`
/// literal can be several lines tall.
///
/// Counted on `raw_text`, not `content`: a `\n` escape decodes to a real
/// newline in `content` while occupying no line at all in the source, so
/// `content` would stretch `"a\nb"` across two lines that do not exist.
pub fn end_lineno(tok: &Token) -> u32 {
    tok.lineno + tok.raw_text().matches('\n').count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use erg_parser::lex::Lexer;

    fn lex(src: &str) -> TokenStream {
        Lexer::from_str(src.to_string())
            .keep_comments()
            .lex()
            .unwrap_or_else(|(_, errs)| panic!("lexing {src:?} failed:\n{errs}"))
    }

    /// The whole point: every token must be found, or the gaps are guesses.
    #[test]
    fn every_token_is_located() {
        for src in [
            "x = 1\n",
            "_ = f(1, 2)\n",
            "f = (x) ->\n    x + 1\n",
            "# c\nx = 1 # d\n",
            "#[ a\nb ]#\nx = 1\n",
            "s = \"\"\"\na\n\"\"\"\n",
            r#"_ = "a\tb\n\x41""#,
            r#"_ = "pre\{x}post""#,
            "_ = {x: Int | x > 0}\n",
            "f|T|(x: T) = x\n",
            "_ = 1.0f64\n",
            "@Deco\nf x = x\n",
            "C = Class {.x = Int}\nC.\n    .new x = 1\n",
        ] {
            let tokens = lex(src);
            let spans = Spans::new(src, &tokens);
            assert!(spans.is_complete(&tokens), "unlocated token in {src:?}");
        }
    }

    /// A string whose decoded length differs from its source length shifts
    /// every column after it -- the reason columns are unusable here.
    #[test]
    fn a_span_survives_an_escape_that_columns_do_not() {
        let src = "_ = \"a\\tb\" + 1\n";
        let tokens = lex(src);
        let spans = Spans::new(src, &tokens);
        let str_idx = tokens
            .iter()
            .position(|t| t.kind == TokenKind::StrLit)
            .unwrap();
        assert_eq!(&src[spans.get(str_idx).unwrap()], r#""a\tb""#);
        // ...whereas the token's own columns claim it is two characters shorter
        let tok = tokens.get(str_idx).unwrap();
        assert_ne!(
            (tok.col_end - tok.col_begin) as usize,
            tok.raw_text().chars().count()
        );
    }

    #[test]
    fn a_gap_is_the_text_between_two_tokens() {
        let src = "_ = f  (1)\n";
        let tokens = lex(src);
        let spans = Spans::new(src, &tokens);
        // `_`(0) `=`(1) `f`(2) `(`(3) `1`(4) `)`(5)
        assert_eq!(spans.gap(src, 2, 3), Some("  "));
        assert_eq!(spans.gap(src, 3, 4), Some(""));
    }

    #[test]
    fn a_gap_spanning_a_line_break_contains_it() {
        let src = "_ = f(\n    1\n)\n";
        let tokens = lex(src);
        let spans = Spans::new(src, &tokens);
        assert_eq!(spans.gap(src, 3, 4), Some("\n    "));
    }

    #[test]
    fn end_lineno_counts_source_lines_not_decoded_ones() {
        let tokens = lex("_ = \"a\\nb\"\n");
        let lit = tokens.iter().find(|t| t.kind == TokenKind::StrLit).unwrap();
        assert_eq!(end_lineno(lit), 1, "the escape occupies no source line");

        let tokens = lex("s = \"\"\"\na\n\"\"\"\n");
        let lit = tokens.iter().find(|t| t.kind == TokenKind::StrLit).unwrap();
        assert_eq!(end_lineno(lit), 3);
    }
}
