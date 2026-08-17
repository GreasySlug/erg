//! Splitting the token stream into the units that get laid out.
//!
//! A *logical line* is a run of tokens ending at a `Newline`. It is not always
//! one source line: inside brackets the lexer swallows newlines, so a call
//! broken over four lines arrives here as a single run.
//!
//! Comments need no special case. One that trails code lands in the same item
//! as that code, because the item is still open when it is reached; one on a
//! line of its own starts an item, because nothing is.

use std::ops::{Range, RangeInclusive};

use erg_common::traits::DequeStream;
use erg_parser::token::{Token, TokenKind, TokenStream};

use crate::spans::{end_lineno, Spans};

/// The first and last source line a token occupies.
///
/// From [`Spans`], which measures the source directly. `Token::lineno` is only
/// the fallback, and only an approximation: it is the token's start line for
/// most kinds but its end line for the multi-line halves of an interpolated
/// string, and there is no way to tell which from the token alone.
fn lines_of(spans: &Spans, index: usize, tok: &Token) -> (u32, u32) {
    spans
        .line_span(index)
        .unwrap_or((tok.lineno, end_lineno(tok)))
}

/// One unit of output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Nesting depth, counted from `Indent`/`Dedent`.
    pub depth: usize,
    /// The 1-origin source lines this item occupies.
    pub lines: RangeInclusive<u32>,
    /// How many blank lines preceded it in the source.
    pub blank_lines_before: usize,
    /// The item's tokens, as a half-open range of indices into the stream it
    /// was split from.
    ///
    /// Indices rather than clones so the renderer can ask [`crate::spans`] what
    /// separated two of them, which is keyed by position in the stream.
    pub tokens: Range<usize>,
}

impl Item {
    /// Whether the item covers more than one source line.
    ///
    /// Either because the author broke it inside brackets, where the lexer
    /// swallows the newline, or because one of its tokens is itself several
    /// lines tall -- a `"""` literal, a `#[ ]#` comment.
    pub fn is_multiline(&self) -> bool {
        self.lines.end() > self.lines.start()
    }
}

