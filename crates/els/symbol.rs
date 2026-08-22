use std::collections::HashSet;

use erg_common::pathutil::project_entry_dir_of;
use erg_common::traits::Locational;
use erg_compiler::context::Context;

use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::ast::DefKind;
use erg_compiler::erg_parser::parse::Parsable;
use erg_compiler::hir::Expr;
use erg_compiler::ty::{HasType, Type};
use erg_compiler::varinfo::VarInfo;
use lsp_types::{
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, Location, SymbolInformation,
    SymbolKind, Url, WorkspaceSymbolParams,
};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::{abs_loc_to_lsp_loc, loc_to_range, NormalizedUrl};

pub(crate) fn symbol_kind(vi: &VarInfo) -> SymbolKind {
    match &vi.t {
        Type::Subr(subr) if subr.self_t().is_some() => SymbolKind::METHOD,
        Type::Quantified(quant) if quant.self_t().is_some() => SymbolKind::METHOD,
        Type::Subr(_) | Type::Quantified(_) => SymbolKind::FUNCTION,
        Type::ClassType => SymbolKind::CLASS,
        Type::TraitType => SymbolKind::INTERFACE,
        t if matches!(&t.qual_name()[..], "Module" | "PyModule" | "GenericModule") => {
            SymbolKind::MODULE
        }
        _ if vi.muty.is_const() => SymbolKind::CONSTANT,
        _ => SymbolKind::VARIABLE,
    }
}

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_workspace_symbol(
        &mut self,
        params: WorkspaceSymbolParams,
    ) -> ELSResult<Option<Vec<SymbolInformation>>> {
        _log!(self, "workspace symbol requested: {params:?}");
        let project_root = project_entry_dir_of(&self.home).unwrap_or(self.home.clone());
        let uris: Vec<_> = self
            .shared
            .raw_path_and_modules()
            .filter(|(path, _)| path.starts_with(&project_root))
            .filter_map(|(path, _)| NormalizedUrl::from_file_path(path.as_path()).ok())
            .collect();
        let mut res = vec![];
        let mut seen = HashSet::new();
        for nurl in uris {
            let uri = nurl.clone().raw();
            let module_name = nurl
                .to_file_path()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()));
            if let Some(hir) = self.get_hir(&nurl) {
                for chunk in hir.module.iter() {
                    if let Some(symbol) = self.symbol(chunk) {
                        flatten_workspace_symbol(
                            symbol,
                            &uri,
                            module_name.clone(),
                            &params.query,
                            &mut seen,
                            &mut res,
                        );
                    }
                }
                continue;
            }
            // Failed checks may still have a context with toplevel names.
            if let Some(mod_ctx) = self.get_mod_ctx(&nurl) {
                collect_context_workspace_symbols(
                    &mod_ctx.context,
                    &uri,
                    module_name.as_deref(),
                    &params.query,
                    &mut seen,
                    &mut res,
                );
            }
        }
        Ok(Some(res))
    }

    pub(crate) fn handle_document_symbol(
        &mut self,
        params: DocumentSymbolParams,
    ) -> ELSResult<Option<DocumentSymbolResponse>> {
        _log!(self, "document symbol requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document.uri);
        if let Some(hir) = self.get_hir(&uri) {
            let mut res = vec![];
            for chunk in hir.module.iter() {
                let symbol = self.symbol(chunk);
                res.extend(symbol);
            }
            return Ok(Some(DocumentSymbolResponse::Nested(res)));
        }
        Ok(None)
    }

    fn symbol(&self, chunk: &Expr) -> Option<DocumentSymbol> {
        match chunk {
            Expr::Def(def) => {
                if def.sig.is_glob() || def.sig.inspect().starts_with(['%']) {
                    return None;
                }
                let range = loc_to_range(def.loc())?;
                let selection_range = loc_to_range(def.sig.loc())?;
                #[allow(deprecated)]
                Some(DocumentSymbol {
                    name: def.sig.name().to_string(),
                    detail: Some(def.sig.ident().ref_t().to_string()),
                    kind: symbol_kind(&def.sig.ident().vi),
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range,
                    children: Some(self.child_symbols(chunk)),
                })
            }
            Expr::ClassDef(def) => {
                let range = loc_to_range(def.loc())?;
                let selection_range = loc_to_range(def.sig.loc())?;
                #[allow(deprecated)]
                Some(DocumentSymbol {
                    name: def.sig.name().to_string(),
                    detail: Some(def.sig.ident().ref_t().to_string()),
                    kind: symbol_kind(&def.sig.ident().vi),
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range,
                    children: Some(self.child_symbols(chunk)),
                })
            }
            Expr::PatchDef(def) => {
                let range = loc_to_range(def.loc())?;
                let selection_range = loc_to_range(def.sig.loc())?;
                #[allow(deprecated)]
                Some(DocumentSymbol {
                    name: def.sig.name().to_string(),
                    detail: Some(def.sig.ident().ref_t().to_string()),
                    kind: symbol_kind(&def.sig.ident().vi),
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range,
                    children: Some(self.child_symbols(chunk)),
                })
            }
            _ => None,
        }
    }

    fn child_symbols(&self, chunk: &Expr) -> Vec<DocumentSymbol> {
        match chunk {
            Expr::Def(def) => match def.def_kind() {
                DefKind::Class | DefKind::Trait => {
                    if let Some(base) = def.get_base() {
                        let mut res = vec![];
                        for member in base.attrs.iter() {
                            let symbol = self.symbol(&Expr::Def(member.clone()));
                            res.extend(symbol);
                        }
                        res
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            },
            Expr::ClassDef(def) => {
                let mut res = vec![];
                if let Some(Expr::Record(rec)) = def.require_or_sup.as_deref() {
                    for member in rec.attrs.iter() {
                        let symbol = self.symbol(&Expr::Def(member.clone()));
                        res.extend(symbol);
                    }
                }
                for method in def.all_methods() {
                    let symbol = self.symbol(method);
                    res.extend(symbol);
                }
                res
            }
            Expr::PatchDef(def) => {
                let mut res = vec![];
                for method in def.methods.iter() {
                    let symbol = self.symbol(method);
                    res.extend(symbol);
                }
                res
            }
            _ => vec![],
        }
    }
}

fn collect_context_workspace_symbols(
    ctx: &Context,
    uri: &Url,
    module_name: Option<&str>,
    query: &str,
    seen: &mut HashSet<String>,
    res: &mut Vec<SymbolInformation>,
) {
    for (name, vi) in ctx.local_dir() {
        if name.inspect().starts_with(['%']) {
            continue;
        }
        if !query.is_empty()
            && !name
                .inspect()
                .to_ascii_lowercase()
                .contains(&query.to_ascii_lowercase())
        {
            continue;
        }
        if vi
            .alias_of
            .as_ref()
            .is_some_and(|alias| &alias.name == name.inspect())
        {
            continue;
        }
        let Some(location) = abs_loc_to_lsp_loc(&vi.def_loc) else {
            continue;
        };
        if &location.uri != uri {
            continue;
        }
        let key = format!(
            "{}@{}:{}:{}",
            name.inspect(),
            uri,
            location.range.start.line,
            location.range.start.character
        );
        if !seen.insert(key) {
            continue;
        }
        #[allow(deprecated)]
        res.push(SymbolInformation {
            name: name.to_string(),
            location,
            kind: symbol_kind(vi),
            container_name: module_name.map(str::to_string),
            tags: None,
            deprecated: None,
        });
    }
}

fn flatten_workspace_symbol(
    symbol: DocumentSymbol,
    uri: &Url,
    container: Option<String>,
    query: &str,
    seen: &mut HashSet<String>,
    res: &mut Vec<SymbolInformation>,
) {
    if query.is_empty()
        || symbol
            .name
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
    {
        let key = format!(
            "{}@{}:{}:{}",
            symbol.name,
            uri,
            symbol.selection_range.start.line,
            symbol.selection_range.start.character
        );
        if seen.insert(key) {
            #[allow(deprecated)]
            res.push(SymbolInformation {
                name: symbol.name.clone(),
                location: Location::new(uri.clone(), symbol.selection_range),
                kind: symbol.kind,
                container_name: container.clone(),
                tags: symbol.tags.clone(),
                deprecated: symbol.deprecated,
            });
        }
    }
    let child_container = Some(symbol.name);
    for child in symbol.children.unwrap_or_default() {
        flatten_workspace_symbol(child, uri, child_container.clone(), query, seen, res);
    }
}
