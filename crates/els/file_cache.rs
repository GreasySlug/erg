use std::fmt;
use std::fs::File;
use std::io::Read;
use std::sync::mpsc::Sender;

use lsp_types::{
    DidChangeTextDocumentParams, FileOperationFilter, FileOperationPattern,
    FileOperationPatternKind, FileOperationRegistrationOptions, OneOf, Position, Range,
    RenameFilesParams, SaveOptions, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, Url, WorkspaceFileOperationsServerCapabilities,
    WorkspaceFoldersServerCapabilities, WorkspaceServerCapabilities,
};
use serde_json::Value;

use erg_common::dict::Dict;
use erg_common::lsp_log;
use erg_common::set::Set;
use erg_common::shared::Shared;
use erg_common::traits::DequeStream;
use erg_common::vfs::VFS;
use erg_compiler::erg_parser::lex::Lexer;
use erg_compiler::erg_parser::token::{Token, TokenCategory, TokenKind, TokenStream};

use crate::server::{ELSResult, RedirectableStdout};
use crate::util::{self, NormalizedUrl};

fn _get_code_from_uri(uri: &Url) -> ELSResult<String> {
    let path = uri
        .to_file_path()
        .or_else(|_| util::denormalize(uri.clone()).to_file_path())
        .map_err(|_| format!("invalid file path: {uri}"))?;
    let mut code = String::new();
    File::open(path.as_path())?.read_to_string(&mut code)?;
    Ok(code)
}

#[derive(Debug, Clone)]
pub struct FileCacheEntry {
    pub code: String,
    pub ver: i32,
    pub token_stream: Option<TokenStream>,
    /// Regions covered by comments, memoized by [`FileCache::cursor_in_comment`].
    /// `None` means "not computed for this `code` yet" -- every write to `code`
    /// must clear it. Deciding this needs the lexer's view of what is a comment
    /// and what is a `#` inside a string, so it cannot be answered per-line.
    comment_spans: Option<Vec<Range>>,
}

impl FileCacheEntry {
    fn new(code: String, ver: i32, token_stream: Option<TokenStream>) -> Self {
        Self {
            code,
            ver,
            token_stream,
            comment_spans: None,
        }
    }

    /// line: 0-based
    pub fn get_line(&self, line0: u32) -> Option<String> {
        let mut lines = self.code.lines();
        lines.nth(line0 as usize).map(|s| s.to_string())
    }
}

/// Half-open at both ends, so code before `#[` or after `]#` is not "in" the
/// comment. Matches how a single-line `#` comment's `col_begin..col_end` reads.
fn span_contains(span: Range, pos: Position) -> bool {
    let key = |p: Position| (p.line, p.character);
    key(span.start) <= key(pos) && key(pos) < key(span.end)
}

