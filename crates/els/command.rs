use erg_compiler::erg_parser::parse::Parsable;
use erg_compiler::varinfo::AbsLocation;
use serde_json::Value;

use erg_common::lsp_log;
use erg_compiler::artifact::BuildRunnable;
use erg_compiler::hir::Expr;

use lsp_types::{Command, ExecuteCommandParams, Location, Url};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::{self, NormalizedUrl};

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_execute_command(
        &mut self,
        params: ExecuteCommandParams,
    ) -> ELSResult<Option<Value>> {
        _log!(self, "command requested: {}", params.command);
        let force_shutdown_cmd = format!("{}.forceShutdown", self.mode());
        let restart_server_cmd = format!("{}.restartServer", self.mode());
        match &params.command[..] {
            cmd if cmd == force_shutdown_cmd => {
                lsp_log!("Force shutdown requested via workspace/executeCommand");
                std::process::exit(1);
            }
            cmd if cmd == restart_server_cmd => {
                lsp_log!("Restart requested via workspace/executeCommand");
                self.restart();
                Ok(None)
            }
            other => {
                _log!(self, "unknown command {other}: {params:?}");
                Ok(None)
            }
        }
    }

    pub(crate) fn gen_show_trait_impls_command(
        &self,
        trait_loc: AbsLocation,
    ) -> ELSResult<Option<Command>> {
        self.gen_show_class_refs_command(trait_loc, "implementations", false)
    }

    /// Build an `erg.showReferences` command listing the class definitions that
    /// reference `referee` (trait implementors, or subclasses of a class).
    /// `noun` is the title noun (e.g. "implementations"/"subclasses"); when
    /// `hide_when_empty` is set, returns `None` if there are no such classes
    /// (so a code lens is not shown on every class without subclasses).
    /// References to `referee` that are class definitions, i.e. the classes that
    /// implement a trait or inherit from a class.
    pub(crate) fn get_class_impls(&self, referee: &AbsLocation) -> Vec<Location> {
        self.get_refs_from_abs_loc(referee)
            .into_iter()
            .filter_map(|loc| {
                let uri = NormalizedUrl::new(loc.uri.clone());
                let visitor = self.get_visitor(&uri)?;
                let Expr::ClassDef(class_def) = visitor.get_min_expr(loc.range.start)? else {
                    return None;
                };
                // exclude self-references inside the class's own body
                // (e.g. `C { .x = x }` in `C`'s own constructor)
                (&class_def.sig.ident().vi.def_loc != referee).then_some(loc)
            })
            .collect()
    }

    pub(crate) fn gen_show_class_refs_command(
        &self,
        referee: AbsLocation,
        noun: &str,
        hide_when_empty: bool,
    ) -> ELSResult<Option<Command>> {
        let impls = self.get_class_impls(&referee);
        let impl_len = impls.len();
        if hide_when_empty && impl_len == 0 {
            return Ok(None);
        }
        let locations = serde_json::to_value(impls)?;
        let Ok(uri) = referee.module.ok_or(()).and_then(Url::from_file_path) else {
            return Ok(None);
        };
        let uri = serde_json::to_value(uri)?;
        let Some(position) = util::loc_to_pos(referee.loc) else {
            return Ok(None);
        };
        let position = serde_json::to_value(position)?;
        Ok(Some(Command {
            title: format!("{impl_len} {noun}"),
            // the command is defined in: https://github.com/erg-lang/vscode-erg/blob/20e6e2154b045ab56fedbc8769d03633acfd12e0/src/extension.ts#L92-L94
            command: "erg.showReferences".to_string(),
            arguments: Some(vec![uri, position, locations]),
        }))
    }
}
