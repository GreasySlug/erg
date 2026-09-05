use std::collections::HashMap;

use erg_compiler::erg_parser::parse::Parsable;
use erg_compiler::varinfo::AbsLocation;
use serde_json::Value;

use erg_common::lsp_log;
use erg_compiler::artifact::BuildRunnable;

use lsp_types::request::{ApplyWorkspaceEdit, Request};
use lsp_types::{
    ApplyWorkspaceEditParams, Command, ExecuteCommandParams, Location, TextEdit, Url, WorkspaceEdit,
};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::NormalizedUrl;

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_execute_command(
        &mut self,
        params: ExecuteCommandParams,
    ) -> ELSResult<Option<Value>> {
        _log!(self, "command requested: {}", params.command);
        let force_shutdown_cmd = format!("{}.forceShutdown", self.mode());
        let restart_server_cmd = format!("{}.restartServer", self.mode());
        let eliminate_unused_cmd = format!("{}.eliminate_unused_vars", self.mode());
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
            cmd if cmd == eliminate_unused_cmd => self.execute_eliminate_unused_vars(&params),
            other => {
                _log!(self, "unknown command {other}: {params:?}");
                Ok(None)
            }
        }
    }

    fn execute_eliminate_unused_vars(
        &self,
        params: &ExecuteCommandParams,
    ) -> ELSResult<Option<Value>> {
        let Some(targets) = self.command_target_uris(&params.arguments) else {
            self.send_error_info(
                "eliminate_unused_vars: pass the document URI to rewrite, \
                 or \"workspace\" to rewrite every open buffer",
            )?;
            return Ok(None);
        };
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for uri in targets {
            let Some(map) = self.unused_var_edits(&uri)? else {
                continue;
            };
            for (url, edits) in map {
                if !edits.is_empty() {
                    changes.entry(url).or_default().extend(edits);
                }
            }
        }
        if changes.is_empty() {
            _log!(self, "eliminate_unused_vars: no edits");
            return Ok(None);
        }
        let label = match changes.len() {
            1 => "Eliminate unused variables".to_string(),
            n => format!("Eliminate unused variables ({n} files)"),
        };
        self.send_client_request(
            ApplyWorkspaceEdit::METHOD,
            ApplyWorkspaceEditParams {
                label: Some(label),
                edit: WorkspaceEdit::new(changes),
            },
        )?;
        Ok(None)
    }

    /// Which documents the command rewrites. `arguments[0]` is either a document
    /// URI (string or `{uri}`) from the current editor, or `"workspace"` /
    /// `{"scope": "workspace"}` for every open buffer.
    ///
    /// `None` means "no target": rewriting every open buffer is too large a
    /// blast radius to infer from a bare, argument-less invocation, so that has
    /// to be asked for.
    fn command_target_uris(&self, args: &[Value]) -> Option<Vec<NormalizedUrl>> {
        let arg = args.first()?;
        if let Some(uri) = uri_from_command_arg(arg) {
            return Some(vec![uri]);
        }
        let scope = arg
            .as_str()
            .or_else(|| arg.get("scope").and_then(|v| v.as_str()))?;
        (scope == "workspace").then(|| self.file_cache.entries())
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
                let class_def = visitor.get_class_def_at(loc.range.start)?;
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
        let Some(position) = self.loc_to_pos(&NormalizedUrl::new(uri.clone()), referee.loc) else {
            return Ok(None);
        };
        let uri = serde_json::to_value(uri)?;
        let position = serde_json::to_value(position)?;
        Ok(Some(Command {
            title: format!("{impl_len} {noun}"),
            // the command is defined in: https://github.com/erg-lang/vscode-erg/blob/20e6e2154b045ab56fedbc8769d03633acfd12e0/src/extension.ts#L92-L94
            command: "erg.showReferences".to_string(),
            arguments: Some(vec![uri, position, locations]),
        }))
    }
}

fn uri_from_command_arg(arg: &Value) -> Option<NormalizedUrl> {
    if let Some(s) = arg.as_str() {
        return Url::parse(s).ok().map(NormalizedUrl::new);
    }
    arg.get("uri")
        .and_then(|v| v.as_str())
        .and_then(|s| Url::parse(s).ok())
        .map(NormalizedUrl::new)
}
