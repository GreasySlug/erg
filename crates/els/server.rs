use std::any::type_name;
use std::io;
use std::io::{stdin, BufRead, Read};
use std::ops::Not;
use std::panic;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use erg_common::config::{ErgConfig, ErgMode};
use erg_common::consts::PYTHON_MODE;
use erg_common::dict::Dict;
use erg_common::env::erg_path;
use erg_common::pathutil::{project_entry_dir_of, NormalizedPathBuf};
use erg_common::set::Set;
use erg_common::shared::{MappedRwLockReadGuard, Shared};
use erg_common::spawn::{safe_yield, spawn_new_thread};
use erg_common::traits::Stream;
use erg_common::vfs::VFS;
use erg_common::{fn_name, lsp_log, normalize_path};

use erg_compiler::artifact::BuildRunnable;
use erg_compiler::build_package::PackageBuilder;
use erg_compiler::context::{Context, ModuleContext};
use erg_compiler::erg_parser::ast::Module;
use erg_compiler::erg_parser::parse::{Parsable, SimpleParser};
use erg_compiler::error::CompileWarning;
use erg_compiler::hir::{Expr, HIR};
use erg_compiler::lower::ASTLowerer;
use erg_compiler::module::{IRs, ModuleEntry, SharedCompilerResource};
use erg_compiler::ty::{HasType, Type};

pub use molc::RedirectableStdout;
use molc::{FakeClient, LangServer};

