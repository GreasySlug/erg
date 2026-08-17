//! Addressing the source by line.
//!
//! Suppressed regions are emitted by copying the original text back out, and
//! the only coordinate that can be trusted for that is the line number. Columns
//! cannot: the lexer advances them by the *decoded* length of a token, so
//! `col_end` drifts from the source on any string containing an escape.
//! Everything here therefore works in 1-origin line numbers and never looks at
//! a column.

use std::ops::RangeInclusive;

/// A source split into lines, each keeping its own line terminator.
///
/// `str::lines` is not usable here: it drops the terminator, which loses both
/// the CRLF/LF distinction and whether the file ended with a newline. Copying a
/// region has to reproduce it byte for byte.
#[derive(Debug, Clone)]
pub struct SourceLines<'a> {
    lines: Vec<&'a str>,
}

impl<'a> SourceLines<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            lines: src.split_inclusive('\n').collect(),
        }
    }

    /// Number of lines. An empty source has none.
    pub fn len(&self) -> u32 {
        self.lines.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// One line, 1-origin, including its terminator. `None` past the end.
    pub fn get(&self, lineno: u32) -> Option<&'a str> {
        self.lines.get(lineno.checked_sub(1)? as usize).copied()
    }

    /// The source text of an inclusive 1-origin line range, verbatim.
    ///
    /// Out-of-range lines are skipped rather than panicking: a token's line
    /// number is only as trustworthy as the lexer that produced it, and a
    /// formatter should degrade rather than crash on a surprise.
    pub fn range(&self, lines: RangeInclusive<u32>) -> String {
        let mut out = String::new();
        for lineno in lines {
            if let Some(line) = self.get(lineno) {
                out.push_str(line);
            }
        }
        out
    }

    /// The whole source, as reassembled from the lines.
    ///
    /// Equal to the input; the tests use it to pin that splitting and rejoining
    /// is lossless, which is what makes [`SourceLines::range`] safe to emit.
    pub fn all(&self) -> String {
        self.lines.concat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_and_rejoining_is_lossless() {
        for src in [
            "",
            "x = 1\n",
            "x = 1",          // no trailing newline
            "x = 1\r\ny = 2", // CRLF, and a bare last line
            "\n\n\n",
            "a\nb\nc\n",
        ] {
            assert_eq!(SourceLines::new(src).all(), src, "for {src:?}");
        }
    }

    #[test]
    fn lines_are_one_origin_and_keep_their_terminator() {
        let src = SourceLines::new("a\nb\nc\n");
        assert_eq!(src.len(), 3);
        assert_eq!(src.get(1), Some("a\n"));
        assert_eq!(src.get(3), Some("c\n"));
        assert_eq!(src.get(0), None, "there is no line 0");
        assert_eq!(src.get(4), None);
    }

    #[test]
    fn a_range_reproduces_the_original_text() {
        let src = SourceLines::new("a\nb\nc\nd\n");
        assert_eq!(src.range(2..=3), "b\nc\n");
        assert_eq!(src.range(1..=1), "a\n");
        assert_eq!(src.range(1..=4), "a\nb\nc\nd\n");
    }

    #[test]
    fn an_out_of_range_line_is_skipped_not_a_panic() {
        let src = SourceLines::new("a\nb\n");
        assert_eq!(src.range(1..=99), "a\nb\n");
        assert_eq!(src.range(50..=99), "");
    }

    /// The whole-file range is exactly the verification fallback, which is the
    /// property that lets both use one code path.
    #[test]
    fn the_full_range_equals_the_source() {
        let text = "# c\nx = 1\n\nf = (y) ->\n    y\n";
        let src = SourceLines::new(text);
        assert_eq!(src.range(1..=src.len()), text);
    }
}