/// The UTF-16 column `char_col` chars into `line`.
///
/// The lexer counts `Token::col_begin` in `char`s, while LSP positions count
/// UTF-16 code units. They agree until a line contains a character outside the
/// BMP -- `x = "𝒳" # hi` puts the `#` at char column 8 but UTF-16 column 9.
fn utf16_col(line: &str, char_col: u32) -> u32 {
    line.chars()
        .take(char_col as usize)
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// The regions `#` comments, `#[ ... ]#` blocks and `'''` doc comments occupy,
/// in LSP coordinates.
fn comment_spans_of(code: &str) -> Vec<Range> {
    let lines: Vec<&str> = code.split('\n').collect();
    let tokens = match Lexer::from_str(code.to_string()).keep_comments().lex() {
        Ok(ts) => ts,
        Err((ts, _)) => ts,
    };
    tokens
        .iter()
        .filter(|tk| matches!(tk.kind, TokenKind::Comment | TokenKind::DocComment))
        .map(|tk| {
            let line = tk.lineno.saturating_sub(1);
            let start_col = lines
                .get(line as usize)
                .map_or(tk.col_begin, |src| utf16_col(src, tk.col_begin));
            let start = Position::new(line, start_col);
            // `Token` only stores the starting line, so the extent comes from
            // its own text -- which is the verbatim source of the comment,
            // delimiters included (`# hi`, `#[ a\nb ]#`, `'''doc'''`).
            let extra = tk.content.matches('\n').count() as u32;
            let end = if extra == 0 {
                Position::new(line, start_col + tk.content.encode_utf16().count() as u32)
            } else {
                // A continuation line of the comment starts at column 0, so its
                // own width is the column just past the comment's close.
                let last = tk.content.split('\n').next_back().unwrap_or("");
                Position::new(line + extra, last.encode_utf16().count() as u32)
            };
            Range::new(start, end)
        })
        .collect()
}

/// Stores the contents of the file on-memory.
/// This struct can save changes in real-time & incrementally.
#[derive(Debug, Clone, Default)]
pub struct FileCache {
    stdout_redirect: Option<Sender<Value>>,
    pub files: Shared<Dict<NormalizedUrl, FileCacheEntry>>,
    pub editing: Shared<Set<NormalizedUrl>>,
    /// Documents that received `textDocument/didOpen` and have not yet been closed.
    /// Distinct from `files`, which also holds dependents loaded from disk.
    opened: Shared<Set<NormalizedUrl>>,
}

impl RedirectableStdout for FileCache {
    fn sender(&self) -> Option<&Sender<Value>> {
        self.stdout_redirect.as_ref()
    }
}

impl fmt::Display for FileCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "FileCache {{")?;
        for (key, entry) in self.files.borrow().iter() {
            writeln!(f, "{key}: \"{}\"", entry.code)?;
        }
        writeln!(f, "}}")?;
        Ok(())
    }
}

impl FileCache {
    pub fn new(stdout_redirect: Option<Sender<Value>>) -> Self {
        Self {
            stdout_redirect,
            files: Shared::new(Dict::new()),
            editing: Shared::new(Set::new()),
            opened: Shared::new(Set::new()),
        }
    }

    #[allow(unused)]
    pub fn clear(&self) {
        self.files.borrow_mut().clear();
        self.editing.borrow_mut().clear();
        self.opened.borrow_mut().clear();
    }

    fn load_once(&self, uri: &NormalizedUrl) -> ELSResult<()> {
        if self.files.borrow_mut().get(uri).is_some() {
            return Ok(());
        }
        let code = _get_code_from_uri(uri)?;
        self.update(uri, code, None);
        Ok(())
    }

    pub fn mark_open(&self, uri: &NormalizedUrl) {
        self.opened.borrow_mut().insert(uri.clone());
    }

    pub fn is_open(&self, uri: &NormalizedUrl) -> bool {
        self.opened.borrow().contains(uri)
    }

    /// Replace the cached contents with what is currently on disk.
    pub fn reload_from_disk(&self, uri: &NormalizedUrl) -> ELSResult<String> {
        let code = _get_code_from_uri(uri)?;
        self.update(uri, code.clone(), None);
        Ok(code)
    }

