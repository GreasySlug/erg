//! `textDocument/moniker` — a stable identifier for the symbol under the cursor.

use erg_common::consts::PYTHON_MODE;
use erg_common::pathutil::NormalizedPathBuf;
use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;
use erg_compiler::varinfo::VarKind;
use lsp_types::{Moniker, MonikerKind, MonikerParams, UniquenessLevel};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::NormalizedUrl;

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_moniker(
        &mut self,
        params: MonikerParams,
    ) -> ELSResult<Option<Vec<Moniker>>> {
        _log!(self, "moniker requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document_position_params.text_document.uri);
        let pos = params.text_document_position_params.position;
        let Some(tok) = self.file_cache.get_symbol(&uri, pos) else {
            return Ok(None);
        };
        let Some(vi) = self
            .get_visitor(&uri)
            .and_then(|visitor| visitor.get_info(&tok))
        else {
            return Ok(None);
        };
        let current = uri.to_file_path().ok().map(NormalizedPathBuf::from);
        let imported = vi
            .def_loc
            .module
            .as_ref()
            .is_some_and(|def| current.as_ref().is_some_and(|cur| def != cur));
        let (kind, unique) = if vi.kind.is_builtin() {
            (MonikerKind::Export, UniquenessLevel::Scheme)
        } else if imported {
            (MonikerKind::Import, UniquenessLevel::Project)
        } else if vi.kind.is_parameter()
            || matches!(vi.kind, VarKind::Declared)
            || !vi.vis.is_public()
        {
            (MonikerKind::Local, UniquenessLevel::Document)
        } else {
            (MonikerKind::Export, UniquenessLevel::Project)
        };
        let scheme = if PYTHON_MODE { "pylyzer" } else { "erg" };
        Ok(Some(vec![Moniker {
            scheme: scheme.to_string(),
            identifier: vi.def_loc.to_string(),
            unique,
            kind: Some(kind),
        }]))
    }
}
