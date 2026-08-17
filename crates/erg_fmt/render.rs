//! Turning items back into text.
//!
//! An [`Item`] is one logical line, which is not always one output line: a
//! bracketed expression the author broke over four lines arrives here as a
//! single item, because the lexer swallows newlines inside brackets. Those
//! breaks are recovered from the tokens' line numbers ([`Layout::chunks`]),
//! since that is the only trace of them left, and the formatter then adds
//! breaks of its own where a line is too wide ([`crate::reflow`]).
//!
//! Breaks are never *removed*. Flattening an expression back onto one line
//! would re-lex an operator that had been at end of line -- and so was read as
//! prefix -- as infix, changing the program. [`crate::verify`] would catch it,
//! but the file would come out unformatted; only ever splitting means the
//! situation cannot arise.

use erg_common::normalize_newline;
use erg_common::traits::DequeStream;
use erg_parser::token::{Token, TokenKind, TokenStream};

use crate::line::{split, Item};
use crate::reflow::wrap;
use crate::skip::Directives;
use crate::source::SourceLines;
use crate::spacing::space_between;
use crate::spans::{end_lineno, Spans};
use crate::FmtOptions;

/// A run of tokens that becomes one output line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Indentation, in units of [`FmtOptions::indent`].
    pub level: usize,
    /// Token indices, in source order.
    pub tokens: Vec<usize>,
}

/// Everything the line-level stages need to look at.
pub struct Layout<'a> {
    /// The source, newline-normalized -- which is the form the lexer read, and
    /// so the form `raw_text` and [`Spans`] agree with.
    pub src: &'a str,
    pub tokens: &'a TokenStream,
    pub spans: Spans,
    pub opts: FmtOptions,
}

impl<'a> Layout<'a> {
    pub fn new(src: &'a str, tokens: &'a TokenStream, opts: FmtOptions) -> Self {
        let spans = Spans::new(src, tokens);
        debug_assert!(
            spans.is_complete(tokens),
            "erg fmt could not locate every token in the source; \
             spacing would be guessed from here on"
        );
        Self {
            src,
            tokens,
            spans,
            opts,
        }
    }

    pub fn tok(&self, index: usize) -> &Token {
        self.tokens
            .get(index)
            .expect("token index came from this stream")
    }

    /// Splits an item at the line breaks the author wrote.
    ///
    /// Inside brackets no `Newline` is emitted, so the breaks survive only as
    /// jumps in the tokens' line numbers. A continuation line is indented by
    /// how many brackets are open at its start, which puts a closing bracket
    /// back level with the line that opened it.
    pub fn chunks(&self, item: &Item) -> Vec<Chunk> {
        let mut chunks: Vec<Chunk> = Vec::new();
        let mut open_brackets = 0usize;
        let mut prev_ends: Option<u32> = None;

        for index in item.tokens.clone() {
            let tok = self.tok(index);
            if is_layout(tok.kind) {
                continue;
            }
            let (first, last) = self
                .spans
                .line_span(index)
                .unwrap_or((tok.lineno, end_lineno(tok)));
            let broken_here = prev_ends.is_some_and(|ends| first > ends);
            if broken_here || prev_ends.is_none() {
                let level = match prev_ends {
                    None => item.depth,
                    Some(_) => item.depth + open_brackets - usize::from(is_closing(tok.kind)),
                };
                chunks.push(Chunk {
                    level,
                    tokens: Vec::new(),
                });
            }
            if let Some(chunk) = chunks.last_mut() {
                chunk.tokens.push(index);
            }
            if is_opening(tok.kind) {
                open_brackets += 1;
            } else if is_closing(tok.kind) {
                open_brackets = open_brackets.saturating_sub(1);
            }
            prev_ends = Some(last);
        }
        chunks
    }

    /// One output line, indentation included.
    ///
    /// May still contain newlines: a `"""` literal or a `#[ ]#` comment is a
    /// single token whose text is several lines tall, and that text is the
    /// source's, verbatim -- re-indenting it would rewrite the string.
    pub fn render_chunk(&self, chunk: &Chunk) -> String {
        let mut out = " ".repeat(chunk.level * self.opts.indent);
        for (nth, &index) in chunk.tokens.iter().enumerate() {
            if nth > 0 {
                let prev = chunk.tokens[nth - 1];
                if nth == 1 && self.tok(prev).kind == TokenKind::Comment {
                    // `#[]#print! 0` lexes; `#[]# print! 0` does not. A line
                    // that opens with a block comment is still in the lexer's
                    // indentation scan when the comment ends, and a space there
                    // is rejected outright -- so leave none.
                } else {
                    let gap = self.spans.gap(self.src, prev, index);
                    out.push_str(space_between(self.tok(prev), self.tok(index)).resolve(gap));
                }
            }
            out.push_str(self.tok(index).raw_text());
        }
        out
    }

