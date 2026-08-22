use std::path::Path;

use erg_common::spawn::safe_yield;
use lsp_types::request::{
    CallHierarchyOutgoingCalls, CallHierarchyPrepare, ExecuteCommand, Formatting, GotoDeclaration,
    GotoImplementation, GotoImplementationParams, PrepareRenameRequest, RangeFormatting,
    WillRenameFiles, WorkspaceSymbol,
};
use lsp_types::{
    ApplyWorkspaceEditParams, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CompletionResponse, DiagnosticSeverity, DocumentChanges, DocumentFormattingParams,
    DocumentRangeFormattingParams, DocumentSymbolResponse, ExecuteCommandParams, FileRename,
    FoldingRange, FoldingRangeKind, FormattingOptions, GotoDefinitionParams,
    GotoDefinitionResponse, HoverContents, InlayHintLabel, MarkedString, Position,
    PrepareRenameResponse, Range, RenameFilesParams, TextDocumentIdentifier,
    TextDocumentPositionParams, WorkspaceSymbolParams,
};
const FILE_A: &str = "tests/a.er";
const FILE_B: &str = "tests/b.er";
const FILE_C: &str = "tests/c.er";
const FILE_IMPORTS: &str = "tests/imports.er";
const FILE_INVALID_SYNTAX: &str = "tests/invalid_syntax.er";
const FILE_RETRIGGER: &str = "tests/retrigger.er";
const FILE_TOLERANT_COMPLETION: &str = "tests/tolerant_completion.er";
const FILE_WITH_LENGTH: &str = "tests/with_length.er";
const FILE_PREPARE_RENAME: &str = "tests/prepare_rename.er";
const FILE_INHERIT_LENS: &str = "tests/inherit_lens.er";
const FILE_CALL_HIERARCHY: &str = "tests/call_hierarchy.er";
const FILE_FOLD: &str = "tests/fold.er";
const FILE_UNUSED_VARS: &str = "tests/unused_vars.er";
const FILE_COMMENTS: &str = "tests/comments.er";
const FILE_QUANTIFIED: &str = "tests/quantified.er";
const FILE_MULTI_IMPORT: &str = "tests/multi_import.er";
const FILE_SUB_MOD: &str = "tests/sub/mod.er";

use els::{
    NormalizedUrl, Server, TypeHierarchyPrepare, TypeHierarchyPrepareParams, TypeHierarchySubtypes,
    TypeHierarchySubtypesParams, TypeHierarchySupertypes, TypeHierarchySupertypesParams,
};
use erg_proc_macros::exec_new_thread;
use molc::{add_char, delete_line, oneline_range};

#[test]
fn test_open() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    client.wait_messages(3)?;
    client.notify_open(FILE_A)?;
    // log, work-done, etc.
    client.wait_messages(6)?;
    assert!(client.responses.iter().any(|val| val
        .to_string()
        .contains("tests/a.er passed, found warns: 0")));
    Ok(())
}

#[test]
fn test_completion() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri_a = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    let uri_b = NormalizedUrl::from_file_path(Path::new(FILE_B).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    client.notify_change(uri_a.clone().raw(), add_char(2, 0, "x"))?;
    client.notify_change(uri_a.clone().raw(), add_char(2, 1, "."))?;
    let resp = client.request_completion(uri_a.raw(), 2, 2, ".")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.len() >= 40);
        assert!(items.iter().any(|item| item.label == "abs"));
    } else {
        return Err(format!("{}: not items: {resp:?}", line!()).into());
    }
    client.notify_open(FILE_B)?;
    client.notify_change(uri_b.clone().raw(), add_char(6, 20, "."))?;
    let resp = client.request_completion(uri_b.raw(), 6, 21, ".")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.iter().any(|item| item.label == "a"));
    } else {
        return Err(format!("{}: not items: {resp:?}", line!()).into());
    }
    Ok(())
}