use lsp_types::request::{
    CallHierarchyIncomingCalls, CallHierarchyOutgoingCalls, CallHierarchyPrepare,
    CodeActionRequest, CodeActionResolveRequest, CodeLensRequest, Completion,
    DocumentHighlightRequest, DocumentLinkRequest, DocumentSymbolRequest, ExecuteCommand,
    FoldingRangeRequest, GotoDefinition, GotoImplementation, GotoTypeDefinition, HoverRequest,
    InlayHintRequest, InlayHintResolveRequest, References, Rename, Request, ResolveCompletionItem,
    SelectionRangeRequest, SemanticTokensFullRequest, SignatureHelpRequest, WillRenameFiles,
    WorkspaceSymbol,
};
use lsp_types::{
    CallHierarchyServerCapability, CodeActionKind, CodeActionOptions, CodeActionProviderCapability,
    CodeLensOptions, CompletionOptions, ConfigurationItem, ConfigurationParams,
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentLinkOptions, ExecuteCommandOptions, FoldingRangeProviderCapability,
    HoverProviderCapability, ImplementationProviderCapability, InitializeParams, InitializeResult,
    InlayHintOptions, InlayHintServerCapabilities, NumberOrString, OneOf, Position, ProgressParams,
    ProgressParamsValue, SelectionRangeProviderCapability, SemanticTokenModifier,
    SemanticTokenType, SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensServerCapabilities, ServerCapabilities, SignatureHelpOptions,
    TypeDefinitionProviderCapability, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressCreateParams, WorkDoneProgressEnd, WorkDoneProgressOptions,
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;

use crate::channels::{SendChannels, Sendable, WorkerMessage};
use crate::completion::CompletionCache;
use crate::file_cache::FileCache;
use crate::hir_visitor::{ExprKind, HIRVisitor};
use crate::message::{ErrorMessage, LSPResult};
use crate::scheduler::Scheduler;
use crate::util::{self, loc_to_pos, NormalizedUrl};

pub const HEALTH_CHECKER_ID: i64 = 10000;
pub const ASK_AUTO_SAVE_ID: i64 = 10001;

pub type ELSResult<T> = Result<T, Box<dyn std::error::Error>>;

pub type ErgLanguageServer = Server<PackageBuilder>;

pub type Handler<Server, Params, Out = ()> = fn(&mut Server, Params) -> ELSResult<Out>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefaultFeatures {
    /* LSP features */
    CodeAction,
    CodeLens,
    Completion,
    Diagnostics,
    FindReferences,
    GotoDefinition,
    GotoTypeDefinition,
    Hover,
    InlayHint,
    Rename,
    SemanticTokens,
    SignatureHelp,
    DocumentHighlight,
    DocumentLink,
    FoldingRange,
    SelectionRange,
    /* ELS specific features */
    SmartCompletion,
    DeepCompletion,
}

impl From<&str> for DefaultFeatures {
    fn from(s: &str) -> Self {
        match s {
            "codeaction" | "codeAction" | "code-action" => DefaultFeatures::CodeAction,
            "codelens" | "codeLens" | "code-lens" => DefaultFeatures::CodeLens,
            "completion" => DefaultFeatures::Completion,
            "diagnostic" | "diagnostics" => DefaultFeatures::Diagnostics,
            "hover" => DefaultFeatures::Hover,
            "semantictoken" | "semantictokens" | "semanticToken" | "semanticTokens"
            | "semantic-tokens" => DefaultFeatures::SemanticTokens,
            "rename" => DefaultFeatures::Rename,
            "inlayhint" | "inlayhints" | "inlayHint" | "inlayHints" | "inlay-hint"
            | "inlay-hints" => DefaultFeatures::InlayHint,
            "findreferences" | "findReferences" | "find-references" | "references" => {
                DefaultFeatures::FindReferences
            }
            "gotodefinition" | "gotoDefinition" | "goto-definition" => {
                DefaultFeatures::GotoDefinition
            }
            "gototypedefinition" | "gotoTypeDefinition" | "goto-type-definition" => {
                DefaultFeatures::GotoTypeDefinition
            }
            "signaturehelp" | "signatureHelp" | "signature-help" | "code-signature" => {
                DefaultFeatures::SignatureHelp
            }
            "documenthighlight" | "documentHighlight" | "document-highlight" => {
                DefaultFeatures::DocumentHighlight
            }
            "documentlink" | "documentLink" | "document-link" => DefaultFeatures::DocumentLink,
            "foldingrange" | "foldingRange" | "folding-range" => DefaultFeatures::FoldingRange,
            "selectionrange" | "selectionRange" | "selection-range" => {
                DefaultFeatures::SelectionRange
            }
            "smartcompletion" | "smartCompletion" | "smart-completion" => {
                DefaultFeatures::SmartCompletion
            }
            "deepcompletion" | "deepCompletion" | "deep-completion" => {
                DefaultFeatures::DeepCompletion
            }
            _ => panic!("unknown feature: {s}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionalFeatures {
    CheckOnType,
    Lint,
}

impl From<&str> for OptionalFeatures {
    fn from(s: &str) -> Self {
        match s {
            "checkontype" | "checkOnType" | "check-on-type" => OptionalFeatures::CheckOnType,
            "lint" | "linting" => OptionalFeatures::Lint,
            _ => panic!("unknown feature: {s}"),
        }
    }
}

#[macro_export]
macro_rules! _log {
    ($self:ident, $($arg:tt)*) => {{
        let s = format!($($arg)*);
        let _ = $self.send_log(format!("{}@{}: {s}", file!(), line!()));
    }};
}

pub const TRIGGER_CHARS: [&str; 4] = [".", ":", "(", " "];

#[derive(Debug, Clone, Default)]
pub struct Flags {
    pub(crate) client_initialized: Arc<AtomicBool>,
    pub(crate) workspace_checked: Arc<AtomicBool>,
    pub(crate) builtin_modules_loaded: Arc<AtomicBool>,
}

impl Flags {
    pub fn client_initialized(&self) -> bool {
        self.client_initialized.load(Ordering::Relaxed)
    }

    pub fn workspace_checked(&self) -> bool {
        self.workspace_checked.load(Ordering::Relaxed)
    }

    #[allow(unused)]
    pub fn builtin_modules_loaded(&self) -> bool {
        self.builtin_modules_loaded.load(Ordering::Relaxed)
    }
}

/// A Language Server, which can be used any object implementing `BuildRunnable` internally by passing it as a generic parameter.
#[derive(Debug)]
pub struct Server<Checker: BuildRunnable = PackageBuilder, Parser: Parsable = SimpleParser> {
    pub(crate) cfg: ErgConfig,
    pub(crate) home: PathBuf,
    pub(crate) erg_path: PathBuf,
    pub(crate) init_params: InitializeParams,
    pub(crate) client_answers: Shared<Dict<i64, Value>>,
    pub(crate) disabled_features: Vec<DefaultFeatures>,
    pub(crate) opt_features: Vec<OptionalFeatures>,
    pub(crate) file_cache: FileCache,
    pub(crate) comp_cache: CompletionCache,
    pub flags: Flags,
    pub(crate) shared: SharedCompilerResource,
    pub(crate) channels: Option<SendChannels>,
    pub(crate) stdout_redirect: Option<mpsc::Sender<Value>>,
    pub(crate) scheduler: Scheduler,
    pub(crate) _parser: std::marker::PhantomData<fn() -> Parser>,
    pub(crate) _checker: std::marker::PhantomData<fn() -> Checker>,
}

impl<C: BuildRunnable, P: Parsable> RedirectableStdout for Server<C, P> {
    fn sender(&self) -> Option<&mpsc::Sender<Value>> {
        self.stdout_redirect.as_ref()
    }
}

impl LangServer for Server {
    fn dispatch(&mut self, msg: impl Into<Value>) -> Result<(), Box<dyn std::error::Error>> {
        self.dispatch(msg.into())
    }
}

impl Server {
    #[allow(unused)]
    pub fn bind_fake_client() -> FakeClient<Server> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cfg = ErgConfig {
            mode: ErgMode::LanguageServer,
            ..Default::default()
        };
        FakeClient::new(Server::new(cfg, Some(sender)), receiver)
    }
}

impl<C: BuildRunnable, P: Parsable> Clone for Server<C, P> {
    fn clone(&self) -> Self {
        Self {
            cfg: self.cfg.clone(),
            home: self.home.clone(),
            erg_path: self.erg_path.clone(),
            init_params: self.init_params.clone(),
            client_answers: self.client_answers.clone(),
            disabled_features: self.disabled_features.clone(),
            opt_features: self.opt_features.clone(),
            file_cache: self.file_cache.clone(),
            comp_cache: self.comp_cache.clone(),
            shared: self.shared.clone(),
            channels: self.channels.clone(),
            flags: self.flags.clone(),
            stdout_redirect: self.stdout_redirect.clone(),
            scheduler: self.scheduler.clone(),
            _parser: std::marker::PhantomData,
            _checker: std::marker::PhantomData,
        }
    }
}

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub fn new(cfg: ErgConfig, stdout_redirect: Option<mpsc::Sender<Value>>) -> Self {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            lsp_log!("{panic_info}");
            default_hook(panic_info);
        }));

        let flags = Flags::default();
        let cfg = Self::register_packages(cfg);
        let shared = SharedCompilerResource::new(cfg.copy());
        let mut args = cfg.runtime_args.iter();
        let mut disabled_features = vec![];
        let mut opt_features = vec![];
        while let Some(&arg) = args.next() {
            if arg == "--disable" {
                if let Some(&feature) = args.next() {
                    disabled_features.push(DefaultFeatures::from(feature));
                }
            } else if arg == "--enable" {
                if let Some(&feature) = args.next() {
                    opt_features.push(OptionalFeatures::from(feature));
                }
            }
        }
        let external_items = !disabled_features.contains(&DefaultFeatures::DeepCompletion);
        Self {
            comp_cache: CompletionCache::new(
                cfg.copy(),
                flags.clone(),
                shared.clone(),
                external_items,
            ),
            shared,
            cfg,
            home: normalize_path(std::env::current_dir().unwrap_or_default()),
            erg_path: erg_path().clone(), // already normalized
            init_params: InitializeParams::default(),
            client_answers: Shared::new(Dict::new()),
            disabled_features,
            opt_features,
            file_cache: FileCache::new(stdout_redirect.clone()),
            channels: None,
            flags,
            stdout_redirect,
            scheduler: Scheduler::new(),
            _parser: std::marker::PhantomData,
            _checker: std::marker::PhantomData,
        }
    }

    fn register_packages(mut cfg: ErgConfig) -> ErgConfig {
        use erg_compiler::erg_parser::ast;
        let home = normalize_path(std::env::current_dir().unwrap_or_default());
        let Ok(src) = VFS.read(home.join("package.lock.er")) else {
            lsp_log!("package.lock.er not found");
            return cfg;
        };
        let Ok(artifact) = <erg_compiler::erg_parser::Parser as Parsable>::parse(src) else {
            lsp_log!("failed to parse package.lock.er");
            return cfg;
        };
        let Some(pkgs) = artifact.ast.get_attr("packages") else {
            lsp_log!("failed to get packages: {}", artifact.ast);
            return cfg;
        };
        let Some(ast::Expr::List(ast::List::Normal(lis))) = pkgs.body.block.first() else {
            lsp_log!("packages must be an array: {pkgs}");
            return cfg;
        };
        let mut packages = vec![];
        for rec in lis.iter() {
            let ast::Expr::Record(rec) = rec else {
                lsp_log!("packages must be records: {rec}");
                break;
            };
            let Some(ast::Expr::Literal(name)) =
                rec.get("name").and_then(|name| name.body.block.first())
            else {
                break;
            };
            let name = name.token.content.replace('\"', "");
            let Some(ast::Expr::Literal(as_name)) = rec
                .get("as_name")
                .and_then(|as_name| as_name.body.block.first())
            else {
                break;
            };
            let as_name = as_name.token.content.replace('\"', "");
            let Some(ast::Expr::Literal(version)) = rec
                .get("version")
                .and_then(|version| version.body.block.first())
            else {
                break;
            };
            let version = version.token.content.replace('\"', "");
            let path = rec.get("path").and_then(|path| {
                if let Some(ast::Expr::Literal(lit)) = path.body.block.first() {
                    Some(lit.token.content.replace('\"', ""))
                } else {
                    None
                }
            });
            let package = erg_common::config::Package::new(
                name.leak(),
                as_name.leak(),
                version.leak(),
                path.map(|p| &*p.leak()),
            );
            packages.push(package);
        }
        cfg.packages = Arc::from(packages);
        cfg
    }

    pub fn run(mut self) {
        let mut _self = self.clone();
        let mut handle = spawn_new_thread(
            move || loop {
                let msg = match _self.read_message() {
                    Ok(msg) => msg,
                    Err(err) => {
                        lsp_log!("error: {err}");
                        if err.kind() == io::ErrorKind::UnexpectedEof {
                            panic!("unexpected EOF");
                        }
                        continue;
                    }
                };
                if let Err(err) = _self.dispatch(msg) {
                    lsp_log!("error: {err}");
                    if err.to_string().contains("sending on a closed channel") {
                        _self
                            .send_error_info("An error occurred. Restarting...")
                            .unwrap();
                        _self.restart();
                    };
                }
            },
            fn_name!(),
        );
        loop {
            // recover from crash
            if handle.is_finished() {
                let _ = self.send_error_info("The compiler has crashed. Restarting...");
                self.restart();
                let mut _self = self.clone();
                handle = spawn_new_thread(
                    move || loop {
                        let msg = match _self.read_message() {
                            Ok(msg) => msg,
                            Err(err) => {
                                lsp_log!("error: {err}");
                                if err.kind() == io::ErrorKind::UnexpectedEof {
                                    panic!("unexpected EOF");
                                }
                                continue;
                            }
                        };
                        if let Err(err) = _self.dispatch(msg) {
                            lsp_log!("error: {err}");
                            if err.to_string().contains("sending on a closed channel") {
                                _self
                                    .send_error_info("An error occurred again. Restarting...")
                                    .unwrap();
                                _self.restart();
                            };
                        }
                    },
                    fn_name!(),
                );
            }
            safe_yield();
        }
        // Ok(())
    }

    pub const fn mode(&self) -> &str {
        if PYTHON_MODE {
            "pylyzer"
        } else {
            "erg"
        }
    }

    fn read_line(&self) -> io::Result<String> {
        let mut line = String::new();
        stdin().lock().read_line(&mut line)?;
        Ok(line)
    }

    fn read_exact(&self, len: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0; len];
        stdin().lock().read_exact(&mut buf)?;
        Ok(buf)
    }

    #[allow(clippy::field_reassign_with_default)]
    fn init(&mut self, msg: &Value, id: i64) -> ELSResult<()> {
        self.send_log("initializing ELS")?;
        if msg.get("params").is_some() && msg["params"].get("capabilities").is_some() {
            self.init_params = InitializeParams::deserialize(&msg["params"])?;
            // self.send_log(format!("set client capabilities: {:?}", self.client_capas))?;
        }
        let mut result = InitializeResult::default();
        result.capabilities = self.init_capabilities();
        self.init_services();
        self.send_stdout(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
    }

    #[allow(clippy::field_reassign_with_default)]
    fn init_capabilities(&mut self) -> ServerCapabilities {
        let mut capabilities = ServerCapabilities::default();
        self.file_cache.set_capabilities(&mut capabilities);
        let mut comp_options = CompletionOptions::default();
        comp_options.trigger_characters = Some(TRIGGER_CHARS.map(String::from).to_vec());
        comp_options.resolve_provider = Some(true);
        capabilities.completion_provider = if self
            .disabled_features
            .contains(&DefaultFeatures::Completion)
        {
            None
        } else {
            Some(comp_options)
        };
        capabilities.rename_provider = Some(OneOf::Left(true));
        capabilities.references_provider = Some(OneOf::Left(true));
        capabilities.definition_provider = Some(OneOf::Left(true));
        capabilities.type_definition_provider =
            Some(TypeDefinitionProviderCapability::Simple(true));
        capabilities.implementation_provider = Some(ImplementationProviderCapability::Simple(true));
        capabilities.hover_provider = self
            .disabled_features
            .contains(&DefaultFeatures::Hover)
            .not()
            .then_some(HoverProviderCapability::Simple(true));
        capabilities.inlay_hint_provider = self
            .disabled_features
            .contains(&DefaultFeatures::InlayHint)
            .not()
            .then_some(OneOf::Right(InlayHintServerCapabilities::Options(
                InlayHintOptions {
                    resolve_provider: Some(true),
                    ..Default::default()
                },
            )));
        let mut sema_options = SemanticTokensOptions::default();
        sema_options.range = Some(false);
        sema_options.full = Some(SemanticTokensFullOptions::Bool(true));
        sema_options.legend = SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::NAMESPACE,
                SemanticTokenType::TYPE,
                SemanticTokenType::CLASS,
                SemanticTokenType::INTERFACE,
                SemanticTokenType::TYPE_PARAMETER,
                SemanticTokenType::PARAMETER,
                SemanticTokenType::VARIABLE,
                SemanticTokenType::PROPERTY,
                SemanticTokenType::FUNCTION,
                SemanticTokenType::METHOD,
                SemanticTokenType::STRING,
                SemanticTokenType::NUMBER,
                SemanticTokenType::OPERATOR,
            ],
            token_modifiers: vec![
                SemanticTokenModifier::DECLARATION,
                SemanticTokenModifier::DEFINITION,
                SemanticTokenModifier::READONLY,
                SemanticTokenModifier::STATIC,
                SemanticTokenModifier::DEPRECATED,
                SemanticTokenModifier::ABSTRACT,
                SemanticTokenModifier::ASYNC,
                SemanticTokenModifier::MODIFICATION,
                SemanticTokenModifier::DOCUMENTATION,
                SemanticTokenModifier::DEFAULT_LIBRARY,
            ],
        };
        capabilities.semantic_tokens_provider = self
            .disabled_features
            .contains(&DefaultFeatures::SemanticTokens)
            .not()
            .then_some(SemanticTokensServerCapabilities::SemanticTokensOptions(
                sema_options,
            ));
        capabilities.code_action_provider = if self
            .disabled_features
            .contains(&DefaultFeatures::CodeAction)
        {
            None
        } else {
            let options = CodeActionProviderCapability::Options(CodeActionOptions {
                code_action_kinds: Some(vec![CodeActionKind::QUICKFIX, CodeActionKind::REFACTOR]),
                resolve_provider: Some(true),
                work_done_progress_options: WorkDoneProgressOptions::default(),
            });
            Some(options)
        };
        capabilities.execute_command_provider = Some(ExecuteCommandOptions {
            commands: vec![format!("{}.eliminate_unused_vars", self.mode())],
            work_done_progress_options: WorkDoneProgressOptions::default(),
        });
        capabilities.signature_help_provider = self
            .disabled_features
            .contains(&DefaultFeatures::SignatureHelp)
            .not()
            .then_some(SignatureHelpOptions {
                trigger_characters: Some(vec!["(".to_string(), ",".to_string(), "|".to_string()]),
                retrigger_characters: None,
                work_done_progress_options: WorkDoneProgressOptions {
                    work_done_progress: None,
                },
            });
        capabilities.code_lens_provider =
            if self.disabled_features.contains(&DefaultFeatures::CodeLens) {
                None
            } else {
                Some(CodeLensOptions {
                    resolve_provider: Some(false),
                })
            };
        capabilities.workspace_symbol_provider = Some(OneOf::Left(true));
        capabilities.document_symbol_provider = Some(OneOf::Left(true));
        capabilities.document_link_provider = if self
            .disabled_features
            .contains(&DefaultFeatures::DocumentLink)
        {
            None
        } else {
            Some(DocumentLinkOptions {
                resolve_provider: Some(false),
                work_done_progress_options: Default::default(),
            })
        };
        capabilities.call_hierarchy_provider = Some(CallHierarchyServerCapability::Simple(true));
        capabilities.folding_range_provider = Some(FoldingRangeProviderCapability::Simple(
            self.disabled_features
                .contains(&DefaultFeatures::FoldingRange)
                .not(),
        ));
        capabilities.selection_range_provider =
            Some(SelectionRangeProviderCapability::Simple(true));
        capabilities.document_highlight_provider = Some(OneOf::Left(
            self.disabled_features
                .contains(&DefaultFeatures::DocumentHighlight)
                .not(),
        ));
        capabilities
    }

    pub(crate) fn ask_auto_save(&self) -> ELSResult<()> {
        let params = ConfigurationParams {
            items: vec![ConfigurationItem {
                scope_uri: None,
                section: Some("files.autoSave".to_string()),
            }],
        };
        self.send_stdout(&json!({
            "jsonrpc": "2.0",
            "id": ASK_AUTO_SAVE_ID,
            "method": "workspace/configuration",
            "params": params,
        }))
    }

    fn start_language_services(&mut self) {
        let (senders, receivers) = SendChannels::new();
        self.channels = Some(senders);
        self.start_service::<Completion>(receivers.completion, Self::handle_completion);
        self.start_service::<ResolveCompletionItem>(
            receivers.resolve_completion,
            Self::handle_resolve_completion,
        );
        self.start_service::<GotoDefinition>(
            receivers.goto_definition,
            Self::handle_goto_definition,
        );
        self.start_service::<GotoTypeDefinition>(
            receivers.goto_type_definition,
            Self::handle_goto_type_definition,
        );
        self.start_service::<GotoImplementation>(
            receivers.goto_implementation,
            Self::handle_goto_implementation,
        );
        self.start_service::<SemanticTokensFullRequest>(
            receivers.semantic_tokens_full,
            Self::handle_semantic_tokens_full,
        );
        self.start_service::<InlayHintRequest>(receivers.inlay_hint, Self::handle_inlay_hint);
        self.start_service::<InlayHintResolveRequest>(
            receivers.inlay_hint_resolve,
            Self::handle_inlay_hint_resolve,
        );
        self.start_service::<HoverRequest>(receivers.hover, Self::handle_hover);
        self.start_service::<References>(receivers.references, Self::handle_references);
        self.start_service::<CodeLensRequest>(receivers.code_lens, Self::handle_code_lens);
        self.start_service::<CodeActionRequest>(receivers.code_action, Self::handle_code_action);
        self.start_service::<CodeActionResolveRequest>(
            receivers.code_action_resolve,
            Self::handle_code_action_resolve,
        );
        self.start_service::<SignatureHelpRequest>(
            receivers.signature_help,
            Self::handle_signature_help,
        );
        self.start_service::<WillRenameFiles>(
            receivers.will_rename_files,
            Self::handle_will_rename_files,
        );
        self.start_service::<ExecuteCommand>(
            receivers.execute_command,
            Self::handle_execute_command,
        );
        self.start_service::<WorkspaceSymbol>(
            receivers.workspace_symbol,
            Self::handle_workspace_symbol,
        );
        self.start_service::<DocumentSymbolRequest>(
            receivers.document_symbol,
            Self::handle_document_symbol,
        );
        self.start_service::<CallHierarchyPrepare>(
            receivers.call_hierarchy_prepare,
            Self::handle_call_hierarchy_prepare,
        );
        self.start_service::<CallHierarchyIncomingCalls>(
            receivers.call_hierarchy_incoming,
            Self::handle_call_hierarchy_incoming,
        );
        self.start_service::<CallHierarchyOutgoingCalls>(
            receivers.call_hierarchy_outgoing,
            Self::handle_call_hierarchy_outgoing,
        );
        self.start_service::<FoldingRangeRequest>(
            receivers.folding_range,
            Self::handle_folding_range,
        );
        self.start_service::<SelectionRangeRequest>(
            receivers.selection_range,
            Self::handle_selection_range,
        );
        self.start_service::<DocumentHighlightRequest>(
            receivers.document_highlight,
            Self::handle_document_highlight,
        );
        self.start_service::<DocumentLinkRequest>(
            receivers.document_link,
            Self::handle_document_link,
        );
        self.start_client_health_checker(receivers.health_check);
    }

    fn init_services(&mut self) {
        self.start_language_services();
        self.start_workspace_diagnostics();
        self.start_auto_diagnostics();
    }

    fn exit(&self) -> ELSResult<()> {
        self.send_log("exiting ELS")?;
        std::process::exit(0);
    }

    fn shutdown(&self, id: i64) -> ELSResult<()> {
        self.send_log("shutting down ELS")?;
        self.send_stdout(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": json!(null),
        }))
    }

    /// Restart the server. Clear caches and close & reopen channels.
    /// The server does not erase client-dependent information because it cannot notify the client of the restart and receive resources (such as workspace files) again.
    #[allow(unused)]
    pub(crate) fn restart(&mut self) {
        lsp_log!("restarting ELS");
        // self.file_cache.clear();
        self.comp_cache.clear();
        if let Some(chan) = self.channels.as_ref() {
            chan.close();
        }
        self.shared.clear_all();
        self.start_language_services();
        self.start_workspace_diagnostics();
    }

    /// Copied and modified from RLS, https://github.com/rust-lang/rls/blob/master/rls/src/server/io.rs
    fn read_message(&self) -> Result<Value, io::Error> {
        // Read in the "Content-Length: xx" part.
        let mut size: Option<usize> = None;
        loop {
            let buffer = self.read_line()?;

            // End of input.
            if buffer.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF encountered in the middle of reading LSP headers",
                ));
            }

            // Header section is finished, break from the loop.
            if buffer == "\r\n" {
                break;
            }

            let res: Vec<&str> = buffer.split(' ').collect();

            // Make sure header is valid.
            if res.len() != 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Header '{buffer}' is malformed"),
                ));
            }
            let header_name = res[0].to_lowercase();
            let header_value = res[1].trim();

            match header_name.as_ref() {
                "content-length:" => {
                    size = Some(header_value.parse::<usize>().map_err(|_e| {
                        io::Error::new(io::ErrorKind::InvalidData, "Couldn't read size")
                    })?);
                }
                "content-type:" => {
                    if header_value != "utf8" && header_value != "utf-8" {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Content type '{header_value}' is invalid"),
                        ));
                    }
                }
                // Ignore unknown headers (specification doesn't say what to do in this case).
                _ => (),
            }
        }
        let size = match size {
            Some(size) => size,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Message is missing 'content-length' header",
                ));
            }
        };

        let content = self.read_exact(size)?;

        let s = String::from_utf8(content)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Value::from_str(&s)?)
    }

    pub fn dispatch(&mut self, msg: Value) -> ELSResult<()> {
        match (
            msg.get("id")
                .and_then(|i| i.as_i64().or(i.as_str().and_then(|s| s.parse().ok()))),
            msg.get("method").and_then(|m| m.as_str()),
        ) {
            (Some(id), Some(method)) => self.handle_request(&msg, id, method),
            (Some(id), None) => self.handle_response(id, &msg),
            (None, Some(notification)) => self.handle_notification(&msg, notification),
            _ => self.send_invalid_req_error(),
        }
    }

    fn parse_send<R>(&self, id: i64, msg: &Value) -> ELSResult<()>
    where
        R: lsp_types::request::Request + 'static,
        R::Result: Serialize,
        Server<Checker, Parser>: Sendable<R>,
    {
        let params = R::Params::deserialize(&msg["params"])?;
        self.send(id, params)?;
        Ok(())
    }

    fn start_service<R>(
        &self,
        receiver: mpsc::Receiver<WorkerMessage<R::Params>>,
        handler: Handler<Server<Checker, Parser>, R::Params, R::Result>,
    ) where
        R: lsp_types::request::Request + 'static,
        R::Params: Send,
        R::Result: Serialize,
    {
        let mut _self = self.clone();
        spawn_new_thread(
            move || loop {
                match receiver.recv() {
                    Ok(WorkerMessage::Request(id, params)) => {
                        _self.scheduler.register(id, R::METHOD);
                        if _self.scheduler.acquire(id).is_none() {
                            _log!(_self, "canceled: {id}");
                            continue;
                        }
                        match handler(&mut _self, params) {
                            Ok(result) => {
                                let _ = _self.send_stdout(&LSPResult::new(id, result));
                            }
                            Err(err) => {
                                lsp_log!("error: {err}");
                                let _ = _self.send_stdout(&ErrorMessage::new(
                                    Some(id),
                                    format!("err from {}: {err}", type_name::<R>()).into(),
                                ));
                            }
                        }
                        _self.scheduler.finish(id);
                    }
                    Ok(WorkerMessage::Kill) => {
                        break;
                    }
                    Err(err) => {
                        lsp_log!("error: {err}");
                        _log!(_self, "error: {err}");
                        break;
                    }
                }
            }, // The receiver channel will be dropped and closed
            R::METHOD,
        );
    }

    fn start_work_done_progress(&self, title: &str) -> NumberOrString {
        let token = NumberOrString::String(title.to_string());
        let progress_token = WorkDoneProgressCreateParams {
            token: token.clone(),
        };
        let _ = self.send_stdout(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "window/workDoneProgress/create",
            "params": progress_token,
        }));
        let progress_begin = WorkDoneProgressBegin {
            title: title.to_string(),
            cancellable: Some(false),
            message: None,
            percentage: Some(0),
        };
        let params = ProgressParams {
            token: token.clone(),
            value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(progress_begin)),
        };
        let _ = self.send_stdout(&json!({
            "jsonrpc": "2.0",
            "method": "$/progress",
            "params": params,
        }));
        token
    }

    fn stop_work_done_progress(&self, token: NumberOrString, message: &str) {
        let progress_end = WorkDoneProgressEnd {
            message: Some(message.to_string()),
        };
        let params = ProgressParams {
            token,
            value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(progress_end)),
        };
        let _ = self.send_stdout(&json!({
            "jsonrpc": "2.0",
            "method": "$/progress",
            "params": params,
        }));
    }

    fn handle_request(&mut self, msg: &Value, id: i64, method: &str) -> ELSResult<()> {
        match method {
            "initialize" => self.init(msg, id),
            "shutdown" => self.shutdown(id),
            Rename::METHOD => self.rename(msg),
            Completion::METHOD => self.parse_send::<Completion>(id, msg),
            ResolveCompletionItem::METHOD => self.parse_send::<ResolveCompletionItem>(id, msg),
            GotoDefinition::METHOD => self.parse_send::<GotoDefinition>(id, msg),
            GotoTypeDefinition::METHOD => self.parse_send::<GotoTypeDefinition>(id, msg),
            GotoImplementation::METHOD => self.parse_send::<GotoImplementation>(id, msg),
            HoverRequest::METHOD => self.parse_send::<HoverRequest>(id, msg),
            References::METHOD => self.parse_send::<References>(id, msg),
            SemanticTokensFullRequest::METHOD => {
                self.parse_send::<SemanticTokensFullRequest>(id, msg)
            }
            InlayHintRequest::METHOD => self.parse_send::<InlayHintRequest>(id, msg),
            InlayHintResolveRequest::METHOD => self.parse_send::<InlayHintResolveRequest>(id, msg),
            CodeActionRequest::METHOD => self.parse_send::<CodeActionRequest>(id, msg),
            CodeActionResolveRequest::METHOD => {
                self.parse_send::<CodeActionResolveRequest>(id, msg)
            }
            SignatureHelpRequest::METHOD => self.parse_send::<SignatureHelpRequest>(id, msg),
            CodeLensRequest::METHOD => self.parse_send::<CodeLensRequest>(id, msg),
            WillRenameFiles::METHOD => self.parse_send::<WillRenameFiles>(id, msg),
            ExecuteCommand::METHOD => self.parse_send::<ExecuteCommand>(id, msg),
            WorkspaceSymbol::METHOD => self.parse_send::<WorkspaceSymbol>(id, msg),
            DocumentSymbolRequest::METHOD => self.parse_send::<DocumentSymbolRequest>(id, msg),
            CallHierarchyIncomingCalls::METHOD => {
                self.parse_send::<CallHierarchyIncomingCalls>(id, msg)
            }
            CallHierarchyOutgoingCalls::METHOD => {
                self.parse_send::<CallHierarchyOutgoingCalls>(id, msg)
            }
            CallHierarchyPrepare::METHOD => self.parse_send::<CallHierarchyPrepare>(id, msg),
            FoldingRangeRequest::METHOD => self.parse_send::<FoldingRangeRequest>(id, msg),
            SelectionRangeRequest::METHOD => self.parse_send::<SelectionRangeRequest>(id, msg),
            DocumentHighlightRequest::METHOD => {
                self.parse_send::<DocumentHighlightRequest>(id, msg)
            }
            DocumentLinkRequest::METHOD => self.parse_send::<DocumentLinkRequest>(id, msg),
            other => self.send_error(Some(id), -32600, format!("{other} is not supported")),
        }
    }

    fn handle_notification(&mut self, msg: &Value, method: &str) -> ELSResult<()> {
        match method {
            "initialized" => {
                self.flags.client_initialized.store(true, Ordering::Relaxed);
                self.ask_auto_save()?;
                self.send_log("successfully bound")
            }
            "exit" => self.exit(),
            "textDocument/didOpen" => {
                while !self.flags.workspace_checked() {
                    safe_yield();
                }
                let params = DidOpenTextDocumentParams::deserialize(msg["params"].clone())?;
                let uri = NormalizedUrl::new(params.text_document.uri);
                self.send_log(format!("{method}: {uri}"))?;
                let code = params.text_document.text;
                let ver = params.text_document.version;
                self.file_cache.update(&uri, code.clone(), Some(ver));
                let token = self.start_work_done_progress("checking files ...");
                let mut checked = Set::new();
                let res = self.check_file(uri.clone(), code, &mut checked);
                // self.send_empty_diagnostics(checked)?;
                self.stop_work_done_progress(token, "checking done");
                res
            }
            "textDocument/didSave" => {
                let params = DidSaveTextDocumentParams::deserialize(msg["params"].clone())?;
                let uri = NormalizedUrl::new(params.text_document.uri);
                self.send_log(format!("{method}: {uri}"))?;
                let code = self.file_cache.get_entire_code(&uri)?;
                let token = self.start_work_done_progress("checking files ...");
                let res = self.recheck_file(uri, code);
                self.stop_work_done_progress(token, "checking done");
                res
            }
            "textDocument/didChange" => {
                let params = DidChangeTextDocumentParams::deserialize(msg["params"].clone())?;
                // Check before updating, because `x.`/`x::` will result in an error
                // Checking should only be performed when needed for completion, i.e., when a trigger character is entered or at the beginning of a line
                if TRIGGER_CHARS.contains(&&params.content_changes[0].text[..])
                    || params.content_changes[0]
                        .range
                        .is_some_and(|r| r.start.character == 0)
                {
                    let uri = NormalizedUrl::new(params.text_document.uri.clone());
                    // TODO: reset mutable dependent types
                    self.quick_check_file(uri)?;
                }
                self.file_cache.incremental_update(params);
                Ok(())
            }
            "$/cancelRequest" => {
                let id = msg["params"]["id"].as_i64().unwrap();
                let task = self.scheduler.cancel(id);
                if task.is_some() {
                    _log!(self, "cancellation approved: {id}");
                }
                Ok(())
            }
            _ => self.send_log(format!("received notification: {method}")),
        }
    }

    fn handle_response(&mut self, id: i64, msg: &Value) -> ELSResult<()> {
        match id {
            HEALTH_CHECKER_ID => {
                self.channels
                    .as_ref()
                    .ok_or("channels not found")?
                    .health_check
                    .send(WorkerMessage::Request(0, ()))?;
            }
            _ => {
                _log!(self, "msg: {msg}");
                if msg.get("error").is_none() {
                    self.client_answers.borrow_mut().insert(id, msg.clone());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn get_mod_ctx(
        &self,
        uri: &NormalizedUrl,
    ) -> Option<MappedRwLockReadGuard<ModuleContext>> {
        let path = uri.to_file_path().ok()?;
        let ent = self.shared.get_module(&path)?;
        Some(MappedRwLockReadGuard::map(ent, |ent| &ent.module))
    }

    pub(crate) fn get_raw_mod_ctx(
        &self,
        uri: &NormalizedUrl,
        timeout: std::time::Duration,
    ) -> Option<&ModuleContext> {
        let path = uri.to_file_path().ok()?;
        self.shared.raw_ref_ctx_with_timeout(&path, timeout)
    }

    /// TODO: Reuse cache.
    /// Because of the difficulty of caching "transitional types" such as assert casting and mutable dependent types,
    /// the cache is deleted after each analysis.
    pub(crate) fn get_checker(&self, path: PathBuf) -> Checker {
        let shared = self.shared.clone();
        shared.clear(&NormalizedPathBuf::from(&path));
        Checker::inherit(self.cfg.inherit(path), shared)
    }

    pub(crate) fn steal_lowerer(&mut self, uri: &NormalizedUrl) -> Option<(ASTLowerer, IRs)> {
        let path = uri.to_file_path().ok()?;
        let module = self.shared.remove_module(&path)?;
        let lowerer = ASTLowerer::new_with_ctx(module.module);
        Some((
            lowerer,
            IRs::new(module.id, module.ast, module.hir, module.status),
        ))
    }

    pub(crate) fn restore_lowerer(
        &mut self,
        uri: NormalizedUrl,
        mut lowerer: ASTLowerer,
        irs: IRs,
    ) {
        let module = lowerer.pop_mod_ctx().unwrap();
        let entry = ModuleEntry::new(irs.id, irs.ast, irs.hir, module, irs.status);
        self.restore_entry(uri, entry);
    }

    pub(crate) fn get_visitor(&self, uri: &NormalizedUrl) -> Option<HIRVisitor> {
        let path = uri.to_file_path().ok()?;
        let Some(ent) = self.shared.get_module(&path) else {
            _log!(self, "module not found: {uri}");
            return None;
        };
        ent.hir.as_ref()?;
        let hir = MappedRwLockReadGuard::map(ent, |ent| ent.hir.as_ref().unwrap());
        Some(HIRVisitor::new(hir, &self.file_cache, uri.clone()))
    }

    pub(crate) fn get_searcher(&self, uri: &NormalizedUrl, kind: ExprKind) -> Option<HIRVisitor> {
        let path = uri.to_file_path().ok()?;
        let ent = self.shared.get_module(&path)?;
        ent.hir.as_ref()?;
        let hir = MappedRwLockReadGuard::map(ent, |ent| ent.hir.as_ref().unwrap());
        Some(HIRVisitor::new_searcher(
            hir,
            &self.file_cache,
            uri.clone(),
            kind,
        ))
    }

    pub(crate) fn get_local_ctx(&self, uri: &NormalizedUrl, pos: Position) -> Vec<&Context> {
        let mut ctxs = vec![];
        if let Some(mod_ctx) = self.get_raw_mod_ctx(uri, Duration::from_millis(100)) {
            if let Some(visitor) = self.get_visitor(uri) {
                // FIXME:
                let mut ns = visitor.get_namespace(pos);
                if &mod_ctx.context.name[..] == "<module>" {
                    ns[0] = "<module>".into();
                }
                for i in 1..ns.len() {
                    let ns = ns[..=ns.len() - i].join("");
                    if let Some(ctx) = mod_ctx.scope.get(&ns[..]) {
                        ctxs.push(ctx);
                    }
                }
            }
            ctxs.push(&mod_ctx.context);
        }
        let builtin_ctx = self.get_raw_builtin_module();
        ctxs.extend(builtin_ctx);
        ctxs
    }

    pub(crate) fn _get_all_ctxs(&self) -> Vec<&ModuleContext> {
        let mut ctxs = vec![];
        ctxs.extend(self.shared.raw_modules().map(|ent| &ent.module));
        ctxs
    }

    pub(crate) fn get_workspace_ctxs(&self) -> Vec<&Context> {
        let project_root = project_entry_dir_of(&self.home).unwrap_or(self.home.clone());
        let mut ctxs = vec![];
        for (path, ent) in self.shared.raw_path_and_modules() {
            if path.starts_with(&project_root) {
                ctxs.push(&ent.module.context);
            }
        }
        ctxs
    }

    /// the ctx of `uri` is not included
    pub(crate) fn get_neighbor_ctxs(&self, uri: &NormalizedUrl) -> Vec<&Context> {
        let mut ctxs = vec![];
        if let Ok(dir) = uri
            .to_file_path()
            .and_then(|p| p.parent().ok_or(())?.read_dir().map_err(|_| ()))
        {
            for neighbor in dir {
                let Ok(neighbor) = neighbor else {
                    continue;
                };
                let neighbor_uri = NormalizedUrl::from_file_path(neighbor.path()).unwrap();
                if &neighbor_uri == uri {
                    continue;
                }
                if let Some(mod_ctx) =
                    self.get_raw_mod_ctx(&neighbor_uri, Duration::from_millis(50))
                {
                    ctxs.push(&mod_ctx.context);
                }
            }
        }
        ctxs
    }

    pub(crate) fn get_receiver(
        &self,
        uri: &NormalizedUrl,
        attr_marker_pos: Position,
    ) -> ELSResult<Option<Expr>> {
        let maybe_token = self.file_cache.get_receiver(uri, attr_marker_pos);
        if let Some(token) = maybe_token {
            let expr = if let Some(visitor) = self.get_visitor(uri) {
                if let Some(expr) =
                    loc_to_pos(token.loc()).and_then(|pos| visitor.get_min_expr(pos).cloned())
                {
                    Some(expr)
                } else {
                    _log!(self, "expr not found: {token}");
                    None
                }
            } else {
                None
            };
            Ok(expr)
        } else {
            self.send_log("token not found")?;
            Ok(None)
        }
    }

    pub(crate) fn get_receiver_and_ctxs(
        &self,
        uri: &NormalizedUrl,
        attr_marker_pos: Position,
    ) -> ELSResult<(Option<Type>, Vec<&Context>)> {
        let Some(module) = self.get_raw_mod_ctx(uri, Duration::from_millis(100)) else {
            return Ok((None, vec![]));
        };
        let maybe_token = self.file_cache.get_receiver(uri, attr_marker_pos);
        if let Some(token) = maybe_token {
            // _log!(self, "token: {token}");
            let mut ctxs = vec![];
            let expr = if let Some(visitor) = self.get_visitor(uri) {
                if let Some(expr) =
                    loc_to_pos(token.loc()).and_then(|pos| visitor.get_min_expr(pos))
                {
                    let type_ctxs = module
                        .context
                        .get_nominal_super_type_ctxs(expr.ref_t())
                        .unwrap_or(vec![]);
                    ctxs.extend(type_ctxs.into_iter().map(|ctx| &ctx.ctx));
                    if let Ok(singular_ctxs) = module
                        .context
                        .get_singular_ctxs_by_hir_expr(expr, &module.context)
                    {
                        ctxs.extend(singular_ctxs);
                    }
                    Some(expr.t())
                } else {
                    _log!(self, "expr not found: {token}");
                    None
                }
            } else {
                None
            };
            Ok((expr, ctxs))
        } else {
            self.send_log("token not found")?;
            Ok((None, vec![]))
        }
    }

    pub(crate) fn get_raw_builtin_module(&self) -> Option<&Context> {
        self.shared.raw_ref_builtins_ctx().map(|mc| &mc.context)
    }

    pub(crate) fn clear_cache(&mut self, uri: &NormalizedUrl) {
        let path = NormalizedPathBuf::from(util::uri_to_path(uri));
        self.shared.clear(&path);
    }

    #[allow(unused)]
    pub fn get_file_cache(&self) -> &FileCache {
        &self.file_cache
    }

    pub fn remove_module_entry(&mut self, uri: &NormalizedUrl) -> Option<ModuleEntry> {
        let path = uri.to_file_path().ok()?;
        self.shared.remove_module(&path)
    }

    pub fn insert_module_entry(&mut self, uri: NormalizedUrl, entry: ModuleEntry) {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        self.shared.insert_module(path.into(), entry);
    }

    pub fn get_hir(&self, uri: &NormalizedUrl) -> Option<MappedRwLockReadGuard<HIR>> {
        let path = uri.to_file_path().ok()?;
        let ent = self.shared.get_module(&path)?;
        MappedRwLockReadGuard::try_map(ent, |ent| ent.hir.as_ref()).ok()
    }

    pub fn steal_entry(&self, uri: &NormalizedUrl) -> Option<ModuleEntry> {
        let path = uri.to_file_path().ok()?;
        self.shared.remove_module(&path)
    }

    pub fn restore_entry(&self, uri: NormalizedUrl, entry: ModuleEntry) {
        let path = uri.to_file_path().unwrap();
        self.shared.insert_module(path.into(), entry);
    }

    pub fn get_ast(&self, uri: &NormalizedUrl) -> Option<MappedRwLockReadGuard<Module>> {
        let path = uri.to_file_path().ok()?;
        let ent = self.shared.get_module(&path)?;
        MappedRwLockReadGuard::try_map(ent, |ent| ent.ast.as_ref()).ok()
    }

    pub fn get_warns(&self, uri: &NormalizedUrl) -> Option<Vec<&CompileWarning>> {
        let path = NormalizedPathBuf::from(uri.to_file_path().ok()?);
        let warns = self.shared.warns.raw_iter();
        let warns = warns.filter(|warn| NormalizedPathBuf::from(warn.input.path()) == path);
        Some(warns.collect())
    }
}