    /// The width the chunk would occupy, in characters.
    ///
    /// Counted on the widest line, so a chunk holding a multi-line literal is
    /// judged by the literal rather than by the code around it. That is the
    /// conservative reading: [`crate::reflow`] declines to touch such a chunk
    /// anyway, and a wrong answer here would only make it try.
    pub fn width(&self, chunk: &Chunk) -> usize {
        self.render_chunk(chunk)
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0)
    }
}

/// Tokens that carry layout rather than text; the formatter emits its own.
pub(crate) fn is_layout(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Newline
            | TokenKind::Indent
            | TokenKind::Dedent
            | TokenKind::BOF
            | TokenKind::EOF
    )
}

pub(crate) fn is_opening(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::LParen | TokenKind::LSqBr | TokenKind::LBrace
    )
}

pub(crate) fn is_closing(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::RParen | TokenKind::RSqBr | TokenKind::RBrace
    )
}

/// Lays out a whole source.
pub fn render(src: &str, tokens: &TokenStream, opts: FmtOptions) -> String {
    let normalized = normalize_newline(src);
    let lines = SourceLines::new(&normalized);
    let directives = Directives::scan(tokens);
    let layout = Layout::new(&normalized, tokens, opts);

    let mut out = String::new();
    for (nth, item) in split(tokens, &layout.spans).iter().enumerate() {
        // blank lines are the formatter's to normalize, except at the top of
        // the file where they simply go
        let blanks = if nth == 0 {
            0
        } else {
            item.blank_lines_before.min(opts.max_blank_lines)
        };
        out.extend(std::iter::repeat_n('\n', blanks));

        if directives.suppresses(item.lines.clone()) {
            out.push_str(trim_terminator(&lines.range(item.lines.clone())));
            out.push('\n');
            continue;
        }
        for chunk in layout
            .chunks(item)
            .into_iter()
            .flat_map(|chunk| wrap(&layout, chunk))
        {
            out.push_str(layout.render_chunk(&chunk).trim_end());
            out.push('\n');
        }
    }
    restore_line_endings(out, src)
}

/// Drops the line terminator, which the caller re-adds. Also removes trailing
/// whitespace on the last line of the run.
fn trim_terminator(text: &str) -> &str {
    text.trim_end_matches(['\n', '\r'])
}

