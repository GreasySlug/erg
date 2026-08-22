//! Type hierarchy (`textDocument/prepareTypeHierarchy`, `typeHierarchy/supertypes`,
//! `typeHierarchy/subtypes`).
//!
//! `lsp-types` 0.93 does not ship these LSP 3.17 types, so they are defined here.

use std::path::Path;
use std::str::FromStr;

use erg_common::shared::MappedRwLockReadGuard;
use erg_compiler::artifact::BuildRunnable;
use erg_compiler::context::ModuleContext;
use erg_compiler::erg_parser::parse::Parsable;
use erg_compiler::ty::Type;
use erg_compiler::varinfo::AbsLocation;
use lsp_types::request::Request;
use lsp_types::{
    PartialResultParams, SymbolKind, TextDocumentPositionParams, WorkDoneProgressParams,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::symbol::symbol_kind;
use crate::util::{abs_loc_to_lsp_loc, NormalizedUrl};

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchyItem {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<lsp_types::SymbolTag>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub uri: lsp_types::Url,
    pub range: lsp_types::Range,
    pub selection_range: lsp_types::Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchyPrepareParams {
    #[serde(flatten)]
    pub text_document_position_params: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done_progress_params: WorkDoneProgressParams,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchySupertypesParams {
    pub item: TypeHierarchyItem,
    #[serde(flatten)]
    pub work_done_progress_params: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: PartialResultParams,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchySubtypesParams {
    pub item: TypeHierarchyItem,
    #[serde(flatten)]
    pub work_done_progress_params: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: PartialResultParams,
}

#[derive(Debug)]
pub enum TypeHierarchyPrepare {}

impl Request for TypeHierarchyPrepare {
    type Params = TypeHierarchyPrepareParams;
    type Result = Option<Vec<TypeHierarchyItem>>;
    const METHOD: &'static str = "textDocument/prepareTypeHierarchy";
}

#[derive(Debug)]
pub enum TypeHierarchySupertypes {}

impl Request for TypeHierarchySupertypes {
    type Params = TypeHierarchySupertypesParams;
    type Result = Option<Vec<TypeHierarchyItem>>;
    const METHOD: &'static str = "typeHierarchy/supertypes";
}

#[derive(Debug)]
pub enum TypeHierarchySubtypes {}

impl Request for TypeHierarchySubtypes {
    type Params = TypeHierarchySubtypesParams;
    type Result = Option<Vec<TypeHierarchyItem>>;
    const METHOD: &'static str = "typeHierarchy/subtypes";
}

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_type_hierarchy_prepare(
        &mut self,
        params: TypeHierarchyPrepareParams,
    ) -> ELSResult<Option<Vec<TypeHierarchyItem>>> {
        _log!(self, "type hierarchy prepare requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document_position_params.text_document.uri);
        let pos = params.text_document_position_params.position;
        Ok(self.type_at(&uri, pos).and_then(|typ| {
            let item = self.item_from_type(&uri, &typ)?;
            Some(vec![item])
        }))
    }

    pub(crate) fn handle_type_hierarchy_supertypes(
        &mut self,
        params: TypeHierarchySupertypesParams,
    ) -> ELSResult<Option<Vec<TypeHierarchyItem>>> {
        _log!(self, "type hierarchy supertypes requested: {params:?}");
        let Some(typ) = self.type_from_item(&params.item) else {
            return Ok(Some(vec![]));
        };
        let uri = NormalizedUrl::new(params.item.uri);
        let supers = self.direct_supers(&uri, &typ);
        let items = supers
            .iter()
            .filter_map(|sup| self.item_from_type(&uri, sup))
            .collect();
        Ok(Some(items))
    }

    pub(crate) fn handle_type_hierarchy_subtypes(
        &mut self,
        params: TypeHierarchySubtypesParams,
    ) -> ELSResult<Option<Vec<TypeHierarchyItem>>> {
        _log!(self, "type hierarchy subtypes requested: {params:?}");
        let Some(data) = params.item.data.as_ref().and_then(|d| d.as_str()) else {
            return Ok(Some(vec![]));
        };
        let Ok(loc) = AbsLocation::from_str(data) else {
            return Ok(Some(vec![]));
        };
        let mut items = vec![];
        for impl_loc in self.get_class_impls(&loc) {
            let uri = NormalizedUrl::new(impl_loc.uri.clone());
            let Some(typ) = self.class_type_at(&uri, impl_loc.range.start) else {
                continue;
            };
            if let Some(item) = self.item_from_type(&uri, &typ) {
                items.push(item);
            }
        }
        Ok(Some(items))
    }

    fn type_at(&self, uri: &NormalizedUrl, pos: lsp_types::Position) -> Option<Type> {
        let token = self.file_cache.get_symbol(uri, pos)?;
        if let Some(mod_ctx) = self.get_mod_ctx(uri) {
            if let Some(ctx) = mod_ctx.context.get_type_ctx(token.inspect()) {
                return Some(ctx.typ.clone());
            }
        }
        let visitor = self.get_visitor(uri)?;
        let vi = visitor.get_info(&token)?;
        Some(vi.t.clone())
    }

    fn type_from_item(&self, item: &TypeHierarchyItem) -> Option<Type> {
        let uri = NormalizedUrl::new(item.uri.clone());
        let mod_ctx = self.get_mod_ctx(&uri)?;
        Some(mod_ctx.context.get_type_ctx(&item.name)?.typ.clone())
    }

    fn class_type_at(&self, uri: &NormalizedUrl, pos: lsp_types::Position) -> Option<Type> {
        let visitor = self.get_visitor(uri)?;
        Some(visitor.get_class_def_at(pos)?.obj.typ().clone())
    }

    /// Direct superclass (if any) plus traits this type implements itself.
    fn direct_supers(&self, uri: &NormalizedUrl, typ: &Type) -> Vec<Type> {
        let Some(mod_ctx) = self.module_for(uri, typ) else {
            return vec![];
        };
        let Some(ctx) = mod_ctx.context.get_nominal_type_ctx(typ) else {
            return vec![];
        };
        let parent = ctx.super_class_types().first().cloned();
        let parent_traits: Vec<Type> = parent
            .as_ref()
            .and_then(|p| mod_ctx.context.get_nominal_type_ctx(p))
            .map(|p| p.super_trait_types().to_vec())
            .unwrap_or_default();
        let mut supers = Vec::new();
        supers.extend(parent);
        for tr in ctx.super_trait_types() {
            if !parent_traits.iter().any(|p| p == tr) {
                supers.push(tr.clone());
            }
        }
        supers
    }

    fn module_for<'a>(
        &'a self,
        uri: &NormalizedUrl,
        typ: &Type,
    ) -> Option<MappedRwLockReadGuard<'a, ModuleContext>> {
        self.shared
            .get_module(Path::new(&typ.namespace()[..]))
            .map(|ent| MappedRwLockReadGuard::map(ent, |ent| &ent.module))
            .or_else(|| self.get_mod_ctx(uri))
    }

    fn item_from_type(&self, uri: &NormalizedUrl, typ: &Type) -> Option<TypeHierarchyItem> {
        let module = self.module_for(uri, typ)?;
        let (_, vi) = module.context.get_type_info(typ)?;
        let loc = abs_loc_to_lsp_loc(&vi.def_loc)?;
        let kind = match vi.t {
            Type::TraitType => SymbolKind::INTERFACE,
            Type::ClassType => SymbolKind::CLASS,
            _ => symbol_kind(vi),
        };
        Some(TypeHierarchyItem {
            name: typ.local_name().to_string(),
            kind,
            tags: None,
            detail: Some(typ.to_string()),
            uri: loc.uri,
            range: loc.range,
            selection_range: loc.range,
            data: Some(vi.def_loc.to_string().into()),
        })
    }
}
