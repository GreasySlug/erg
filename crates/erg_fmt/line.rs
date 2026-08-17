//! Splitting the token stream into the units that get laid out.
//!
//! A *logical line* is a run of tokens ending at a `Newline`. It is not always
//! one source line: inside brackets the lexer swallows newlines, so a call
//! broken over four lines arrives here as a single run.
//!
//! Comments need no special case. One that trails code lands in the same item
//! as that code, because the item is still open when it is reached; one on a
//! line of its own starts an item, because nothing is.

use std::ops::RangeInclusive;

use erg_common::traits::DequeStream;
use erg_parser::token::{Token, TokenKind, TokenStream};

/// The last source line a token occupies.
///
/// `Token` records only where it starts, but a `#[ ]#` comment or a `"""`
/// literal can be several lines tall, and an item that under-reports its span
/// would have its tail dropped when the renderer copies it out.
///
/// Counted on `raw_text`, not `content`: a `\n` escape decodes to a real
/// newline in `content` while occupying no line at all in the source, so
/// `content` would stretch `"a\nb"` across two lines that do not exist.
fn end_lineno(tok: &Token) -> u32 {
    tok.lineno + tok.raw_text().matches('\n').count() as u32
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
}

impl Item {
    /// Whether the item covers more than one source line.
    ///
    /// Those are emitted verbatim for now: the interior of a bracket run and
    /// the interior of a `"""` literal both live here, and re-indenting either
    /// needs care that belongs with the continuation-line stage.
    pub fn is_multiline(&self) -> bool {
        self.lines.end() > self.lines.start()
    }
}

/// Groups a token stream into items, in source order.
///
/// Expects a stream lexed with `keep_comments`; without it the comments are
/// simply absent and the result is still well-formed.
pub fn split(tokens: &TokenStream) -> Vec<Item> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut blanks = 0usize;
    // the item being built: (depth, first line, last line, blank lines before)
    let mut open: Option<(usize, u32, u32, usize)> = None;

    let close = |open: &mut Option<(usize, u32, u32, usize)>, items: &mut Vec<Item>| {
        if let Some((depth, first, last, blanks)) = open.take() {
            items.push(Item {
                depth,
                lines: first..=last,
                blank_lines_before: blanks,
            });
            true
        } else {
            false
        }
    };

    for tok in tokens.iter() {
        match tok.kind {
            TokenKind::Indent => depth += 1,
            TokenKind::Dedent => depth = depth.saturating_sub(1),
            // the first newline ends the item; any after it are blank lines
            TokenKind::Newline => {
                if !close(&mut open, &mut items) {
                    blanks += 1;
                }
            }
            TokenKind::EOF => {}
            _ => match &mut open {
                // a token can end on a later line than the item began on, both
                // because newlines inside brackets never reach us and because
                // a single token can itself be several lines tall
                Some((_, _, last, _)) => *last = (*last).max(end_lineno(tok)),
                None => {
                    open = Some((depth, tok.lineno, end_lineno(tok), blanks));
                    blanks = 0;
                }
            },
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
        split(&tokens)
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

    #[test]
    fn an_empty_source_has_no_items() {
        assert_eq!(shape(""), vec![]);
        assert_eq!(shape("\n\n\n"), vec![]);
    }
}
