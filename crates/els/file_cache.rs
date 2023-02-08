use std::fs::{metadata, Metadata};

use lsp_types::{DidChangeTextDocumentParams, Position, Url};

use erg_common::dict::Dict;
use erg_common::shared::Shared;
use erg_common::traits::DequeStream;
use erg_compiler::erg_parser::lex::Lexer;
use erg_compiler::erg_parser::token::{Token, TokenKind, TokenStream};

use crate::server::ELSResult;
use crate::util;

#[derive(Debug, Clone)]
pub struct FileCacheEntry {
    pub code: String,
    pub metadata: Metadata,
    pub token_stream: Option<TokenStream>,
}

#[derive(Debug, Clone)]
pub struct FileCache {
    pub files: Shared<Dict<Url, FileCacheEntry>>,
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            files: Shared::new(Dict::new()),
        }
    }

    pub fn get(&self, uri: &Url) -> ELSResult<&FileCacheEntry> {
        let Some(entry) = unsafe { self.files.as_ref() }.get(uri) else {
            let code = util::get_code_from_uri(uri)?;
            self.update(uri, code);
            let entry = unsafe { self.files.as_ref() }.get(uri).ok_or("not found")?;
            return Ok(entry);
        };
        let last_modified = entry.metadata.modified().unwrap();
        let current_modified = metadata(uri.to_file_path().unwrap())
            .unwrap()
            .modified()
            .unwrap();
        if last_modified != current_modified {
            let code = util::get_code_from_uri(uri)?;
            self.update(uri, code);
            unsafe { self.files.as_ref() }
                .get(uri)
                .ok_or("not found".into())
        } else {
            let entry = unsafe { self.files.as_ref() }.get(uri).ok_or("not found")?;
            Ok(entry)
        }
    }

    pub fn get_token_index(&self, uri: &Url, pos: Position) -> ELSResult<Option<usize>> {
        let tokens = self.get(uri)?.token_stream.as_ref().ok_or("lex error")?;
        for (i, tok) in tokens.iter().enumerate() {
            if util::pos_in_loc(tok, pos) {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    pub fn get_token(&self, uri: &Url, pos: Position) -> ELSResult<Option<Token>> {
        let tokens = self.get(uri)?.token_stream.as_ref().ok_or("lex error")?;
        for tok in tokens.iter() {
            if util::pos_in_loc(tok, pos) {
                return Ok(Some(tok.clone()));
            }
        }
        Ok(None)
    }

    pub fn get_token_relatively(
        &self,
        uri: &Url,
        pos: Position,
        offset: isize,
    ) -> ELSResult<Option<Token>> {
        let Some(index) = self.get_token_index(uri, pos)? else {
            let tokens = self.get(uri)?.token_stream.as_ref().ok_or("lex error")?;
            while let Some(token) = tokens.iter().next_back(){
                match token.kind {
                    TokenKind::EOF | TokenKind::Dedent | TokenKind::Newline => {}
                    _ => {
                        return Ok(Some(token.clone()))
                    }
                }
            }
            return Ok(None);
        };
        let index = (index as isize + offset) as usize;
        let tokens = self.get(uri)?.token_stream.as_ref().ok_or("lex error")?;
        if index < tokens.len() {
            Ok(Some(tokens[index].clone()))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn update(&self, uri: &Url, code: String) {
        let metadata = metadata(uri.to_file_path().unwrap()).unwrap();
        let token_stream = Lexer::from_str(code.clone()).lex().ok();
        self.files.borrow_mut().insert(
            uri.clone(),
            FileCacheEntry {
                code,
                metadata,
                token_stream,
            },
        );
    }

    pub(crate) fn incremental_update(&self, params: DidChangeTextDocumentParams) {
        let uri = util::normalize_url(params.text_document.uri);
        let Some(entry) = unsafe { self.files.as_mut() }.get_mut(&uri) else {
            return;
        };
        let metadata = metadata(uri.to_file_path().unwrap()).unwrap();
        let mut code = entry.code.clone();
        for change in params.content_changes {
            let range = change.range.unwrap();
            let start = util::pos_to_index(&code, range.start);
            let end = util::pos_to_index(&code, range.end);
            code.replace_range(start..end, &change.text);
        }
        let token_stream = Lexer::from_str(code.clone()).lex().ok();
        entry.code = code;
        entry.metadata = metadata;
        entry.token_stream = token_stream;
    }

    #[allow(unused)]
    pub fn remove(&mut self, uri: &Url) {
        self.files.borrow_mut().remove(uri);
    }
}