/// Restores CRLF if the input used it.
///
/// The lexer normalizes newlines before it does anything else, so `raw_text`
/// and every line copied out of the source arrive here as LF. Emitting that as
/// is would rewrite every line of a CRLF file and show up as an all-lines diff
/// on Windows.
fn restore_line_endings(out: String, src: &str) -> String {
    if src.contains("\r\n") {
        out.replace('\n', "\r\n")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::{format_str, FmtOptions};

    fn fmt(src: &str) -> String {
        format_str(src, FmtOptions::default())
    }

    fn fmt_with(src: &str, opts: FmtOptions) -> String {
        format_str(src, opts)
    }

    #[test]
    fn indentation_is_normalized_to_the_configured_width() {
        assert_eq!(fmt("f = (x) ->\n  x\n"), "f = (x) ->\n    x\n");
        assert_eq!(fmt("f = (x) ->\n        x\n"), "f = (x) ->\n    x\n");
    }

    #[test]
    fn nested_blocks_are_indented_by_depth() {
        assert_eq!(
            fmt("f = (x) ->\n  g = (y) ->\n      y\n  g\n"),
            "f = (x) ->\n    g = (y) ->\n        y\n    g\n"
        );
    }

    #[test]
    fn the_indent_width_is_configurable() {
        let opts = FmtOptions {
            indent: 2,
            ..FmtOptions::default()
        };
        assert_eq!(fmt_with("f = (x) ->\n    x\n", opts), "f = (x) ->\n  x\n");
    }

    #[test]
    fn runs_of_blank_lines_are_capped() {
        assert_eq!(fmt("x = 1\n\n\n\n\ny = 2\n"), "x = 1\n\n\ny = 2\n");
        // one and two are already within the cap
        assert_eq!(fmt("x = 1\n\ny = 2\n"), "x = 1\n\ny = 2\n");
    }

    #[test]
    fn the_blank_line_cap_is_configurable() {
        let opts = FmtOptions {
            max_blank_lines: 1,
            ..FmtOptions::default()
        };
        assert_eq!(fmt_with("x = 1\n\n\n\ny = 2\n", opts), "x = 1\n\ny = 2\n");
    }

    #[test]
    fn leading_and_trailing_blank_lines_go() {
        assert_eq!(fmt("\n\n\nx = 1\n"), "x = 1\n");
        assert_eq!(fmt("x = 1\n\n\n"), "x = 1\n");
        assert_eq!(fmt("\n\nx = 1\n\n\n"), "x = 1\n");
    }

    /// Only spaces: a tab is `Illegal` to the lexer, so a source containing one
    /// never reaches the renderer at all.
    #[test]
    fn trailing_whitespace_goes() {
        assert_eq!(fmt("x = 1   \ny = 2  \n"), "x = 1\ny = 2\n");
    }

    #[test]
    fn a_missing_final_newline_is_added() {
        assert_eq!(fmt("x = 1"), "x = 1\n");
    }

    #[test]
    fn an_empty_source_stays_empty() {
        assert_eq!(fmt(""), "");
        assert_eq!(fmt("\n\n\n"), "");
    }

    #[test]
    fn comments_are_indented_with_their_block() {
        assert_eq!(
            fmt("f = (x) ->\n  # c\n  x\n"),
            "f = (x) ->\n    # c\n    x\n"
        );
    }

    #[test]
    fn a_trailing_comment_stays_on_its_line() {
        assert_eq!(fmt("x = 1 # c\n"), "x = 1 # c\n");
        // a run of spaces may be aligning a column of them, so it survives
        assert_eq!(fmt("x = 1     # c\n"), "x = 1     # c\n");
    }

    /// The interior of a `"""` literal is part of the string, so only the line
    /// it starts on may be re-indented.
    #[test]
    fn a_multi_line_string_keeps_its_interior() {
        assert_eq!(
            fmt("f = (x) ->\n        s = \"\"\"\n    a\n    \"\"\"\n"),
            "f = (x) ->\n    s = \"\"\"\n    a\n    \"\"\"\n"
        );
    }

    #[test]
    fn a_multi_line_comment_keeps_its_interior() {
        let src = "#[ a\n     b ]#\nx = 1\n";
        assert_eq!(fmt(src), src);
    }

    /// The breaks the author wrote inside brackets survive; the indentation
    /// around them is normalized to the nesting.
    #[test]
    fn continuation_lines_are_indented_by_bracket_nesting() {
        assert_eq!(
            fmt("_ = f(\n      1,\n        2\n)\n"),
            "_ = f(\n    1,\n    2\n)\n"
        );
        assert_eq!(
            fmt("f = (x) ->\n  _ = [\n   1,\n  ]\n"),
            "f = (x) ->\n    _ = [\n        1,\n    ]\n"
        );
    }

    #[test]
    fn a_nested_bracket_indents_one_further() {
        assert_eq!(
            fmt("_ = f(a, [\n  1,\n], b)\n"),
            "_ = f(a, [\n        1,\n    ], b)\n"
        );
    }

    /// Only ever split, never join: a break the author put inside brackets
    /// stays, even when the line would fit without it.
    #[test]
    fn an_existing_break_is_never_undone() {
        assert_eq!(
            fmt("_ = f(\n    1,\n    2,\n)\n"),
            "_ = f(\n    1,\n    2,\n)\n"
        );
    }

    #[test]
    fn fmt_off_suppresses_a_region() {
        let src = "# fmt: off\nf = (x) ->\n      x\n# fmt: on\ng = (y) ->\n      y\n";
        assert_eq!(
            fmt(src),
            "# fmt: off\nf = (x) ->\n      x\n# fmt: on\ng = (y) ->\n    y\n"
        );
    }

    #[test]
    fn fmt_skip_suppresses_one_line() {
        assert_eq!(
            fmt("f = (x) ->\n      x # fmt: skip\ng = 1\n"),
            "f = (x) ->\n      x # fmt: skip\ng = 1\n"
        );
    }

    #[test]
    fn a_leading_fmt_off_suppresses_the_file() {
        let src = "# fmt: off\nf = (x) ->\n      x\n";
        assert_eq!(fmt(src), src);
    }

    #[test]
    fn crlf_input_stays_crlf() {
        assert_eq!(fmt("f = (x) ->\r\n  x\r\n"), "f = (x) ->\r\n    x\r\n");
    }

    #[test]
    fn lf_input_stays_lf() {
        assert_eq!(fmt("f = (x) ->\n  x\n"), "f = (x) ->\n    x\n");
    }

    #[test]
    fn formatting_is_idempotent() {
        for src in [
            "f = (x) ->\n  x\n",
            "\n\n\nx = 1\n\n\n\n\ny = 2\n\n",
            "# c\nf = (x) ->\n      # d\n      x\n",
            "_ = f(\n    1,\n    2\n)\n",
            "_ = f(a, [\n  1,\n], b)\n",
            "# fmt: off\n  x = 1\n# fmt: on\n  y = 2\n",
            "s = \"\"\"\n  a\n  \"\"\"\n",
        ] {
            let once = fmt(src);
            assert_eq!(fmt(&once), once, "not idempotent for {src:?}");
        }
    }
}