#[test]
fn test_completion_item_resolve() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri_a = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    client.notify_change(uri_a.clone().raw(), add_char(2, 0, "x"))?;
    client.notify_change(uri_a.clone().raw(), add_char(2, 1, "."))?;
    let resp = client.request_completion(uri_a.clone().raw(), 2, 2, ".")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.len() >= 40);
        assert!(items.iter().any(|item| item.label == "abs"));
        let Some(item) = items.into_iter().find(|item| item.label == "bit_count") else {
            return Err("`bit_count` method not found".into());
        };
        assert!(item.documentation.is_none());
        let resolved = client.request_completion_item_resolve(item)?;
        assert_eq!(resolved.label, "bit_count");
        assert!(resolved.documentation.is_some());
    } else {
        return Err(format!("not items: {resp:?}").into());
    }
    Ok(())
}

#[test]
fn test_neighbor_completion() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    client.notify_open(FILE_B)?;
    let resp = client.request_completion(uri.raw(), 2, 0, "n")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.len() >= 40);
        assert!(items
            .iter()
            .any(|item| item.label == "neighbor (import from b)"));
        Ok(())
    } else {
        Err(format!("not items: {resp:?}").into())
    }
}

#[test]
fn test_pymodule_completion() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    while !client.server.flags.builtin_modules_loaded() {
        safe_yield();
    }
    client.notify_change(uri.clone().raw(), add_char(2, 0, "c"))?;
    let resp = client.request_completion(uri.raw(), 2, 0, "c")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.len() >= 100);
        assert!(items
            .iter()
            .any(|item| item.label == "cos (import from math)"));
        Ok(())
    } else {
        Err(format!("not items: {resp:?}").into())
    }
}

#[test]
fn test_completion_retrigger() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_RETRIGGER).canonicalize()?)?;
    client.notify_open(FILE_RETRIGGER)?;
    let _ = client.wait_diagnostics()?;
    client.notify_change(uri.clone().raw(), add_char(2, 7, "n"))?;
    let resp = client.request_completion(uri.clone().raw(), 2, 7, "n")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(!items.is_empty());
        assert!(items.iter().any(|item| item.label == "print!"));
    } else {
        return Err(format!("{}: not items: {resp:?}", line!()).into());
    }
    client.notify_change(uri.clone().raw(), add_char(3, 15, "t"))?;
    let resp = client.request_completion(uri.raw(), 3, 15, "t")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(!items.is_empty());
        assert!(items.iter().any(|item| item.label == "bit_count"));
        assert!(items.iter().any(|item| item.label == "bit_length"));
    } else {
        return Err(format!("{}: not items: {resp:?}", line!()).into());
    }
    Ok(())
}

#[test]
fn test_tolerant_completion() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_TOLERANT_COMPLETION).canonicalize()?)?;
    client.notify_open(FILE_TOLERANT_COMPLETION)?;
    let _ = client.wait_diagnostics()?;
    client.notify_change(uri.clone().raw(), add_char(5, 16, "."))?;
    let resp = client.request_completion(uri.clone().raw(), 5, 17, ".")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.len() >= 40);
        assert!(items.iter().any(|item| item.label == "capitalize"));
    } else {
        return Err(format!("{}: not items: {resp:?}", line!()).into());
    }
    client.notify_change(uri.clone().raw(), add_char(5, 14, "."))?;
    let resp = client.request_completion(uri.clone().raw(), 5, 15, ".")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.len() >= 40);
        assert!(items.iter().any(|item| item.label == "abs"));
    } else {
        return Err(format!("{}: not items: {resp:?}", line!()).into());
    }
    client.notify_change(uri.clone().raw(), add_char(2, 9, "."))?;
    let resp = client.request_completion(uri.raw(), 2, 10, ".")?;
    if let Some(CompletionResponse::Array(items)) = resp {
        assert!(items.len() >= 10);
        assert!(items.iter().any(|item| item.label == "pi"));
        Ok(())
    } else {
        Err(format!("{}: not items: {resp:?}", line!()).into())
    }
}

