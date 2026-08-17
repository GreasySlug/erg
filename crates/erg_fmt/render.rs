//! Turning items back into text.
//!
//! At this stage only the layout *around* a line is normalized -- its
//! indentation, the blank lines before it, trailing whitespace, the file's
//! final newline. What is written on the line is still copied from the source,
//! so nothing here can disturb the spacing that decides operator fixity. That
//! comes next, and arrives under the verification already in place.

use erg_parser::token::TokenStream;

use crate::line::{split, Item};
use crate::skip::Directives;
use crate::source::SourceLines;
use crate::FmtOptions;

/// Lays out a whole source.
pub fn render(src: &str, tokens: &TokenStream, opts: FmtOptions) -> String {
    let lines = SourceLines::new(src);
    let directives = Directives::scan(tokens);
    let items = split(tokens);

    let mut out = String::new();
    for (nth, item) in items.iter().enumerate() {
        // blank lines are the formatter's to normalize, except at the top of
        // the file where they simply go
        let blanks = if nth == 0 {
            0
        } else {
            item.blank_lines_before.min(opts.max_blank_lines)
        };
        out.extend(std::iter::repeat_n('\n', blanks));

        if keep_verbatim(item, &directives) {
            out.push_str(trim_terminator(&lines.range(item.lines.clone())));
        } else {
            let line = lines.get(*item.lines.start()).unwrap_or_default();
            out.push_str(&" ".repeat(item.depth * opts.indent));
            out.push_str(line.trim());
        }
        out.push('\n');
    }
    restore_line_endings(out, src)
}

/// Whether an item is copied out rather than laid out.
fn keep_verbatim(item: &Item, directives: &Directives) -> bool {
    // Spanning several source lines means either a bracket run or the inside
    // of a `"""` literal. The literal's interior is part of the string and must
    // not be touched at all; the bracket run needs continuation-line handling
    // that does not exist yet. Copying covers both correctly.
    item.is_multiline() || directives.suppresses(item.lines.clone())
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
        // normalize first: verbatim runs may already carry CRLF
        out.replace("\r\n", "\n").replace('\n', "\r\n")
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
    }

    /// The interior of a `"""` literal is part of the string, so the whole item
    /// is copied out rather than re-indented.
    #[test]
    fn a_multi_line_string_is_left_alone() {
        let src = "f = (x) ->\n        s = \"\"\"\n    a\n    \"\"\"\n";
        assert_eq!(fmt(src), src, "not even the first line, for now");
    }

    #[test]
    fn a_multi_line_comment_is_left_alone() {
        let src = "#[ a\n     b ]#\nx = 1\n";
        assert_eq!(fmt(src), src);
    }

    /// Continuation lines inside brackets are the next stage's business.
    #[test]
    fn a_bracketed_run_is_left_alone() {
        let src = "_ = f(\n      1,\n        2\n)\n";
        assert_eq!(fmt(src), src);
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
            "# fmt: off\n  x = 1\n# fmt: on\n  y = 2\n",
        ] {
            let once = fmt(src);
            assert_eq!(fmt(&once), once, "not idempotent for {src:?}");
        }
    }
}
