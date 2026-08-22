//! Pull diagnostics (`textDocument/diagnostic`, `workspace/diagnostic`).
//!
//! `lsp-types` 0.93 does not ship these LSP 3.17 types, so they are defined here.
//! Push diagnostics (`textDocument/publishDiagnostics`) stay the live path;
//! pull returns the same stored errors/warnings.

use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::parse::Parsable;
use lsp_types::request::Request;
use lsp_types::{
    Diagnostic, PartialResultParams, TextDocumentIdentifier, Url, WorkDoneProgressParams,
};
use serde::{Deserialize, Serialize};

use crate::_log;
use crate::server::{DefaultFeatures, ELSResult, RedirectableStdout, Server};
use crate::util::NormalizedUrl;

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDiagnosticParams {
    pub text_document: TextDocumentIdentifier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_result_id: Option<String>,
    #[serde(flatten)]
    pub work_done_progress_params: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: PartialResultParams,
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FullDocumentDiagnosticReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    pub items: Vec<Diagnostic>,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnchangedDocumentDiagnosticReport {
    pub result_id: String,
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DocumentDiagnosticReport {
    Full(FullDocumentDiagnosticReport),
    Unchanged(UnchangedDocumentDiagnosticReport),
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
pub struct PreviousResultId {
    pub uri: Url,
    pub value: String,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiagnosticParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub previous_result_ids: Vec<PreviousResultId>,
    #[serde(flatten)]
    pub work_done_progress_params: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result_params: PartialResultParams,
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFullDocumentDiagnosticReport {
    pub uri: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    pub items: Vec<Diagnostic>,
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUnchangedDocumentDiagnosticReport {
    pub uri: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    pub result_id: String,
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkspaceDocumentDiagnosticReport {
    Full(WorkspaceFullDocumentDiagnosticReport),
    Unchanged(WorkspaceUnchangedDocumentDiagnosticReport),
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
pub struct WorkspaceDiagnosticReport {
    pub items: Vec<WorkspaceDocumentDiagnosticReport>,
}

#[derive(Debug)]
pub enum DocumentDiagnostic {}

impl Request for DocumentDiagnostic {
    type Params = DocumentDiagnosticParams;
    type Result = DocumentDiagnosticReport;
    const METHOD: &'static str = "textDocument/diagnostic";
}

#[derive(Debug)]
pub enum WorkspaceDiagnostic {}

impl Request for WorkspaceDiagnostic {
    type Params = WorkspaceDiagnosticParams;
    type Result = WorkspaceDiagnosticReport;
    const METHOD: &'static str = "workspace/diagnostic";
}

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_document_diagnostic(
        &mut self,
        params: DocumentDiagnosticParams,
    ) -> ELSResult<DocumentDiagnosticReport> {
        _log!(self, "document diagnostic requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document.uri);
        let result_id = self.diagnostic_result_id(&uri);
        if params.previous_result_id.as_ref() == Some(&result_id) {
            return Ok(DocumentDiagnosticReport::Unchanged(
                UnchangedDocumentDiagnosticReport { result_id },
            ));
        }
        let items = if self
            .disabled_features
            .contains(&DefaultFeatures::Diagnostics)
        {
            vec![]
        } else {
            self.diagnostics_for(&uri)
        };
        Ok(DocumentDiagnosticReport::Full(
            FullDocumentDiagnosticReport {
                result_id: Some(result_id),
                items,
            },
        ))
    }

    pub(crate) fn handle_workspace_diagnostic(
        &mut self,
        params: WorkspaceDiagnosticParams,
    ) -> ELSResult<WorkspaceDiagnosticReport> {
        _log!(self, "workspace diagnostic requested: {params:?}");
        let disabled = self
            .disabled_features
            .contains(&DefaultFeatures::Diagnostics);
        let mut items = vec![];
        for uri in self.diagnostic_uris() {
            let result_id = self.diagnostic_result_id(&uri);
            let version = self.file_cache.get_ver(&uri);
            let previous = params
                .previous_result_ids
                .iter()
                .find(|prev| NormalizedUrl::new(prev.uri.clone()) == uri)
                .map(|prev| prev.value.as_str());
            let lsp_uri = uri.clone().raw();
            if previous == Some(result_id.as_str()) {
                items.push(WorkspaceDocumentDiagnosticReport::Unchanged(
                    WorkspaceUnchangedDocumentDiagnosticReport {
                        uri: lsp_uri,
                        version,
                        result_id,
                    },
                ));
                continue;
            }
            let report_items = if disabled {
                vec![]
            } else {
                self.diagnostics_for(&uri)
            };
            items.push(WorkspaceDocumentDiagnosticReport::Full(
                WorkspaceFullDocumentDiagnosticReport {
                    uri: lsp_uri,
                    version,
                    result_id: Some(result_id),
                    items: report_items,
                },
            ));
        }
        Ok(WorkspaceDiagnosticReport { items })
    }
}