#[test]
#[exec_new_thread]
fn test_rename() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    let edit = client
        .request_rename(uri.clone().raw(), 1, 5, "y")?
        .unwrap();
    assert!(edit
        .changes
        .is_some_and(|changes| changes.values().next().unwrap().len() == 2));
    client.notify_open(FILE_C)?;
    client.notify_open(FILE_B)?;
    let uri_b = NormalizedUrl::from_file_path(Path::new(FILE_B).canonicalize()?)?;
    // let uri_c = NormalizedUrl::from_file_path(Path::new(FILE_C).canonicalize()?)?;
    let edit = client
        .request_rename(uri_b.clone().raw(), 2, 1, "y")?
        .unwrap();
    assert_eq!(edit.changes.as_ref().unwrap().iter().count(), 2);
    for (_, change) in edit.changes.unwrap() {
        assert_eq!(change.len(), 1);
    }
    client.notify_save(uri_b.clone().raw())?;
    client.wait_diagnostics()?;
    let edit = client
        .request_rename(uri_b.clone().raw(), 4, 14, "b")?
        .unwrap();
    assert_eq!(edit.changes.as_ref().unwrap().iter().count(), 2);
    for (uri, change) in edit.changes.unwrap() {
        if uri.as_str().ends_with("b.er") {
            assert_eq!(change.len(), 2);
        } else {
            assert_eq!(change.len(), 1); // c.er
        }
    }
    Ok(())
}

#[test]
fn test_signature_help() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    client.notify_change(uri.clone().raw(), add_char(2, 0, "assert"))?;
    client.notify_change(uri.clone().raw(), add_char(2, 6, "("))?;
    let help = client
        .request_signature_help(uri.raw(), 2, 7, "(")?
        .unwrap();
    assert_eq!(help.signatures.len(), 1);
    let sig = &help.signatures[0];
    assert_eq!(sig.label, "::assert: (test: Bool, msg := Str) -> NoneType");
    assert_eq!(sig.active_parameter, Some(0));
    Ok(())
}

#[test]
fn test_hover() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    let hover = client.request_hover(uri.raw(), 1, 4)?.unwrap();
    let HoverContents::Array(contents) = hover.contents else {
        todo!()
    };
    assert_eq!(contents.len(), 2);
    let MarkedString::LanguageString(content) = &contents[0] else {
        todo!()
    };
    assert!(
        content.value == "# tests/a.er, line 1\nx = 1"
            || content.value == "# tests\\a.er, line 1\nx = 1"
    );
    let MarkedString::LanguageString(content) = &contents[1] else {
        todo!()
    };
    assert_eq!(content.value, "x: {1}");
    Ok(())
}

#[test]
#[exec_new_thread]
fn test_references() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    let locations = client.request_references(uri.raw(), 1, 4)?.unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(&locations[0].range, &oneline_range(1, 4, 5));
    client.notify_open(FILE_C)?;
    client.notify_open(FILE_B)?;
    let uri_b = NormalizedUrl::from_file_path(Path::new(FILE_B).canonicalize()?)?;
    let uri_c = NormalizedUrl::from_file_path(Path::new(FILE_C).canonicalize()?)?;
    let locations = client.request_references(uri_b.raw(), 0, 2)?.unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(NormalizedUrl::new(locations[0].uri.clone()), uri_c);
    Ok(())
}

/// `textDocument/prepareRename` returns the span of a renameable symbol and
/// `null` for a position that is not a user-defined symbol.
#[test]
#[exec_new_thread]
fn test_prepare_rename() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_PREPARE_RENAME).canonicalize()?)?;
    client.notify_open(FILE_PREPARE_RENAME)?;
    // `x` defined on line 0 is renameable
    let params = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier::new(uri.clone().raw()),
        position: Position::new(0, 0),
    };
    let resp = client.request::<PrepareRenameRequest>(params)?;
    let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) = resp else {
        return Err(format!("expected RangeWithPlaceholder, got {resp:?}").into());
    };
    assert_eq!(placeholder, "x");
    assert_eq!(range, oneline_range(0, 0, 1));
    // the builtin `abs` (line 1, col 4) cannot be renamed
    let params = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier::new(uri.raw()),
        position: Position::new(1, 4),
    };
    let resp = client.request::<PrepareRenameRequest>(params)?;
    assert!(resp.is_none(), "a builtin must not be renameable: {resp:?}");
    Ok(())
}

#[test]
fn test_goto_definition() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    let Some(GotoDefinitionResponse::Scalar(location)) =
        client.request_goto_definition(uri.raw(), 1, 4)?
    else {
        todo!()
    };
    assert_eq!(&location.range, &oneline_range(0, 0, 1));
    Ok(())
}