/// Groups a token stream into items, in source order.
///
/// Expects a stream lexed with `keep_comments`; without it the comments are
/// simply absent and the result is still well-formed.
pub fn split(tokens: &TokenStream, spans: &Spans) -> Vec<Item> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut blanks = 0usize;
    // the item being built
    let mut open: Option<Item> = None;

    let close = |open: &mut Option<Item>, items: &mut Vec<Item>| match open.take() {
        Some(item) => {
            items.push(item);
            true
        }
        None => false,
    };

    for (index, tok) in tokens.iter().enumerate() {
        match tok.kind {
            TokenKind::Indent => depth += 1,
            TokenKind::Dedent => depth = depth.saturating_sub(1),
            // the first newline ends the item; any after it are blank lines
            TokenKind::Newline => {
                if !close(&mut open, &mut items) {
                    blanks += 1;
                }
            }
            TokenKind::EOF | TokenKind::BOF => {}
            _ => {
                let (first, last) = lines_of(spans, index, tok);
                match &mut open {
                    // a token can end on a later line than the item began on,
                    // both because newlines inside brackets never reach us and
                    // because a single token can itself be several lines tall
                    Some(item) => {
                        item.lines = *item.lines.start()..=(*item.lines.end()).max(last);
                        item.tokens.end = index + 1;
                    }
                    None => {
                        open = Some(Item {
                            depth,
                            lines: first..=last,
                            blank_lines_before: blanks,
                            tokens: index..index + 1,
                        });
                        blanks = 0;
                    }
                }
            }
        }
    }
    // a file whose last line has no newline still ends with an item
    close(&mut open, &mut items);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use erg_parser::lex::Lexer;

    fn split_src(src: &str) -> Vec<Item> {
        let tokens = Lexer::from_str(src.to_string())
            .keep_comments()
            .lex()
            .unwrap_or_else(|(_, errs)| panic!("lexing failed:\n{errs}"));
        split(&tokens, &Spans::new(src, &tokens))
    }

    /// `(depth, first line, last line, blank lines before)`
    fn shape(src: &str) -> Vec<(usize, u32, u32, usize)> {
        split_src(src)
            .into_iter()
            .map(|i| {
                (
                    i.depth,
                    *i.lines.start(),
                    *i.lines.end(),
                    i.blank_lines_before,
                )
            })
            .collect()
    }

    #[test]
    fn one_statement_per_line() {
        assert_eq!(shape("x = 1\ny = 2\n"), vec![(0, 1, 1, 0), (0, 2, 2, 0)]);
    }

    #[test]
    fn blank_lines_are_counted_against_the_following_item() {
        assert_eq!(
            shape("x = 1\n\n\ny = 2\n"),
            vec![(0, 1, 1, 0), (0, 4, 4, 2)]
        );
    }

    #[test]
    fn leading_blank_lines_are_counted_too() {
        assert_eq!(shape("\n\nx = 1\n"), vec![(0, 3, 3, 2)]);
    }

    #[test]
    fn indentation_becomes_depth() {
        assert_eq!(
            shape("f = (x) ->\n    x\ny = 2\n"),
            vec![(0, 1, 1, 0), (1, 2, 2, 0), (0, 3, 3, 0)]
        );
    }

    #[test]
    fn nested_blocks_nest_depth() {
        assert_eq!(
            shape("f = (x) ->\n    g = (y) ->\n        y\n    g\n"),
            vec![(0, 1, 1, 0), (1, 2, 2, 0), (2, 3, 3, 0), (1, 4, 4, 0)]
        );
    }

    /// Newlines inside brackets never reach the splitter, so the whole call is
    /// one item spanning four source lines.
    #[test]
    fn a_bracketed_run_is_one_item() {
        assert_eq!(shape("_ = f(\n    1,\n    2\n)\n"), vec![(0, 1, 4, 0)]);
        assert!(split_src("_ = f(\n    1,\n    2\n)\n")[0].is_multiline());
    }

    #[test]
    fn a_trailing_comment_joins_the_line_it_follows() {
        assert_eq!(
            shape("x = 1 # c\ny = 2\n"),
            vec![(0, 1, 1, 0), (0, 2, 2, 0)]
        );
    }

    #[test]
    fn a_comment_on_its_own_line_is_its_own_item() {
        assert_eq!(shape("# c\nx = 1\n"), vec![(0, 1, 1, 0), (0, 2, 2, 0)]);
    }

    #[test]
    fn a_comment_inside_a_block_takes_the_blocks_depth() {
        assert_eq!(
            shape("f = (x) ->\n    # c\n    x\n"),
            vec![(0, 1, 1, 0), (1, 2, 2, 0), (1, 3, 3, 0)]
        );
    }

    #[test]
    fn a_multi_line_comment_is_one_item() {
        assert_eq!(
            shape("#[ a\nb ]#\nx = 1\n"),
            vec![(0, 1, 2, 0), (0, 3, 3, 0)]
        );
    }

    #[test]
    fn a_multi_line_string_keeps_its_line_span() {
        let items = split_src("s = \"\"\"\na\n\"\"\"\n");
        assert_eq!(items.len(), 1);
        assert!(items[0].is_multiline());
    }

    #[test]
    fn a_file_without_a_final_newline_still_ends_with_an_item() {
        assert_eq!(shape("x = 1"), vec![(0, 1, 1, 0)]);
    }

    /// The token range is what the renderer lays out, so it must cover the
    /// item's own tokens and nothing else -- in particular not the `Newline`
    /// that ended it, nor a `Dedent` that follows it at end of file.
    #[test]
    fn an_item_owns_exactly_its_own_tokens() {
        let src = "f = (x) ->\n    x + 1\n";
        let tokens = Lexer::from_str(src.to_string())
            .keep_comments()
            .lex()
            .unwrap();
        let items = split(&tokens, &Spans::new(src, &tokens));
        let texts: Vec<Vec<&str>> = items
            .iter()
            .map(|i| {
                i.tokens
                    .clone()
                    .map(|n| tokens.get(n).unwrap().raw_text())
                    .collect()
            })
            .collect();
        assert_eq!(
            texts,
            vec![vec!["f", "=", "(", "x", ")", "->"], vec!["x", "+", "1"]]
        );
    }

    #[test]
    fn an_empty_source_has_no_items() {
        assert_eq!(shape(""), vec![]);
        assert_eq!(shape("\n\n\n"), vec![]);
    }
}
