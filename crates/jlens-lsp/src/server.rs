//! The server: the message loop, the open documents and the requests.

use std::collections::HashMap;
use std::path::PathBuf;

use jlens_schema::{CompletionKind, LanguageService};
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument,
    DidSaveTextDocument, LogMessage, Notification as _, PublishDiagnostics, ShowMessage,
};
use lsp_types::request::{
    Completion, HoverRequest, RegisterCapability, Request as _, SignatureHelpRequest,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, Documentation, FileSystemWatcher,
    GlobPattern, Hover, HoverContents, HoverParams, HoverProviderCapability, InitializeParams,
    LogMessageParams, MarkupContent, MarkupKind, MessageType, NumberOrString, ParameterInformation,
    ParameterLabel, PositionEncodingKind, PublishDiagnosticsParams, Registration,
    RegistrationParams, ServerCapabilities, ServerInfo, ShowMessageParams, SignatureHelp,
    SignatureHelpOptions, SignatureHelpParams, SignatureInformation, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Uri,
};
use serde_json::Value;

use crate::config::{self, Entry};
use crate::text::{Encoding, LineIndex, apply_changes};

/// The error type of the server.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

struct Document {
    text: String,
    version: i32,
}

/// Where a document's language service comes from.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
    /// An entry of the initialization options.
    Init(usize),
    /// An entry of the `jlens.json` of a workspace folder.
    Folder(usize, usize),
    /// No configuration applies.
    Default,
}

struct Server<'c> {
    connection: &'c Connection,
    encoding: Encoding,
    /// The workspace folders, each with the entries of its `jlens.json`.
    folders: Vec<(PathBuf, Vec<Entry>)>,
    /// The entries of the initialization options, relative to the first folder.
    init: Vec<Entry>,
    documents: HashMap<Uri, Document>,
    services: HashMap<Source, LanguageService>,
    next_request: i32,
}

/// Runs the server on `connection` until the client asks it to exit: answers `initialize`, then
/// handles requests and notifications.
pub fn run(connection: &Connection) -> Result<(), Error> {
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params)?;
    let encoding = negotiate_encoding(&params);
    let capabilities = capabilities(encoding);
    connection.initialize_finish(
        id,
        serde_json::json!({
            "capabilities": capabilities,
            "serverInfo": ServerInfo {
                name: "jlens".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            },
        }),
    )?;

    let mut server = Server {
        connection,
        encoding,
        folders: folders(&params),
        init: Vec::new(),
        documents: HashMap::new(),
        services: HashMap::new(),
        next_request: 0,
    };
    if let Some(options) = params
        .initialization_options
        .clone()
        .filter(|o| !o.is_null())
    {
        let root = server.folders.first().map_or_else(
            || std::env::current_dir().unwrap_or_default(),
            |(r, _)| r.clone(),
        );
        match config::parse(&options, &root) {
            Ok(entries) => server.init = entries,
            Err(e) => server.warn(format!("jlens initialization options: {e}"))?,
        }
    }
    server.load_folders()?;
    if supports_watching(&params) {
        server.watch_configuration()?;
    }

    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let response = server.request(request);
                connection.sender.send(response.into())?;
            }
            Message::Notification(notification) => server.notification(notification)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// UTF-8 when the client offers it, UTF-16 (the protocol's default) otherwise.
fn negotiate_encoding(params: &InitializeParams) -> Encoding {
    let offered = params
        .capabilities
        .general
        .as_ref()
        .and_then(|g| g.position_encodings.as_ref());
    match offered {
        Some(kinds) if kinds.contains(&PositionEncodingKind::UTF8) => Encoding::Utf8,
        _ => Encoding::Utf16,
    }
}

fn capabilities(encoding: Encoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(match encoding {
            Encoding::Utf8 => PositionEncodingKind::UTF8,
            Encoding::Utf16 => PositionEncodingKind::UTF16,
        }),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some([".", "$", "[", "`"].map(String::from).to_vec()),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".into(), ",".into()]),
            retrigger_characters: None,
            work_done_progress_options: Default::default(),
        }),
        ..Default::default()
    }
}