/// Regression: the HIR visitor must descend into `[elem; len]` (`ListWithLength`),
/// otherwise hover / goto-definition / references on `m` and `n` inside `a = [m; n]`
/// return nothing.
#[test]
#[exec_new_thread]
fn test_goto_definition_in_list_with_length() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_WITH_LENGTH).canonicalize()?)?;
    client.notify_open(FILE_WITH_LENGTH)?;
    // `m` (the element) used in `a = [m; n]` -> definition at line 0
    let Some(GotoDefinitionResponse::Scalar(location)) =
        client.request_goto_definition(uri.clone().raw(), 2, 5)?
    else {
        return Err("no definition found for `m` in `[m; n]`".into());
    };
    assert_eq!(&location.range, &oneline_range(0, 0, 1));
    // `n` (the length) used in `a = [m; n]` -> definition at line 1
    let Some(GotoDefinitionResponse::Scalar(location)) =
        client.request_goto_definition(uri.raw(), 2, 8)?
    else {
        return Err("no definition found for `n` in `[m; n]`".into());
    };
    assert_eq!(&location.range, &oneline_range(1, 0, 1));
    Ok(())
}

#[test]
#[exec_new_thread]
fn test_folding_range() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_IMPORTS).canonicalize()?)?;
    client.notify_open(FILE_IMPORTS)?;
    let ranges = client.request_folding_range(uri.raw())?.unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(
        &ranges[0],
        &FoldingRange {
            start_line: 0,
            start_character: Some(0),
            end_line: 3,
            end_character: Some(22),
            kind: Some(FoldingRangeKind::Imports),
        }
    );
    Ok(())
}

#[test]
fn test_document_symbol() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    let Some(DocumentSymbolResponse::Nested(symbols)) =
        client.request_document_symbols(uri.raw())?
    else {
        todo!()
    };
    assert_eq!(symbols.len(), 2);
    assert_eq!(&symbols[0].name, "x");
    Ok(())
}

/// `textDocument/didClose` must be handled (it was previously unhandled despite
/// `open_close: Some(true)`): the document is dropped from the cache and its
/// diagnostics are cleared.
#[test]
#[exec_new_thread]
fn test_did_close() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    client.wait_messages(3)?;
    client.notify_open(FILE_A)?;
    client.wait_messages(6)?;
    client.responses.clear();
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_close(uri.clone().raw())?;
    let diags = client.wait_diagnostics()?;
    assert_eq!(NormalizedUrl::new(diags.uri), uri);
    assert!(diags.diagnostics.is_empty(), "{:?}", diags.diagnostics);
    Ok(())
}

/// Code lens shows the subclass count above a class that is inherited from
/// (`send_class_inherits_lens` previously always returned an empty list).
/// `textDocument/implementation` lists the classes that implement a trait /
/// inherit a class (it previously only redirected to the definition).
/// Call hierarchy outgoing calls must populate `from_ranges` (the call sites),
/// which were previously always empty.
#[test]
#[exec_new_thread]
fn test_call_hierarchy_outgoing() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_CALL_HIERARCHY).canonicalize()?)?;
    client.notify_open(FILE_CALL_HIERARCHY)?;
    // prepare on `g` (line 1, col 0), which calls `f`
    let prep = CallHierarchyPrepareParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier::new(uri.raw()),
            position: Position::new(1, 0),
        },
        work_done_progress_params: Default::default(),
    };
    let items = client.request::<CallHierarchyPrepare>(prep)?.unwrap();
    let item = items
        .into_iter()
        .next()
        .ok_or("no call hierarchy item for `g`")?;
    let out = CallHierarchyOutgoingCallsParams {
        item,
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let calls = client.request::<CallHierarchyOutgoingCalls>(out)?.unwrap();
    let f_call = calls
        .iter()
        .find(|c| c.to.name == "f")
        .ok_or("`g` should have an outgoing call to `f`")?;
    assert!(
        !f_call.from_ranges.is_empty(),
        "from_ranges must contain the call site: {f_call:?}"
    );
    Ok(())
}

