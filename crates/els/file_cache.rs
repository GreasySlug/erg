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
    /// The byte offset each line starts at, memoized by [`FileCacheEntry::get_line`].
    /// `None` means "not computed for this `code` yet" -- every write to `code`
    /// must clear it, as for `comment_spans`. Every position that goes to or
    /// comes from the client is converted through a line of text, so a document
    /// symbol request would otherwise scan the file once per symbol.
    line_starts: Option<Vec<usize>>,
}

impl FileCacheEntry {
    fn new(code: String, ver: i32, token_stream: Option<TokenStream>) -> Self {
        Self {
            code,
            ver,
            token_stream,
            comment_spans: None,
            line_starts: None,
        }
    }

    /// The 0-based line `line0`, without its `\n` or a `\r` before it, like
    /// `str::lines` (so a file ending in `\n` has no empty line after it).
    pub fn get_line(&mut self, line0: u32) -> Option<String> {
        let starts = self.line_starts.get_or_insert_with(|| {
            std::iter::once(0)
                .chain(self.code.match_indices('\n').map(|(i, _)| i + 1))
                .collect()
        });
        let start = *starts.get(line0 as usize)?;
        if start >= self.code.len() {
            return None;
        }
        let end = starts
            .get(line0 as usize + 1)
            .map_or(self.code.len(), |next| next - 1);
        let line = &self.code[start..end];
        Some(line.strip_suffix('\r').unwrap_or(line).to_string())
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
pub(crate) fn utf16_col(line: &str, char_col: u32) -> u32 {
    let mut seen = 0;
    let mut chars = 0;
    for c in line.chars().take(char_col as usize) {
        seen += c.len_utf16() as u32;
        chars += 1;
    }
    // a column past the end keeps its excess, and `u32::MAX` is used as an
    // end-of-line sentinel, so the sum has to saturate too
    seen.saturating_add(char_col.saturating_sub(chars))
}

/// The `char` column of UTF-16 column `utf16_col` in `line`: the inverse of
/// [`utf16_col`].
///
/// A column inside a surrogate pair rounds up to the next `char`. A column past
/// the end of the line keeps its excess (`len + k` maps to `chars + k`), so a
/// position that was pushed past the end on purpose stays past the end.
fn char_col(line: &str, utf16_col: u32) -> u32 {
    let mut seen = 0;
    let mut chars = 0;
    for c in line.chars() {
        if seen >= utf16_col {
            return chars;
        }
        seen += c.len_utf16() as u32;
        chars += 1;
    }
    chars + utf16_col.saturating_sub(seen)
}

/// The byte offset of UTF-16 column `col` in `line`, or `None` when the line is
/// shorter than that.
///
/// A column inside a surrogate pair rounds up to the next `char` boundary --
/// the callers slice `line` with it, and `str` indexing panics on anything else.
fn utf16_col_to_byte(line: &str, col: u32) -> Option<usize> {
    let mut seen = 0;
    for (index, c) in line.char_indices() {
        if seen >= col {
            return Some(index);
        }
        seen += c.len_utf16() as u32;
    }
    (seen >= col).then_some(line.len())
}

/// The text of `code` between the two LSP positions, or `None` when a line is
/// shorter than the column the range asks for.
///
/// `Range` columns count UTF-16 code units while `str` is indexed in bytes, so
/// every column has to go through [`utf16_col_to_byte`] first: on a line holding
/// any non-ASCII character the two disagree, and slicing at a byte offset that
/// lands inside a character panics. `None` is what tells `code_action` that the
/// character it asked for past the end of a definition is the newline.
fn ranged(code: &str, range: Range) -> Option<String> {
    let mut out = String::new();
    for (i, line) in code.lines().enumerate() {
        if i < range.start.line as usize || i > range.end.line as usize {
            continue;
        }
        let starts = i == range.start.line as usize;
        let ends = i == range.end.line as usize;
        // A start column past the end of its line contributes nothing, rather
        // than failing the whole range: only `end` decides `None`, as before.
        let from = if starts {
            utf16_col_to_byte(line, range.start.character).unwrap_or(line.len())
        } else {
            0
        };
        let to = if ends {
            utf16_col_to_byte(line, range.end.character)?
        } else {
            line.len()
        };
        out.push_str(line.get(from..to).unwrap_or_default());
        if !ends {
            out.push('\n');
        }
    }
    Some(out)
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

    /// The LSP position (UTF-16 column) of the Erg position `pos` (`char`
    /// column, as the lexer counts) in `uri`.
    ///
    /// The two agree until the line holds a character outside the BMP, which
    /// is one `char` but two UTF-16 units. Every `Location` that goes out in a
    /// response has to pass through here (or [`Server::loc_to_range`]). When
    /// the line cannot be read the column is returned as it is.
    pub(crate) fn to_lsp_pos(&self, uri: &NormalizedUrl, pos: Position) -> Position {
        match self.line_for_conversion(uri, pos.line) {
            Some(line) => Position::new(pos.line, utf16_col(&line, pos.character)),
            None => pos,
        }
    }

    /// The line a column conversion looks at: only in an Erg source. A `.py` or
    /// `.pyi` module's declarations carry the columns of the declaration text
    /// generated from it, not of the file, and running the lexer over a Python
    /// file just to count a line's characters is not worth it either.
    fn line_for_conversion(&self, uri: &NormalizedUrl, line: u32) -> Option<String> {
        let path = uri.to_file_path().ok()?;
        let is_erg = path.extension().is_some_and(|ext| ext == "er");
        is_erg.then(|| self.get_line(uri, line)).flatten()
    }

    pub(crate) fn to_lsp_range(&self, uri: &NormalizedUrl, range: Range) -> Range {
        Range::new(
            self.to_lsp_pos(uri, range.start),
            self.to_lsp_pos(uri, range.end),
        )
    }

    /// The Erg position (`char` column) of the LSP position `pos` (UTF-16
    /// column) in `uri`: the inverse of [`Self::to_lsp_pos`]. Compare the result
    /// with token and HIR locations, never the LSP position itself.
    pub(crate) fn to_erg_pos(&self, uri: &NormalizedUrl, pos: Position) -> Position {
        match self.line_for_conversion(uri, pos.line) {
            Some(line) => Position::new(pos.line, char_col(&line, pos.character)),
            None => pos,
        }
    }

    /// The token under the LSP position `pos`.
    pub fn get_token(&self, uri: &NormalizedUrl, pos: Position) -> Option<Token> {
        let pos = self.to_erg_pos(uri, pos);
        self.token_at(uri, pos)
    }

    /// `erg_pos` counts `char`s, like the tokens it is compared with.
    fn token_at(&self, uri: &NormalizedUrl, erg_pos: Position) -> Option<Token> {
        let _ = self.load_once(uri);
        let ent = self.files.borrow_mut();
        let tokens = ent.get(uri)?.token_stream.as_ref()?;
        for tok in tokens.iter() {
            if util::pos_in_loc(tok, erg_pos) {
                return Some(tok.clone());
            }
        }
        for tok in tokens.iter() {
            if util::roughly_pos_in_loc(tok, erg_pos) {
                return Some(tok.clone());
            }
        }
        None
    }

    /// a{pos}\n -> \n -> a
    pub fn get_symbol(&self, uri: &NormalizedUrl, pos: Position) -> Option<Token> {
        let pos = self.to_erg_pos(uri, pos);
        let mut token = self.token_at(uri, pos)?;
        let mut offset = 0;
        while !matches!(token.category(), TokenCategory::Symbol) {
            offset -= 1;
            token = self.token_relative_to(uri, pos, offset)?;
        }
        Some(token)
    }

    pub fn get_receiver(&self, uri: &NormalizedUrl, attr_marker_pos: Position) -> Option<Token> {
        let attr_marker_pos = self.to_erg_pos(uri, attr_marker_pos);
        let mut token = self.token_at(uri, attr_marker_pos)?;
        let mut offset = 0;
        while !matches!(token.kind, TokenKind::Dot | TokenKind::DblColon) {
            offset -= 1;
            token = self.token_relative_to(uri, attr_marker_pos, offset)?;
        }
        offset -= 1;
        self.token_relative_to(uri, attr_marker_pos, offset)
    }

    /// The token `offset` tokens away from the one under the LSP position `pos`.
    pub fn get_token_relatively(
        &self,
        uri: &NormalizedUrl,
        pos: Position,
        offset: isize,
    ) -> Option<Token> {
        let pos = self.to_erg_pos(uri, pos);
        self.token_relative_to(uri, pos, offset)
    }

    fn token_relative_to(
        &self,
        uri: &NormalizedUrl,
        erg_pos: Position,
        offset: isize,
    ) -> Option<Token> {
        if offset == 0 {
            return self.token_at(uri, erg_pos);
        }
        let _ = self.load_once(uri);
        let ent = self.files.borrow_mut();
        let tokens = ent.get(uri)?.token_stream.as_ref()?;
        let index = (|| {
            for (i, tok) in tokens.iter().enumerate() {
                if util::pos_in_loc(tok, erg_pos) {
                    return Some(i);
                }
            }
            for (i, tok) in tokens.iter().enumerate() {
                if util::roughly_pos_in_loc(tok, erg_pos) {
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
        self.files.borrow_mut().get_mut(uri)?.get_line(line0)
    }

    pub(crate) fn get_ranged(
        &self,
        uri: &NormalizedUrl,
        range: Range,
    ) -> ELSResult<Option<String>> {
        self.load_once(uri)?;
        let ent = self.files.borrow_mut();
        let file = ent.get(uri).ok_or("file entry not found")?;
        Ok(ranged(&file.code, range))
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
        entry.line_starts = None;
    }

    /// Applies a `textDocument/didChange`. Returns `false` when a change could
    /// not be applied and the cached text is therefore no longer what the client
    /// has; [`FileCache::resync_from_disk`] is the caller's way out.
    ///
    /// Nothing here may panic. This runs on the server's message loop, so a
    /// panic kills it, and the supervisor's in-process restart keeps the pre-crash
    /// text -- every later change is then spliced into a buffer the client never
    /// had, and completions built from it paste that stale text back into the
    /// document.
    pub(crate) fn incremental_update(&self, params: DidChangeTextDocumentParams) -> bool {
        let uri = NormalizedUrl::new(params.text_document.uri);
        let mut ent = self.files.borrow_mut();
        let Some(entry) = ent.get_mut(&uri) else {
            return true;
        };
        if entry.ver >= params.text_document.version {
            crate::_log!(
                self,
                "212: double update detected {}, {}, code:\n{}",
                entry.ver,
                params.text_document.version,
                entry.code
            );
            return true;
        }
        let mut code = entry.code.clone();
        for change in params.content_changes {
            let Some(range) = change.range else {
                // No range: this change is the whole document.
                code = change.text;
                continue;
            };
            let start = util::pos_to_byte_index(&code, range.start);
            let end = util::pos_to_byte_index(&code, range.end);
            if start > end {
                lsp_log!("inverted change range {range:?} for {uri}; buffer is out of sync");
                return false;
            }
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
        entry.line_starts = None;
        true
    }

    /// Re-reads every cached file from disk, dropping the entries whose file is
    /// gone.
    ///
    /// The recovery path after the message loop dies: the process itself stays
    /// up, so the client never re-sends `didOpen` and the cache would otherwise
    /// keep whatever text it held when the loop died. Disk content can still be
    /// behind an editor with unsaved edits, but it is text the user actually
    /// wrote, and the next `didOpen` or `didSave` makes it exact again.
    pub fn resync_from_disk(&self) {
        for uri in self.entries() {
            if let Err(err) = self.reload_from_disk(&uri) {
                lsp_log!("failed to resync {uri}: {err}");
                self.files.borrow_mut().remove(&uri);
            }
        }
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

    fn at(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
        Range::new(Position::new(sl, sc), Position::new(el, ec))
    }

    #[test]
    fn a_range_is_taken_in_utf16_columns() {
        let code = "one = 1\ntwo = 2\n";
        assert_eq!(ranged(code, at(0, 0, 0, 3)).as_deref(), Some("one"));
        assert_eq!(ranged(code, at(0, 6, 1, 3)).as_deref(), Some("1\ntwo"));
        // the same, one UTF-16 column per Japanese character
        let code = "おは = 1\nいろは = 2\n";
        assert_eq!(ranged(code, at(0, 0, 0, 2)).as_deref(), Some("おは"));
        assert_eq!(ranged(code, at(1, 0, 1, 3)).as_deref(), Some("いろは"));
        assert_eq!(ranged(code, at(0, 5, 1, 3)).as_deref(), Some("1\nいろは"));
    }

    /// A magic completion pastes this text back into the document, so an
    /// off-by-bytes slice is how the buffer's Japanese ends up auto-inserted.
    #[test]
    fn char_and_utf16_columns_convert_both_ways() {
        // `x = "𝒳", y`: the `𝒳` is char 5 and UTF-16 units 5-6, so `y` is
        // char 9 but UTF-16 column 10.
        let line = "x = \"𝒳\", y";
        assert_eq!(utf16_col(line, 9), 10);
        assert_eq!(char_col(line, 10), 9);
        // a plain line converts to itself
        assert_eq!(utf16_col("x = 1", 3), 3);
        assert_eq!(char_col("x = 1", 3), 3);
        // inside the surrogate pair: round up to the next char
        assert_eq!(char_col(line, 6), 6);
        // past the end of the line the excess is kept
        assert_eq!(utf16_col(line, 12), 13);
        assert_eq!(char_col(line, 13), 12);
        assert_eq!(char_col(line, u32::MAX), u32::MAX - 1);
    }

    #[test]
    fn a_receiver_after_a_japanese_line_is_not_shifted() {
        let code = "お = 1\nお.\n";
        assert_eq!(ranged(code, at(1, 0, 1, 1)).as_deref(), Some("お"));
        // `one` sits at columns 4..7 of a line whose bytes start three later
        let code = "お = one.\n";
        assert_eq!(ranged(code, at(0, 4, 0, 7)).as_deref(), Some("one"));
    }

    #[test]
    fn a_range_past_the_end_of_its_line_is_none() {
        // `code_action` reads this as "the character after the def is `\n`"
        let code = "x = 1\ny = 2\n";
        assert_eq!(ranged(code, at(0, 5, 0, 6)), None);
        assert_eq!(ranged(code, at(0, 5, 0, 5)).as_deref(), Some(""));
        // a line of Japanese is 1 column per character, not 3
        let code = "おは\n";
        assert_eq!(ranged(code, at(0, 0, 0, 3)), None);
        assert_eq!(ranged(code, at(0, 0, 0, 2)).as_deref(), Some("おは"));
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use lsp_types::{TextDocumentContentChangeEvent, VersionedTextDocumentIdentifier};

    fn cache(name: &str, code: &str) -> (FileCache, NormalizedUrl) {
        let cache = FileCache::new(None);
        let uri = NormalizedUrl::from_file_path(format!("/tmp/els_update_{name}.er")).unwrap();
        cache.update(&uri, code.to_string(), Some(1));
        (cache, uri)
    }

    #[test]
    fn positions_convert_between_char_and_utf16_columns() {
        let (cache, uri) = cache("astral", "y = 1\npair = (\"𝒳\", y)\n");
        // `y` on the second line: char 13, UTF-16 unit 14
        assert_eq!(
            cache.to_erg_pos(&uri, Position::new(1, 14)),
            Position::new(1, 13)
        );
        assert_eq!(
            cache.to_lsp_pos(&uri, Position::new(1, 13)),
            Position::new(1, 14)
        );
        // nothing to convert on an ASCII line
        assert_eq!(
            cache.to_erg_pos(&uri, Position::new(0, 4)),
            Position::new(0, 4)
        );
        // the token lookup takes the LSP column
        let token = cache.get_token(&uri, Position::new(1, 14)).unwrap();
        assert_eq!(&token.content[..], "y");
        // a line the file does not have: the column is kept
        assert_eq!(
            cache.to_lsp_pos(&uri, Position::new(9, 3)),
            Position::new(9, 3)
        );
        // a Python file's columns are not the lexer's: nothing to convert
        let py = NormalizedUrl::from_file_path("/tmp/els_update_astral.py").unwrap();
        cache.update(&py, "pair = (\"𝒳\", y)\n".to_string(), Some(1));
        assert_eq!(
            cache.to_lsp_pos(&py, Position::new(0, 13)),
            Position::new(0, 13)
        );
    }

    fn edit(
        uri: &NormalizedUrl,
        ver: i32,
        range: Range,
        text: &str,
    ) -> DidChangeTextDocumentParams {
        DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone().raw(),
                version: ver,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: Some(range),
                range_length: None,
                text: text.to_string(),
            }],
        }
    }

    /// Typing on past a multi-byte character at the very end of the buffer.
    #[test]
    fn typing_after_a_japanese_character_at_eof() {
        let (cache, uri) = cache("eof", "one = 1\nお");
        assert!(cache.incremental_update(edit(
            &uri,
            2,
            Range::new(Position::new(1, 1), Position::new(1, 1)),
            "あ",
        )));
        assert_eq!(cache.get_entire_code(&uri).unwrap(), "one = 1\nおあ");
    }

    #[test]
    fn an_edit_inside_a_japanese_line_lands_on_the_right_character() {
        let (cache, uri) = cache("mid", "おはよう = 1\n");
        // replace `よ` (column 2) with `Y`
        assert!(cache.incremental_update(edit(
            &uri,
            2,
            Range::new(Position::new(0, 2), Position::new(0, 3)),
            "Y",
        )));
        assert_eq!(cache.get_entire_code(&uri).unwrap(), "おはYう = 1\n");
    }

    #[test]
    fn a_full_document_change_replaces_the_buffer() {
        let (cache, uri) = cache("full", "お\n");
        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone().raw(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "x = 1\n".to_string(),
            }],
        };
        assert!(cache.incremental_update(params));
        assert_eq!(cache.get_entire_code(&uri).unwrap(), "x = 1\n");
    }

    /// What the supervisor does after the message loop dies: the entry it left
    /// behind is not what the client has, and the client will not re-send it.
    #[test]
    fn a_resync_replaces_a_stale_buffer_with_what_is_on_disk() {
        let path = std::env::temp_dir().join("els_resync_test.er");
        std::fs::write(&path, "x = 1\n").unwrap();
        let uri = NormalizedUrl::from_file_path(&path).unwrap();
        let cache = FileCache::new(None);
        cache.update(&uri, "お = 1\n".to_string(), Some(7));

        cache.resync_from_disk();
        assert_eq!(cache.get_entire_code(&uri).unwrap(), "x = 1\n");
        // the client keeps counting from where it was, so its next edit applies
        assert!(cache.incremental_update(edit(
            &uri,
            8,
            Range::new(Position::new(0, 5), Position::new(0, 5)),
            "0",
        )));
        assert_eq!(cache.get_entire_code(&uri).unwrap(), "x = 10\n");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_inverted_range_is_refused_rather_than_applied() {
        let (cache, uri) = cache("inverted", "one = 1\n");
        assert!(!cache.incremental_update(edit(
            &uri,
            2,
            Range::new(Position::new(0, 5), Position::new(0, 2)),
            "x",
        )));
        assert_eq!(cache.get_entire_code(&uri).unwrap(), "one = 1\n");
    }
}