/// The workspace folders, or the root of the workspace for older clients.
fn folders(params: &InitializeParams) -> Vec<(PathBuf, Vec<Entry>)> {
    #[allow(deprecated)] // `root_uri` is how clients without workspace folders send the root.
    let uris: Vec<&Uri> = match &params.workspace_folders {
        Some(folders) if !folders.is_empty() => folders.iter().map(|f| &f.uri).collect(),
        _ => params.root_uri.iter().collect(),
    };
    uris.into_iter()
        .filter_map(path_of)
        .map(|path| (path, Vec::new()))
        .collect()
}

fn supports_watching(params: &InitializeParams) -> bool {
    params
        .capabilities
        .workspace
        .as_ref()
        .and_then(|w| w.did_change_watched_files.as_ref())
        .and_then(|d| d.dynamic_registration)
        .unwrap_or(false)
}

/// The file system path of a `file:` URI.
fn path_of(uri: &Uri) -> Option<PathBuf> {
    crate::uri::path_of(uri.as_str())
}

impl Server<'_> {
    fn send_notification<N: lsp_types::notification::Notification>(
        &self,
        params: N::Params,
    ) -> Result<(), Error> {
        let notification = Notification::new(N::METHOD.into(), params);
        self.connection.sender.send(notification.into())?;
        Ok(())
    }

    fn warn(&self, message: String) -> Result<(), Error> {
        self.send_notification::<ShowMessage>(ShowMessageParams {
            typ: MessageType::WARNING,
            message,
        })
    }

    /// Reads the `jlens.json` of every folder and forgets the services built from the previous
    /// configuration.
    fn load_folders(&mut self) -> Result<(), Error> {
        let mut problems = Vec::new();
        for (root, entries) in &mut self.folders {
            *entries = config::load(root).unwrap_or_else(|e| {
                problems.push(e);
                Vec::new()
            });
        }
        self.services.clear();
        for problem in problems {
            self.warn(problem)?;
        }
        Ok(())
    }

    /// Asks the client to report changes to JSON files: configurations and schemas.
    fn watch_configuration(&mut self) -> Result<(), Error> {
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*.json".into()),
                kind: None,
            }],
        };
        let params = RegistrationParams {
            registrations: vec![Registration {
                id: "jlens-watch-json".into(),
                method: DidChangeWatchedFiles::METHOD.into(),
                register_options: Some(serde_json::to_value(options)?),
            }],
        };
        self.next_request += 1;
        let request = Request::new(
            RequestId::from(format!("jlens-{}", self.next_request)),
            RegisterCapability::METHOD.into(),
            params,
        );
        self.connection.sender.send(request.into())?;
        Ok(())
    }

    /// The language service of a document, built on first use from the configuration entry that
    /// matches its path.
    fn service(&mut self, uri: &Uri) -> Result<&LanguageService, Error> {
        let path = path_of(uri);
        let source = path.as_deref().map_or(Source::Default, |path| {
            if let Some((i, _)) = config::find(&self.init, path) {
                return Source::Init(i);
            }
            for (f, (root, entries)) in self.folders.iter().enumerate() {
                if path.starts_with(root)
                    && let Some((i, _)) = config::find(entries, path)
                {
                    return Source::Folder(f, i);
                }
            }
            Source::Default
        });
        if !self.services.contains_key(&source) {
            let (service, problems) = match source {
                Source::Init(i) => self.init[i].service(),
                Source::Folder(f, i) => self.folders[f].1[i].service(),
                Source::Default => (LanguageService::new(None), Vec::new()),
            };
            for problem in problems {
                self.warn(problem)?;
            }
            self.services.insert(source, service);
        }
        Ok(&self.services[&source])
    }

    fn publish_diagnostics(&mut self, uri: &Uri) -> Result<(), Error> {
        let Some(document) = self.documents.get(uri) else {
            return Ok(());
        };
        let (text, version) = (document.text.clone(), document.version);
        let encoding = self.encoding;
        let diagnostics = self.service(uri)?.diagnostics(&text);
        let index = LineIndex::new(&text, encoding);
        let diagnostics = diagnostics
            .into_iter()
            .map(|d| Diagnostic {
                range: index.range(d.span),
                severity: Some(match d.severity {
                    jlens_core::Severity::Error => DiagnosticSeverity::ERROR,
                    jlens_core::Severity::Warning => DiagnosticSeverity::WARNING,
                }),
                code: Some(NumberOrString::String(d.code)),
                source: Some("jlens".into()),
                message: d.message,
                ..Default::default()
            })
            .collect();
        self.send_notification::<PublishDiagnostics>(PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics,
            version: Some(version),
        })
    }

    /// The parameters of a notification; malformed ones are logged and the notification ignored.
    fn params<P: serde::de::DeserializeOwned>(
        &self,
        notification: Notification,
        method: &str,
    ) -> Result<Option<P>, Error> {
        match notification.extract::<P>(method) {
            Ok(params) => Ok(Some(params)),
            Err(e) => {
                self.send_notification::<LogMessage>(LogMessageParams {
                    typ: MessageType::ERROR,
                    message: format!("jlens: invalid {method} notification: {e}"),
                })?;
                Ok(None)
            }
        }
    }

    fn notification(&mut self, notification: Notification) -> Result<(), Error> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let Some(params) = self.params::<DidOpenTextDocumentParams>(
                    notification,
                    DidOpenTextDocument::METHOD,
                )?
                else {
                    return Ok(());
                };
                let uri = params.text_document.uri;
                self.documents.insert(
                    uri.clone(),
                    Document {
                        text: params.text_document.text,
                        version: params.text_document.version,
                    },
                );
                self.publish_diagnostics(&uri)
            }
            DidChangeTextDocument::METHOD => {
                let Some(params) = self.params::<DidChangeTextDocumentParams>(
                    notification,
                    DidChangeTextDocument::METHOD,
                )?
                else {
                    return Ok(());
                };
                let uri = params.text_document.uri;
                let encoding = self.encoding;
                if let Some(document) = self.documents.get_mut(&uri) {
                    apply_changes(&mut document.text, params.content_changes, encoding);
                    document.version = params.text_document.version;
                }
                self.publish_diagnostics(&uri)
            }
            DidCloseTextDocument::METHOD => {
                let Some(params) = self.params::<DidCloseTextDocumentParams>(
                    notification,
                    DidCloseTextDocument::METHOD,
                )?
                else {
                    return Ok(());
                };
                self.documents.remove(&params.text_document.uri);
                self.send_notification::<PublishDiagnostics>(PublishDiagnosticsParams {
                    uri: params.text_document.uri,
                    diagnostics: Vec::new(),
                    version: None,
                })
            }
            DidSaveTextDocument::METHOD => {
                let Some(params) = self.params::<DidSaveTextDocumentParams>(
                    notification,
                    DidSaveTextDocument::METHOD,
                )?
                else {
                    return Ok(());
                };
                if params.text_document.uri.as_str().ends_with(".json") {
                    self.reload()?;
                }
                Ok(())
            }
            DidChangeWatchedFiles::METHOD => {
                let Some(_) = self.params::<DidChangeWatchedFilesParams>(
                    notification,
                    DidChangeWatchedFiles::METHOD,
                )?
                else {
                    return Ok(());
                };
                self.reload()
            }
            _ => Ok(()),
        }
    }

    /// Reloads the configuration (and, through it, the schemas), and checks the open documents
    /// again.
    fn reload(&mut self) -> Result<(), Error> {
        self.load_folders()?;
        let uris: Vec<Uri> = self.documents.keys().cloned().collect();
        for uri in uris {
            self.publish_diagnostics(&uri)?;
        }
        Ok(())
    }

    fn request(&mut self, request: Request) -> Response {
        let id = request.id.clone();
        let result = match request.method.as_str() {
            Completion::METHOD => request
                .extract::<CompletionParams>(Completion::METHOD)
                .map_err(|e| e.to_string())
                .and_then(|(_, p)| self.completion(p))
                .and_then(to_value),
            HoverRequest::METHOD => request
                .extract::<HoverParams>(HoverRequest::METHOD)
                .map_err(|e| e.to_string())
                .and_then(|(_, p)| self.hover(p))
                .and_then(to_value),
            SignatureHelpRequest::METHOD => request
                .extract::<SignatureHelpParams>(SignatureHelpRequest::METHOD)
                .map_err(|e| e.to_string())
                .and_then(|(_, p)| self.signature_help(p))
                .and_then(to_value),
            method => {
                return Response::new_err(
                    id,
                    ErrorCode::MethodNotFound as i32,
                    format!("unsupported request: {method}"),
                );
            }
        };
        match result {
            Ok(value) => Response::new_ok(id, value),
            Err(message) => Response::new_err(id, ErrorCode::InvalidParams as i32, message),
        }
    }

    /// The text of an open document, and its language service.
    fn document(&mut self, uri: &Uri) -> Result<Option<(String, &LanguageService)>, String> {
        let Some(document) = self.documents.get(uri) else {
            return Ok(None);
        };
        let text = document.text.clone();
        let service = self.service(uri).map_err(|e| e.to_string())?;
        Ok(Some((text, service)))
    }

    fn completion(
        &mut self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>, String> {
        let position = params.text_document_position.position;
        let encoding = self.encoding;
        let Some((text, service)) =
            self.document(&params.text_document_position.text_document.uri)?
        else {
            return Ok(None);
        };
        let index = LineIndex::new(&text, encoding);
        let completions = service.complete(&text, index.offset(position));
        let range = index.range(completions.range);
        let items = completions
            .items
            .into_iter()
            .enumerate()
            .map(|(rank, item)| CompletionItem {
                label: item.label,
                kind: Some(match item.kind {
                    CompletionKind::Field => CompletionItemKind::FIELD,
                    CompletionKind::Variable => CompletionItemKind::VARIABLE,
                    CompletionKind::Function => CompletionItemKind::FUNCTION,
                    CompletionKind::Keyword => CompletionItemKind::KEYWORD,
                    CompletionKind::Operator => CompletionItemKind::OPERATOR,
                }),
                detail: item.detail,
                documentation: item
                    .documentation
                    .map(markdown)
                    .map(Documentation::MarkupContent),
                // The service ranks its items: keep its order.
                sort_text: Some(format!("{rank:05}")),
                text_edit: Some(TextEdit::new(range, item.insert_text).into()),
                ..Default::default()
            })
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    fn hover(&mut self, params: HoverParams) -> Result<Option<Hover>, String> {
        let position = params.text_document_position_params.position;
        let encoding = self.encoding;
        let Some((text, service)) =
            self.document(&params.text_document_position_params.text_document.uri)?
        else {
            return Ok(None);
        };
        let index = LineIndex::new(&text, encoding);
        Ok(service
            .hover(&text, index.offset(position))
            .map(|hover| Hover {
                contents: HoverContents::Markup(markdown(hover.contents)),
                range: Some(index.range(hover.range)),
            }))
    }

    fn signature_help(
        &mut self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>, String> {
        let position = params.text_document_position_params.position;
        let encoding = self.encoding;
        let Some((text, service)) =
            self.document(&params.text_document_position_params.text_document.uri)?
        else {
            return Ok(None);
        };
        let index = LineIndex::new(&text, encoding);
        let Some(help) = service.signature_help(&text, index.offset(position)) else {
            return Ok(None);
        };
        let label = format!("{}({})", help.name, help.params.join(", "));
        let mut documentation = help.summary;
        if !help.signature.is_empty() {
            documentation = format!("`{}`\n\n{documentation}", help.signature);
        }
        Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: Some(Documentation::MarkupContent(markdown(documentation))),
                parameters: Some(
                    help.params
                        .into_iter()
                        .map(|p| ParameterInformation {
                            label: ParameterLabel::Simple(p),
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter: None,
            }],
            active_signature: Some(0),
            active_parameter: Some(help.active_param as u32),
        }))
    }
}

fn markdown(value: String) -> MarkupContent {
    MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    }
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}