#[test]
#[exec_new_thread]
fn test_goto_implementation() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_INHERIT_LENS).canonicalize()?)?;
    client.notify_open(FILE_INHERIT_LENS)?;
    // `C` (defined on line 1) is inherited by exactly one subclass `D`
    let params = GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier::new(uri.raw()),
            position: Position::new(1, 0),
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let resp = client.request::<GotoImplementation>(params)?;
    let Some(GotoDefinitionResponse::Array(locations)) = resp else {
        return Err(format!("expected Array of implementations, got {resp:?}").into());
    };
    assert_eq!(locations.len(), 1, "{locations:?}");
    Ok(())
}

/// Type hierarchy: `D = Inherit C` → C's subtypes include D, D's supertypes include C.
#[test]
#[exec_new_thread]
fn test_type_hierarchy() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_INHERIT_LENS).canonicalize()?)?;
    client.notify_open(FILE_INHERIT_LENS)?;

    let c = client
        .request::<TypeHierarchyPrepare>(TypeHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier::new(uri.clone().raw()),
                position: Position::new(1, 0),
            },
            work_done_progress_params: Default::default(),
        })?
        .ok_or("prepareTypeHierarchy for C returned None")?
        .into_iter()
        .next()
        .ok_or("prepareTypeHierarchy for C returned no items")?;
    assert_eq!(c.name, "C", "{c:?}");

    let d = client
        .request::<TypeHierarchyPrepare>(TypeHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier::new(uri.clone().raw()),
                position: Position::new(5, 0),
            },
            work_done_progress_params: Default::default(),
        })?
        .ok_or("prepareTypeHierarchy for D returned None")?
        .into_iter()
        .next()
        .ok_or("prepareTypeHierarchy for D returned no items")?;
    assert_eq!(d.name, "D", "{d:?}");

    let subtypes = client
        .request::<TypeHierarchySubtypes>(TypeHierarchySubtypesParams {
            item: c,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })?
        .unwrap_or_default();
    assert!(
        subtypes.iter().any(|item| item.name == "D"),
        "C's subtypes should include D: {subtypes:?}"
    );

    let supertypes = client
        .request::<TypeHierarchySupertypes>(TypeHierarchySupertypesParams {
            item: d,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })?
        .unwrap_or_default();
    assert!(
        supertypes.iter().any(|item| item.name == "C"),
        "D's supertypes should include C: {supertypes:?}"
    );
    Ok(())
}

#[test]
#[exec_new_thread]
fn test_code_lens_inherits() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_INHERIT_LENS).canonicalize()?)?;
    client.notify_open(FILE_INHERIT_LENS)?;
    let lenses = client.request_code_lens(uri.raw())?.unwrap();
    // class `C` is inherited by exactly one subclass `D`
    assert!(
        lenses.iter().any(|l| l
            .command
            .as_ref()
            .is_some_and(|c| c.title == "1 subclasses")),
        "expected a '1 subclasses' lens, got {lenses:?}"
    );
    Ok(())
}

#[test]
fn test_inlay_hint() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    let hints = client.request_inlay_hint(uri.raw())?.unwrap();
    assert_eq!(hints.len(), 2);
    let InlayHintLabel::String(label) = &hints[0].label else {
        todo!()
    };
    assert_eq!(label, ": {1}");
    let InlayHintLabel::String(label) = &hints[1].label else {
        todo!()
    };
    // `x + 1` (x: {1}) is refined to the singleton `{2}`
    assert_eq!(label, ": {2}");
    Ok(())
}

