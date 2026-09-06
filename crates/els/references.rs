use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;
use erg_compiler::varinfo::AbsLocation;

use lsp_types::{
    LinkedEditingRangeParams, LinkedEditingRanges, Location, Position, Range, ReferenceParams, Url,
};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::NormalizedUrl;

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_references(
        &mut self,
        params: ReferenceParams,
    ) -> ELSResult<Option<Vec<Location>>> {
        _log!(self, "references: {params:?}");
        let uri = NormalizedUrl::new(params.text_document_position.text_document.uri);
        let pos = params.text_document_position.position;
        let result = self.show_refs_inner(&uri, pos);
        Ok(Some(result))
    }

    fn show_refs_inner(&self, uri: &NormalizedUrl, pos: Position) -> Vec<lsp_types::Location> {
        if let Some(tok) = self.file_cache.get_symbol(uri, pos) {
            if let Some(visitor) = self.get_visitor(uri) {
                if let Some(vi) = visitor.get_info(&tok) {
                    return self.get_refs_from_abs_loc(&vi.def_loc);
                }
            }
        }
        vec![]
    }

    pub(crate) fn get_refs_from_abs_loc(&self, referee: &AbsLocation) -> Vec<lsp_types::Location> {
        let mut refs = vec![];
        if let Some(value) = self.shared.index.get_refs(referee) {
            if value.vi.def_loc == AbsLocation::unknown() {
                return vec![];
            }
            for referrer in value.referrers.iter() {
                if let (Some(path), Some(range)) =
                    (&referrer.module, self.abs_loc_to_range(referrer))
                {
                    let Ok(ref_uri) = Url::from_file_path(path) else {
                        continue;
                    };
                    refs.push(lsp_types::Location::new(ref_uri, range));
                }
            }
        }
        refs
    }

    pub(crate) fn handle_linked_editing_range(
        &mut self,
        params: LinkedEditingRangeParams,
    ) -> ELSResult<Option<LinkedEditingRanges>> {
        _log!(self, "linked editing range requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document_position_params.text_document.uri);
        let pos = params.text_document_position_params.position;
        Ok(self.linked_editing_ranges(&uri, pos))
    }

    fn linked_editing_ranges(
        &self,
        uri: &NormalizedUrl,
        pos: Position,
    ) -> Option<LinkedEditingRanges> {
        let tok = self.file_cache.get_symbol(uri, pos)?;
        let visitor = self.get_visitor(uri)?;
        let vi = visitor.get_info(&tok)?;
        let mut ranges: Vec<Range> = Vec::new();
        if let Some(path) = &vi.def_loc.module {
            if let Ok(def_uri) = Url::from_file_path(path) {
                if NormalizedUrl::new(def_uri) == *uri {
                    if let Some(range) = self.loc_to_range(uri, vi.def_loc.loc) {
                        ranges.push(range);
                    }
                }
            }
        }
        for loc in self.get_refs_from_abs_loc(&vi.def_loc) {
            if NormalizedUrl::new(loc.uri) == *uri {
                ranges.push(loc.range);
            }
        }
        ranges.sort_by_key(|r| (r.start.line, r.start.character, r.end.line, r.end.character));
        ranges.dedup();
        if ranges.is_empty() {
            return None;
        }
        let width = |r: &Range| {
            if r.start.line == r.end.line {
                r.end.character.saturating_sub(r.start.character)
            } else {
                u32::MAX
            }
        };
        let expected = self
            .loc_to_range(uri, tok.loc())
            .map(|r| width(&r))
            .unwrap_or_else(|| width(&ranges[0]));
        ranges.retain(|r| width(r) == expected);
        if ranges.is_empty() {
            return None;
        }
        Some(LinkedEditingRanges {
            ranges,
            word_pattern: Some(String::from(r"[A-Za-z_][A-Za-z0-9_]*!?")),
        })
    }
}