    pub(crate) fn set_capabilities(&mut self, capabilities: &mut ServerCapabilities) {
        let workspace_folders = WorkspaceFoldersServerCapabilities {
            supported: Some(true),
            change_notifications: Some(OneOf::Left(true)),
        };
        let file_op = FileOperationRegistrationOptions {
            filters: vec![
                FileOperationFilter {
                    scheme: Some(String::from("file")),
                    pattern: FileOperationPattern {
                        glob: String::from("**/*.{er,py}"),
                        matches: Some(FileOperationPatternKind::File),
                        options: None,
                    },
                },
                FileOperationFilter {
                    scheme: Some(String::from("file")),
                    pattern: FileOperationPattern {
                        glob: String::from("**"),
                        matches: Some(FileOperationPatternKind::Folder),
                        options: None,
                    },
                },
            ],
        };
        capabilities.workspace = Some(WorkspaceServerCapabilities {
            workspace_folders: Some(workspace_folders),
            file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                will_rename: Some(file_op),
                ..Default::default()
            }),
        });
        let sync_option = TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::INCREMENTAL),
            save: Some(SaveOptions::default().into()),
            ..Default::default()
        };
        capabilities.text_document_sync = Some(TextDocumentSyncCapability::Options(sync_option));
    }

    /// This method clones and returns the entire file.
    /// If you only need part of the file, use `get_ranged` or `get_line` instead.
    pub fn get_entire_code(&self, uri: &NormalizedUrl) -> ELSResult<String> {
        self.load_once(uri)?;
        Ok(self
            .files
            .borrow_mut()
            .get(uri)
            .ok_or("file entry not found")?
            .code
            .clone())
    }

    pub fn get_token_stream(&self, uri: &NormalizedUrl) -> Option<TokenStream> {
        let _ = self.load_once(uri);
        self.files.borrow_mut().get(uri)?.token_stream.clone()
    }

    /// True when `pos` sits in a `#` comment, `#[ ... ]#` block, or `'''` doc comment.
    ///
    /// Only the interior of a block or doc comment is whole-line: code before
    /// `#[` or after `]#` / `'''` on the same line must still complete, so the
    /// span is half-open at both ends.
    ///
    /// Completion asks this on every keystroke, so the spans are memoized per
    /// revision instead of re-lexing the buffer each time, and a file with no
    /// comment opener at all never lexes. The lexing itself happens with the
    /// cache unlocked -- the other workers share this cache, and none of them
    /// should wait behind a lex.
    pub fn cursor_in_comment(&self, uri: &NormalizedUrl, pos: Position) -> bool {
        let _ = self.load_once(uri);
        let code = {
            let lock = self.files.borrow();
            let Some(entry) = lock.get(uri) else {
                return false;
            };
            if let Some(spans) = entry.comment_spans.as_deref() {
                return spans.iter().any(|span| span_contains(*span, pos));
            }
            entry.code.clone()
        };
        let spans = if code.contains('#') || code.contains("'''") {
            comment_spans_of(&code)
        } else {
            vec![]
        };
        let found = spans.iter().any(|span| span_contains(*span, pos));
        if let Some(entry) = self.files.borrow_mut().get_mut(uri) {
            // The buffer may have been edited while this ran; a memo for text
            // that is no longer there would answer later queries wrongly.
            if entry.code == code {
                entry.comment_spans = Some(spans);
            }
        }
        found
    }

    pub fn get_token(&self, uri: &NormalizedUrl, pos: Position) -> Option<Token> {
        let _ = self.load_once(uri);
        let ent = self.files.borrow_mut();
        let tokens = ent.get(uri)?.token_stream.as_ref()?;
        for tok in tokens.iter() {
            if util::pos_in_loc(tok, pos) {
                return Some(tok.clone());
            }
        }
        for tok in tokens.iter() {
            if util::roughly_pos_in_loc(tok, pos) {
                return Some(tok.clone());
            }
        }
        None
    }

    /// a{pos}\n -> \n -> a
    pub fn get_symbol(&self, uri: &NormalizedUrl, pos: Position) -> Option<Token> {
        let mut token = self.get_token(uri, pos)?;
        let mut offset = 0;
        while !matches!(token.category(), TokenCategory::Symbol) {
            offset -= 1;
            token = self.get_token_relatively(uri, pos, offset)?;
        }
        Some(token)
    }

    pub fn get_receiver(&self, uri: &NormalizedUrl, attr_marker_pos: Position) -> Option<Token> {
        let mut token = self.get_token(uri, attr_marker_pos)?;
        let mut offset = 0;
        while !matches!(token.kind, TokenKind::Dot | TokenKind::DblColon) {
            offset -= 1;
            token = self.get_token_relatively(uri, attr_marker_pos, offset)?;
        }
        offset -= 1;
        self.get_token_relatively(uri, attr_marker_pos, offset)
    }

    pub fn get_token_relatively(
        &self,
        uri: &NormalizedUrl,
        pos: Position,
        offset: isize,
    ) -> Option<Token> {
        if offset == 0 {
            return self.get_token(uri, pos);
        }
        let _ = self.load_once(uri);
        let ent = self.files.borrow_mut();
        let tokens = ent.get(uri)?.token_stream.as_ref()?;
        let index = (|| {
            for (i, tok) in tokens.iter().enumerate() {
                if util::pos_in_loc(tok, pos) {
                    return Some(i);
                }
            }
            for (i, tok) in tokens.iter().enumerate() {
                if util::roughly_pos_in_loc(tok, pos) {
                    return Some(i);
                }
            }
            None
        })()?;
        let index = (index as isize + offset) as usize;
        if index < tokens.len() {
            Some(tokens[index].clone())
        } else {
            None
        }
    }

    /// 0-based
    pub(crate) fn get_line(&self, uri: &NormalizedUrl, line0: u32) -> Option<String> {
        let _ = self.load_once(uri);
        self.files.borrow_mut().get(uri)?.get_line(line0)
    }

    pub(crate) fn get_ranged(
        &self,
        uri: &NormalizedUrl,
        range: Range,
    ) -> ELSResult<Option<String>> {
        self.load_once(uri)?;
        let ent = self.files.borrow_mut();
        let file = ent.get(uri).ok_or("file entry not found")?;
        let mut code = String::new();
        for (i, line) in file.code.lines().enumerate() {
            if i >= range.start.line as usize && i <= range.end.line as usize {
                if i == range.start.line as usize && i == range.end.line as usize {
                    if line.len() < range.end.character as usize {
                        return Ok(None);
                    }
                    code.push_str(
                        &line[range.start.character as usize..range.end.character as usize],
                    );
                } else if i == range.start.line as usize {
                    code.push_str(&line[range.start.character as usize..]);
                    code.push('\n');
                } else if i == range.end.line as usize {
                    if line.len() < range.end.character as usize {
                        return Ok(None);
                    }
                    code.push_str(&line[..range.end.character as usize]);
                } else {
                    code.push_str(line);
                    code.push('\n');
                }
            }
        }
        Ok(Some(code))
    }

    pub(crate) fn update(&self, uri: &NormalizedUrl, code: String, ver: Option<i32>) {
        let lock = self.files.borrow_mut();
        let entry = lock.get(uri);
        if let Some(entry) = entry {
            if ver.is_some_and(|ver| ver <= entry.ver) {
                // crate::_log!(self, "171: double update detected: {ver:?}, {}, code:\n{}", entry.ver, entry.code);
                return;
            }
        }
        let token_stream = match Lexer::from_str(code.clone()).lex() {
            Ok(ts) => Some(ts),
            Err((ts, es)) => {
                lsp_log!("failed to lex: {es}");
                Some(ts)
            }
        };
        let ver = ver.unwrap_or({
            if let Some(entry) = entry {
                entry.ver
            } else {
                1
            }
        });
        drop(lock);
        VFS.update(uri.to_file_path().unwrap(), code.clone());
        self.files
            .borrow_mut()
            .insert(uri.clone(), FileCacheEntry::new(code, ver, token_stream));
    }

    pub(crate) fn _ranged_update(&self, uri: &NormalizedUrl, old: Range, new_code: &str) {
        let mut ent = self.files.borrow_mut();
        let Some(entry) = ent.get_mut(uri) else {
            return;
        };
        let mut code = entry.code.clone();
        let start = util::pos_to_byte_index(&code, old.start);
        let end = util::pos_to_byte_index(&code, old.end);
        code.replace_range(start..end, new_code);
        let token_stream = match Lexer::from_str(code.clone()).lex() {
            Ok(ts) => Some(ts),
            Err((ts, es)) => {
                lsp_log!("failed to lex: {es}");
                Some(ts)
            }
        };
        VFS.update(uri.to_file_path().unwrap(), code.clone());
        entry.code = code;
        // entry.ver += 1;
        entry.token_stream = token_stream;
        entry.comment_spans = None;
    }

    pub(crate) fn incremental_update(&self, params: DidChangeTextDocumentParams) {
        let uri = NormalizedUrl::new(params.text_document.uri);
        let mut ent = self.files.borrow_mut();
        let Some(entry) = ent.get_mut(&uri) else {
            return;
        };
        if entry.ver >= params.text_document.version {
            crate::_log!(
                self,
                "212: double update detected {}, {}, code:\n{}",
                entry.ver,
                params.text_document.version,
                entry.code
            );
            return;
        }
        let mut code = entry.code.clone();
        for change in params.content_changes {
            let Some(range) = change.range else {
                continue;
            };
            let start = util::pos_to_byte_index(&code, range.start);
            let end = util::pos_to_byte_index(&code, range.end);
            code.replace_range(start..end, &change.text);
        }
        VFS.update(uri.to_file_path().unwrap(), code.clone());
        let token_stream = match Lexer::from_str(code.clone()).lex() {
            Ok(ts) => Some(ts),
            Err((ts, es)) => {
                lsp_log!("failed to lex: {es}");
                Some(ts)
            }
        };
        entry.code = code;
        entry.ver = params.text_document.version;
        entry.token_stream = token_stream;
        entry.comment_spans = None;
    }

    pub fn remove(&mut self, uri: &NormalizedUrl) {
        VFS.remove(uri.to_file_path().unwrap());
        self.files.borrow_mut().remove(uri);
        self.opened.borrow_mut().remove(uri);
    }

    pub fn rename_files(&mut self, params: &RenameFilesParams) -> ELSResult<()> {
        for file in &params.files {
            let Ok(old_uri) = NormalizedUrl::parse(&file.old_uri) else {
                lsp_log!("failed to parse old uri: {}", file.old_uri);
                continue;
            };
            let Ok(new_uri) = NormalizedUrl::parse(&file.new_uri) else {
                lsp_log!("failed to parse new uri: {}", file.new_uri);
                continue;
            };
            let Some(entry) = self.files.borrow_mut().remove(&old_uri) else {
                lsp_log!("failed to find old uri: {}", file.old_uri);
                continue;
            };
            VFS.rename(
                old_uri.to_file_path().unwrap(),
                new_uri.to_file_path().unwrap(),
            );
            let was_open = self.opened.borrow_mut().remove(&old_uri);
            if was_open {
                self.opened.borrow_mut().insert(new_uri.clone());
            }
            self.files.borrow_mut().insert(new_uri, entry);
        }
        Ok(())
    }

    pub fn entries(&self) -> Vec<NormalizedUrl> {
        self.files.borrow().keys().cloned().collect()
    }

    pub fn get_ver(&self, uri: &NormalizedUrl) -> Option<i32> {
        self.files.borrow().get(uri).map(|x| x.ver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_comment(code: &str, line: u32, character: u32) -> bool {
        comment_spans_of(code)
            .iter()
            .any(|span| span_contains(*span, Position::new(line, character)))
    }

    #[test]
    fn a_trailing_comment_covers_only_itself() {
        let code = "x = 1 # hi\ny = 2\n";
        assert!(
            !in_comment(code, 0, 5),
            "the code before `#` still completes"
        );
        assert!(in_comment(code, 0, 6), "`#` itself");
        assert!(in_comment(code, 0, 9));
        assert!(!in_comment(code, 0, 10), "past the end of the comment");
        assert!(!in_comment(code, 1, 0), "the next line is code");
    }

    #[test]
    fn a_block_comment_covers_its_interior_but_not_the_code_around_it() {
        let code = "x = 1 #[ a\nb\nc ]# + 2\n";
        assert!(!in_comment(code, 0, 5));
        assert!(in_comment(code, 0, 6));
        assert!(in_comment(code, 1, 0), "an interior line is all comment");
        assert!(in_comment(code, 2, 3), "up to `]#`");
        assert!(!in_comment(code, 2, 4), "` + 2` is code again");
    }

    #[test]
    fn a_doc_comment_covers_its_lines() {
        let code = "'''\ndoc\n'''\nx = 1\n";
        assert!(in_comment(code, 0, 0));
        assert!(in_comment(code, 1, 1));
        assert!(!in_comment(code, 3, 0));
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        let code = "x = \"# not a comment\"\n";
        assert!(!in_comment(code, 0, 5));
    }

    #[test]
    fn columns_are_utf16_units_not_chars() {
        // `𝒳` is one `char` but two UTF-16 code units, so the lexer's char
        // column 8 for `#` is column 9 to the client.
        let code = "x = \"\u{1D4B3}\" # hi\n";
        assert!(!in_comment(code, 0, 8), "still inside the string literal");
        assert!(in_comment(code, 0, 9), "`#` in the client's coordinates");
    }

    #[test]
    fn a_file_without_comments_has_no_spans() {
        assert!(comment_spans_of("x = 1\ny = 2\n").is_empty());
    }
}