#[test]
#[exec_new_thread]
fn test_dependents_check() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    client.wait_messages(3)?;
    client.notify_open(FILE_B)?;
    client.wait_messages(6)?;
    client.notify_open(FILE_C)?;
    client.wait_messages(6)?;
    let uri_b = NormalizedUrl::from_file_path(Path::new(FILE_B).canonicalize()?)?;
    // delete b.er:3, causing an error in c.er
    client.notify_change(uri_b.clone().raw(), delete_line(2))?;
    client.wait_messages(2)?;
    client.responses.clear();
    client.notify_save(uri_b.clone().raw())?;
    let b_diags = client.wait_diagnostics()?;
    assert!(b_diags.diagnostics.is_empty(), "{:?}", b_diags.diagnostics);
    let c_diags = client.wait_diagnostics()?;
    assert_eq!(
        c_diags.diagnostics.len(),
        1,
        "{:?} / {:?}",
        b_diags.diagnostics,
        c_diags.diagnostics
    );
    assert_eq!(
        c_diags.diagnostics[0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
    // insert invalid code to b.er:8, causing a syntax error in b.er but not c.er
    client.notify_change(uri_b.clone().raw(), add_char(7, 0, "a.\n"))?;
    client.wait_messages(1)?;
    client.responses.clear();
    client.notify_save(uri_b.clone().raw())?;
    let b_diags = client.wait_diagnostics()?;
    assert_eq!(b_diags.diagnostics.len(), 1, "{:?}", b_diags.diagnostics,);
    let c_diags = client.wait_diagnostics()?;
    assert!(c_diags
        .diagnostics
        .iter()
        .all(|diag| !diag.message.contains("expected: Indent, got: EOF")));
    Ok(())
}

#[test]
fn test_fix_error() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    client.notify_open(FILE_INVALID_SYNTAX)?;
    let diags = client.wait_diagnostics()?;
    assert_eq!(diags.diagnostics.len(), 1);
    assert_eq!(
        diags.diagnostics[0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_INVALID_SYNTAX).canonicalize()?)?;
    client.notify_change(uri.clone().raw(), add_char(0, 10, " 1"))?;
    client.notify_save(uri.clone().raw())?;
    let diags = client.wait_diagnostics()?;
    assert_eq!(diags.diagnostics.len(), 0);
    Ok(())
}

fn formatting_params(uri: lsp_types::Url) -> DocumentFormattingParams {
    DocumentFormattingParams {
        text_document: TextDocumentIdentifier::new(uri),
        options: FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            ..FormattingOptions::default()
        },
        work_done_progress_params: Default::default(),
    }
}

/// `textDocument/formatting` answers from the buffer, without type checking,
/// so it must work on a file that was only just changed and never saved.
#[test]
fn test_formatting() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;

    let edits = client.request::<Formatting>(formatting_params(uri.clone().raw()))?;
    assert_eq!(
        edits,
        Some(vec![]),
        "an already-formatted file needs no edits"
    );

    // `x = 1` -> `x =   1`, in the buffer only
    client.notify_change(uri.clone().raw(), add_char(0, 3, "  "))?;
    let edits = client
        .request::<Formatting>(formatting_params(uri.raw()))?
        .unwrap();
    assert_eq!(edits.len(), 1, "the whole document is replaced at once");
    assert_eq!(edits[0].new_text, "x = 1\n_ = x + 1\n");
    assert_eq!(edits[0].range.start, Position::new(0, 0));
    assert_eq!(
        edits[0].range.end,
        Position::new(2, 0),
        "past the last line"
    );
    Ok(())
}

fn formatting_options() -> FormattingOptions {
    FormattingOptions {
        tab_size: 4,
        insert_spaces: true,
        ..FormattingOptions::default()
    }
}

fn range_formatting_params(uri: lsp_types::Url, range: Range) -> DocumentRangeFormattingParams {
    DocumentRangeFormattingParams {
        text_document: TextDocumentIdentifier::new(uri),
        range,
        options: formatting_options(),
        work_done_progress_params: Default::default(),
    }
}

fn goto_params(uri: lsp_types::Url, line: u32, col: u32) -> GotoDefinitionParams {
    GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier::new(uri),
            position: Position::new(line, col),
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    }
}

/// `textDocument/rangeFormatting` must rewrite only a change that sits inside
/// the selection. A selection that does not contain the change is left alone.
#[test]
fn test_range_formatting() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;

    // `x = 1` -> `x =   1`, in the buffer only
    client.notify_change(uri.clone().raw(), add_char(0, 3, "  "))?;
    let first_line = Range {
        start: Position::new(0, 0),
        end: Position::new(1, 0),
    };
    let edits = client
        .request::<RangeFormatting>(range_formatting_params(uri.clone().raw(), first_line))?
        .unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "x = 1\n");
    assert_eq!(edits[0].range, first_line);

    let second_line = Range {
        start: Position::new(1, 0),
        end: Position::new(2, 0),
    };
    let edits = client
        .request::<RangeFormatting>(range_formatting_params(uri.raw(), second_line))?
        .unwrap();
    assert!(
        edits.is_empty(),
        "a selection that does not contain the change must not be rewritten: {edits:?}"
    );
    Ok(())
}

