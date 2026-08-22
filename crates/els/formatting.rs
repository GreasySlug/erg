//! `textDocument/formatting` and `textDocument/rangeFormatting`.
//!
//! Both run `erg_fmt` over the buffer and do not type-check, so they answer
//! immediately even on a large workspace, and they keep working while the file
//! is mid-edit: a source `erg_fmt` cannot format safely comes back unchanged,
//! which reaches the client as an empty edit list rather than an error.
//!
//! Full-document formatting replaces the whole buffer. Range formatting first
//! formats the whole buffer (the formatter is not range-aware), then keeps only
//! the changed region when that region sits inside the requested range -- the
//! LSP spec requires every edit to fall within it. A selection that does not
//! contain the change is left alone.

use erg_common::config::FmtConfig;
use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;
use erg_fmt::{format_str, FmtOptions};
use lsp_types::{
    DocumentFormattingParams, DocumentRangeFormattingParams, FormattingOptions, Position, Range,
    TextEdit,
};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::NormalizedUrl;

/// Merges the editor's settings into the server's.
///
/// `insert_spaces` is deliberately ignored. Erg's lexer rejects a tab outright
/// ([`erg_parser::lex`]), so honouring "indent with tabs" would produce a file
/// that no longer lexes -- and the verification would then throw the result
/// away, leaving the user with a format command that silently does nothing.
fn options_for(cfg: &FmtConfig, editor: &FormattingOptions) -> FmtOptions {
    FmtOptions {
        // `.max(1)` because a client may send 0 and an indent of 0 flattens
        // every block, which the verification refuses -- turning a misreported
        // editor setting into a format command that silently does nothing.
        indent: (editor.tab_size as usize).max(1),
        ..FmtOptions::from(cfg)
    }
}

/// The range covering an entire document.
///
/// LSP positions count UTF-16 code units, not bytes or characters, so the end
/// column has to be measured that way -- otherwise a last line containing any
/// non-ASCII character would leave a tail of it unreplaced.
fn whole_document(code: &str) -> Range {
    let last = code.split('\n').next_back().unwrap_or("");
    let lines = code.split('\n').count() as u32;
    Range {
        start: Position::new(0, 0),
        end: Position::new(lines.saturating_sub(1), last.encode_utf16().count() as u32),
    }
}

fn pos_le(a: Position, b: Position) -> bool {
    a.line < b.line || (a.line == b.line && a.character <= b.character)
}

fn range_contains(outer: Range, inner: Range) -> bool {
    pos_le(outer.start, inner.start) && pos_le(inner.end, outer.end)
}

/// The smallest line-aligned region that differs between `original` and
/// `formatted`, plus the formatted text that should replace it.
///
/// Built from a common prefix/suffix of `split('\n')` lines, which keeps a
/// trailing empty line when the file ends in a newline -- the same split
/// [`whole_document`] uses.
fn changed_region(original: &str, formatted: &str) -> Option<(Range, String)> {
    if original == formatted {
        return None;
    }
    let old: Vec<&str> = original.split('\n').collect();
    let new: Vec<&str> = formatted.split('\n').collect();
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut old_suffix = 0;
    let mut new_suffix = 0;
    while prefix + old_suffix < old.len()
        && prefix + new_suffix < new.len()
        && old[old.len() - 1 - old_suffix] == new[new.len() - 1 - new_suffix]
    {
        old_suffix += 1;
        new_suffix += 1;
    }
    let start = Position::new(prefix as u32, 0);
    let end = if old_suffix == 0 {
        let last = old.last().copied().unwrap_or("");
        Position::new(
            (old.len().saturating_sub(1)) as u32,
            last.encode_utf16().count() as u32,
        )
    } else {
        Position::new((old.len() - old_suffix) as u32, 0)
    };
    let mid = &new[prefix..new.len() - new_suffix];
    let mut replacement = mid.join("\n");
    if old_suffix > 0 {
        replacement.push('\n');
    }
    Some((Range { start, end }, replacement))
}

