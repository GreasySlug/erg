use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;
use lsp_types::request::{GotoImplementationParams, GotoImplementationResponse};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::{loc_to_pos, NormalizedUrl};

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_goto_implementation(
        &mut self,
        params: GotoImplementationParams,
    ) -> ELSResult<Option<GotoImplementationResponse>> {
        _log!(self, "implementation requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document_position_params.text_document.uri);
        let pos = params.text_document_position_params.position;
        let Some(symbol) = self.file_cache.get_symbol(&uri, pos) else {
            return Ok(None);
        };
        // {x #[ ← this is not the impl ]#;} = import "foo"
        // print! x # ← symbol
        if let Some(vi) = self.get_definition(&uri, &symbol)? {
            // classes that implement this trait / inherit from this class
            let impls = self.get_class_impls(&vi.def_loc);
            if !impls.is_empty() {
                return Ok(Some(GotoImplementationResponse::Array(impls)));
            }
            // no implementors: fall back to the definition itself
            let Some(def_uri) = vi
                .def_loc
                .module
                .clone()
                .and_then(|p| NormalizedUrl::from_file_path(p).ok())
            else {
                return Ok(None);
            };
            let Some(pos) = loc_to_pos(vi.def_loc.loc) else {
                return Ok(None);
            };
            if let Some(location) = self.get_definition_location(&def_uri, pos)? {
                return Ok(Some(GotoImplementationResponse::Scalar(location)));
            }
        }
        Ok(None)
    }
}