/// `textDocument/declaration` jumps to the binding and does not follow an
/// import the way `textDocument/definition` does.
#[test]
#[exec_new_thread]
fn test_goto_declaration() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_A).canonicalize()?)?;
    client.notify_open(FILE_A)?;
    let Some(GotoDefinitionResponse::Scalar(location)) =
        client.request::<GotoDeclaration>(goto_params(uri.clone().raw(), 1, 4))?
    else {
        return Err("no declaration found for `x`".into());
    };
    assert_eq!(&location.range, &oneline_range(0, 0, 1));

    let uri = NormalizedUrl::from_file_path(Path::new(FILE_IMPORTS).canonicalize()?)?;
    client.notify_open(FILE_IMPORTS)?;
    // `glob` on the import line: declaration stays on the binding.
    let Some(GotoDefinitionResponse::Scalar(location)) =
        client.request::<GotoDeclaration>(goto_params(uri.raw(), 0, 0))?
    else {
        return Err("no declaration found for imported `glob`".into());
    };
    assert_eq!(&location.range, &oneline_range(0, 0, 4));
    Ok(())
}

/// Folding covers functions and method blocks, not only consecutive imports.
#[test]
#[exec_new_thread]
fn test_folding_range_blocks() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_FOLD).canonicalize()?)?;
    client.notify_open(FILE_FOLD)?;
    let ranges = client.request_folding_range(uri.raw())?.unwrap();
    assert!(
        ranges.iter().any(|r| {
            r.kind == Some(FoldingRangeKind::Region) && r.start_line == 0 && r.end_line >= 2
        }),
        "function `f` should fold: {ranges:?}"
    );
    assert!(
        ranges.iter().any(|r| {
            r.kind == Some(FoldingRangeKind::Region)
                && r.start_line >= 4
                && r.end_line > r.start_line
        }),
        "methods of `C` should fold: {ranges:?}"
    );
    Ok(())
}

/// `erg.eliminate_unused_vars` sends `workspace/applyEdit`: drop unused defs
/// and rename unused parameters to `_`.
#[test]
#[exec_new_thread]
fn test_eliminate_unused_vars() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_UNUSED_VARS).canonicalize()?)?;
    client.notify_open(FILE_UNUSED_VARS)?;
    client.request::<ExecuteCommand>(ExecuteCommandParams {
        command: "erg.eliminate_unused_vars".to_string(),
        arguments: vec![serde_json::to_value(uri.clone().raw())?],
        work_done_progress_params: Default::default(),
    })?;
    let apply = client
        .responses
        .iter()
        .find(|msg| msg.get("method").and_then(|m| m.as_str()) == Some("workspace/applyEdit"))
        .ok_or("server did not send workspace/applyEdit")?;
    let params: ApplyWorkspaceEditParams = serde_json::from_value(apply["params"].clone())?;
    let edits = params
        .edit
        .changes
        .as_ref()
        .and_then(|c| c.get(&uri.raw()))
        .ok_or("no text edits for unused_vars.er")?;
    assert!(
        edits
            .iter()
            .any(|e| e.new_text.is_empty() && e.range.start.line == 0),
        "unused `foo` should be deleted: {edits:?}"
    );
    assert!(
        edits
            .iter()
            .any(|e| e.new_text == "_" && e.range.start.line == 1),
        "unused parameter `i` should become `_`: {edits:?}"
    );
    Ok(())
}

/// Completion is suppressed inside `#[ ]#` block comments and `'''` doc comments.
#[test]
fn test_completion_in_multiline_comment() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_COMMENTS).canonicalize()?)?;
    client.notify_open(FILE_COMMENTS)?;
    let inside_block = client.request_completion(uri.clone().raw(), 2, 0, "h")?;
    assert!(
        inside_block.is_none()
            || inside_block
                .as_ref()
                .is_some_and(|r| matches!(r, CompletionResponse::Array(a) if a.is_empty())),
        "completion inside #[ ]# must be suppressed: {inside_block:?}"
    );
    let inside_doc = client.request_completion(uri.raw(), 5, 0, "d")?;
    assert!(
        inside_doc.is_none()
            || inside_doc
                .as_ref()
                .is_some_and(|r| matches!(r, CompletionResponse::Array(a) if a.is_empty())),
        "completion inside ''' must be suppressed: {inside_doc:?}"
    );
    Ok(())
}