fn full_document_edits(code: &str, opts: FmtOptions) -> Vec<TextEdit> {
    let formatted = format_str(code, opts);
    if formatted == code {
        return vec![];
    }
    vec![TextEdit {
        range: whole_document(code),
        new_text: formatted,
    }]
}

fn range_edits(code: &str, range: Range, opts: FmtOptions) -> Vec<TextEdit> {
    let formatted = format_str(code, opts);
    let Some((change, new_text)) = changed_region(code, &formatted) else {
        return vec![];
    };
    if range_contains(range, change) {
        vec![TextEdit {
            range: change,
            new_text,
        }]
    } else {
        vec![]
    }
}

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_formatting(
        &mut self,
        params: DocumentFormattingParams,
    ) -> ELSResult<Option<Vec<TextEdit>>> {
        _log!(self, "formatting requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document.uri);
        let code = self.file_cache.get_entire_code(&uri)?;
        Ok(Some(full_document_edits(
            &code,
            options_for(&self.cfg.fmt, &params.options),
        )))
    }

    pub(crate) fn handle_range_formatting(
        &mut self,
        params: DocumentRangeFormattingParams,
    ) -> ELSResult<Option<Vec<TextEdit>>> {
        _log!(self, "range formatting requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document.uri);
        let code = self.file_cache.get_entire_code(&uri)?;
        Ok(Some(range_edits(
            &code,
            params.range,
            options_for(&self.cfg.fmt, &params.options),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_document_range_reaches_the_end() {
        assert_eq!(whole_document("").end, Position::new(0, 0));
        assert_eq!(whole_document("x = 1\n").end, Position::new(1, 0));
        assert_eq!(whole_document("x = 1\ny = 2").end, Position::new(1, 5));
    }

    /// Counted in UTF-16 code units, which is neither characters nor bytes:
    /// `あ` is one unit and three bytes, `🦀` is two units and four.
    #[test]
    fn the_end_column_is_in_utf16_units() {
        // `_ = "あ"` -- 6 ASCII + 1
        assert_eq!(whole_document("_ = \"あ\"").end.character, 7);
        // `_ = "🦀"` -- 6 ASCII + 2, the surrogate pair
        assert_eq!(whole_document("_ = \"🦀\"").end.character, 8);
    }

    #[test]
    fn the_editors_tab_size_becomes_the_indent() {
        let editor = FormattingOptions {
            tab_size: 2,
            insert_spaces: true,
            ..FormattingOptions::default()
        };
        let opts = options_for(&FmtConfig::default(), &editor);
        assert_eq!(opts.indent, 2);
        let zero = FormattingOptions {
            tab_size: 0,
            ..FormattingOptions::default()
        };
        assert_eq!(
            options_for(&FmtConfig::default(), &zero).indent,
            1,
            "an indent of 0 would flatten every block"
        );
        assert_eq!(
            opts.max_width,
            FmtConfig::default().max_width,
            "the rest still comes from the server's config"
        );
    }

    #[test]
    fn changed_region_is_the_first_line_when_only_it_differs() {
        let (range, text) =
            changed_region("x =   1\n_ = x + 1\n", "x = 1\n_ = x + 1\n").expect("a change");
        assert_eq!(range.start, Position::new(0, 0));
        assert_eq!(range.end, Position::new(1, 0));
        assert_eq!(text, "x = 1\n");
    }

    #[test]
    fn range_edits_keep_only_a_change_inside_the_selection() {
        let opts = FmtOptions::default();
        let code = "x =   1\n_ = x + 1\n";
        let first_line = Range {
            start: Position::new(0, 0),
            end: Position::new(1, 0),
        };
        let edits = range_edits(code, first_line, opts);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "x = 1\n");
        assert_eq!(edits[0].range, first_line);

        let second_line = Range {
            start: Position::new(1, 0),
            end: Position::new(2, 0),
        };
        assert!(
            range_edits(code, second_line, opts).is_empty(),
            "a selection that does not contain the change must not be rewritten"
        );
    }
}
