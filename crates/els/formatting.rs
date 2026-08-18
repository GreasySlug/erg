//! `textDocument/formatting` -- format on save, and the editor's Format
//! Document command.
//!
//! Runs `erg_fmt` over the buffer and replaces the whole document with the
//! result. No type checking is involved, so this answers immediately even on a
//! large workspace, and it keeps working while the file is mid-edit: a source
//! `erg_fmt` cannot format safely comes back unchanged, which reaches the
//! client as an empty edit list rather than an error.

use erg_common::config::FmtConfig;
use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;
use erg_fmt::{format_str, FmtOptions};
use lsp_types::{DocumentFormattingParams, FormattingOptions, Position, Range, TextEdit};

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

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_formatting(
        &mut self,
        params: DocumentFormattingParams,
    ) -> ELSResult<Option<Vec<TextEdit>>> {
        _log!(self, "formatting requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document.uri);
        let code = self.file_cache.get_entire_code(&uri)?;
        let formatted = format_str(&code, options_for(&self.cfg.fmt, &params.options));
        if formatted == code {
            // an empty list is "nothing to do", which is what an unformattable
            // or already-formatted buffer means
            return Ok(Some(vec![]));
        }
        Ok(Some(vec![TextEdit {
            range: whole_document(&code),
            new_text: formatted,
        }]))
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
}