/// `id|` (type application) shows quantified type parameters.
#[test]
#[exec_new_thread]
fn test_signature_help_vbar() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    let uri = NormalizedUrl::from_file_path(Path::new(FILE_QUANTIFIED).canonicalize()?)?;
    client.notify_open(FILE_QUANTIFIED)?;
    client.notify_change(uri.clone().raw(), add_char(1, 6, "|"))?;
    let help = client
        .request_signature_help(uri.raw(), 1, 7, "|")?
        .ok_or("no signature help for type application")?;
    assert_eq!(help.signatures.len(), 1);
    let sig = &help.signatures[0];
    assert!(
        sig.label.contains('|') && sig.label.contains('T'),
        "expected type-parameter signature, got {}",
        sig.label
    );
    assert_eq!(sig.active_parameter, Some(0));
    Ok(())
}

/// Workspace symbols report the containing module or class.
#[test]
#[exec_new_thread]
fn test_workspace_symbol_container_name() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    client.notify_open(FILE_INHERIT_LENS)?;
    let symbols = client
        .request::<WorkspaceSymbol>(WorkspaceSymbolParams {
            query: String::new(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })?
        .ok_or("no workspace symbols")?;
    let class = symbols
        .iter()
        .find(|s| s.name == "C")
        .ok_or_else(|| format!("C not found: {symbols:?}"))?;
    assert!(
        class
            .container_name
            .as_deref()
            .is_some_and(|n| n.contains("inherit_lens")),
        "class C should be contained by the module, got {:?}",
        class.container_name
    );
    if let Some(method) = symbols.iter().find(|s| s.name == "new") {
        assert_eq!(method.container_name.as_deref(), Some("C"));
    }
    Ok(())
}

/// Renaming `sub/mod.er` rewrites `import "sub/mod"`, not a same-named sibling.
#[test]
#[exec_new_thread]
fn test_will_rename_multipath_import() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Server::bind_fake_client();
    client.request_initialize()?;
    client.notify_initialized()?;
    client.notify_open(FILE_MULTI_IMPORT)?;
    client.notify_open(FILE_SUB_MOD)?;
    let old = NormalizedUrl::from_file_path(Path::new(FILE_SUB_MOD).canonicalize()?)?;
    let mut new_path = Path::new(FILE_SUB_MOD).canonicalize()?;
    new_path.set_file_name("renamed.er");
    let new = NormalizedUrl::from_file_path(&new_path)?;
    let importer = NormalizedUrl::from_file_path(Path::new(FILE_MULTI_IMPORT).canonicalize()?)?;
    let importer_uri = importer.raw();
    let edit = client
        .request::<WillRenameFiles>(RenameFilesParams {
            files: vec![FileRename {
                old_uri: old.raw().to_string(),
                new_uri: new.raw().to_string(),
            }],
        })?
        .ok_or("willRenameFiles returned null")?;
    let rewritten = match &edit.document_changes {
        Some(DocumentChanges::Operations(ops)) => ops.iter().any(|op| match op {
            lsp_types::DocumentChangeOperation::Edit(td) => edit_rewrites_import(td, &importer_uri),
            _ => false,
        }),
        Some(DocumentChanges::Edits(edits)) => edits
            .iter()
            .any(|td| edit_rewrites_import(td, &importer_uri)),
        None => false,
    };
    assert!(
        rewritten,
        "import \"sub/mod\" should become \"sub/renamed\": {edit:?}"
    );
    Ok(())
}

fn edit_rewrites_import(td: &lsp_types::TextDocumentEdit, importer: &lsp_types::Url) -> bool {
    td.text_document.uri == *importer
        && td.edits.iter().any(|e| match e {
            lsp_types::OneOf::Left(te) => te.new_text.contains("sub/renamed"),
            lsp_types::OneOf::Right(ann) => ann.text_edit.new_text.contains("sub/renamed"),
        })
}
