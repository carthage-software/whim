mod analysis;
mod completion;
mod diagnostics;
mod error;
mod folding;
mod project;
#[cfg(test)]
mod project_tests;
mod selection;
mod text;

use std::collections::HashMap;
use std::io::Error as IoError;
use std::path::Path;
use std::path::PathBuf;

use lsp_server::Connection;
use lsp_server::ErrorCode;
use lsp_server::Message;
use lsp_server::Notification as ServerNotification;
use lsp_server::Request as ServerRequest;
use lsp_server::RequestId;
use lsp_server::Response;
use lsp_server::ResponseError;
use lsp_types::CompletionOptions;
use lsp_types::CompletionResponse;
use lsp_types::DiagnosticOptions;
use lsp_types::DiagnosticServerCapabilities;
use lsp_types::DidChangeTextDocumentParams;
use lsp_types::DocumentDiagnosticReport;
use lsp_types::DocumentDiagnosticReportResult;
use lsp_types::DocumentFormattingParams;
use lsp_types::FoldingRangeParams;
use lsp_types::FoldingRangeProviderCapability;
use lsp_types::FullDocumentDiagnosticReport;
use lsp_types::InitializeResult;
use lsp_types::MessageType;
use lsp_types::OneOf;
use lsp_types::PositionEncodingKind;
use lsp_types::PublishDiagnosticsParams;
use lsp_types::RelatedFullDocumentDiagnosticReport;
use lsp_types::SelectionRangeParams;
use lsp_types::ServerCapabilities;
use lsp_types::ServerInfo;
use lsp_types::ShowMessageParams;
use lsp_types::TextDocumentSyncCapability;
use lsp_types::TextDocumentSyncKind;
use lsp_types::TextEdit;
use lsp_types::Uri;
use lsp_types::notification::DidChangeTextDocument;
use lsp_types::notification::DidCloseTextDocument;
use lsp_types::notification::DidOpenTextDocument;
use lsp_types::notification::Notification as LspNotification;
use lsp_types::notification::PublishDiagnostics;
use lsp_types::notification::ShowMessage;
use lsp_types::request::Completion;
use lsp_types::request::DocumentDiagnosticRequest;
use lsp_types::request::FoldingRangeRequest;
use lsp_types::request::Formatting;
use lsp_types::request::Request as LspRequest;
use lsp_types::request::SelectionRangeRequest;
use lsp_types::request::WorkspaceDiagnosticRequest;
use serde::de::DeserializeOwned;
use whim_syn::arena::LocalArena;

use crate::server::analysis::Analysis;
pub(crate) use crate::server::error::Error;
use crate::server::project::Project;

struct Server {
    documents: HashMap<Uri, Document>,
    projects: HashMap<PathBuf, Option<Project>>,
    standalone: Project,
}

struct Document {
    text: String,
    version: i32,
}

impl Server {
    fn new() -> Result<Self, Error> {
        Ok(Self {
            documents: HashMap::new(),
            projects: HashMap::new(),
            standalone: Project::standalone()?,
        })
    }

    fn add_project(
        &mut self,
        connection: &Connection,
        root: PathBuf,
        configuration: Option<&Path>,
    ) -> Result<(), Error> {
        let root = root.canonicalize().unwrap_or(root);
        let project = match Project::load(&root, configuration) {
            Ok(project) => {
                if project.has_manifest {
                    tracing::info!(root = %root.display(), configuration_root = %project.root.display(), "loaded language-server project");
                } else {
                    show_error(
                        connection,
                        format!(
                            "No whim.toml found for `{}`. Create one with `whim init` and restart the language server to enable workspace diagnostics. Only open files will be checked until then.",
                            root.display(),
                        ),
                    )?;
                }
                Some(project)
            }
            Err(error) => {
                show_error(
                    connection,
                    format!(
                        "Could not load Whim settings for `{}`: {error}. Fix the config and restart the language server. Formatting and diagnostics are disabled for this project.",
                        root.display(),
                    ),
                )?;
                None
            }
        };
        self.projects.insert(root, project);
        Ok(())
    }

    fn project(&self, uri: &Uri) -> Option<&Project> {
        let path = diagnostics::file_path(uri);
        path.as_ref()
            .and_then(|path| {
                self.projects
                    .iter()
                    .filter_map(|(root, project)| {
                        let root = if path.starts_with(root) {
                            root
                        } else {
                            &project
                                .as_ref()
                                .filter(|project| path.starts_with(&project.root))?
                                .root
                        };
                        Some((root.components().count(), project))
                    })
                    .max_by_key(|(depth, _)| *depth)
            })
            .map_or(Some(&self.standalone), |(_, project)| project.as_ref())
    }

    fn run(&mut self, connection: &Connection) -> Result<(), Error> {
        for message in &connection.receiver {
            match message {
                Message::Request(request) => {
                    if connection.handle_shutdown(&request)? {
                        return Ok(());
                    }

                    self.request(connection, request)?;
                }
                Message::Notification(notification) => {
                    self.notification(connection, notification)?;
                }
                Message::Response(response) => {
                    tracing::debug!(?response, "ignored an unexpected client response");
                }
            }
        }

        Ok(())
    }

    fn request(&self, connection: &Connection, request: ServerRequest) -> Result<(), Error> {
        match request.method.as_str() {
            DocumentDiagnosticRequest::METHOD => {
                respond::<DocumentDiagnosticRequest>(connection, request, |params| {
                    Ok(DocumentDiagnosticReportResult::Report(
                        DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                            related_documents: None,
                            full_document_diagnostic_report: FullDocumentDiagnosticReport {
                                result_id: None,
                                items: self.document_diagnostics(&params.text_document.uri)?,
                            },
                        }),
                    ))
                })
            }
            WorkspaceDiagnosticRequest::METHOD => {
                respond::<WorkspaceDiagnosticRequest>(connection, request, |params| {
                    self.workspace_diagnostics(&params)
                })
            }
            Completion::METHOD => {
                respond::<Completion>(connection, request, |params| self.complete(&params))
            }
            FoldingRangeRequest::METHOD => {
                respond::<FoldingRangeRequest>(connection, request, |params| self.fold(&params))
            }
            Formatting::METHOD => {
                respond::<Formatting>(connection, request, |params| self.format(&params))
            }
            SelectionRangeRequest::METHOD => {
                respond::<SelectionRangeRequest>(connection, request, |params| self.select(&params))
            }
            _ => send_error(
                connection,
                request.id,
                ErrorCode::MethodNotFound,
                format!("unsupported language-server request `{}`", request.method),
            ),
        }
    }

    fn notification(
        &mut self,
        connection: &Connection,
        notification: ServerNotification,
    ) -> Result<(), Error> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                match decode_notification::<DidOpenTextDocument>(notification) {
                    Ok(params) => {
                        let document = params.text_document;
                        let uri = document.uri;
                        self.documents.insert(
                            uri.clone(),
                            Document {
                                text: document.text,
                                version: document.version,
                            },
                        );
                        self.publish(connection, &uri)?;
                    }
                    Err(error) => tracing::warn!(%error, "ignored an invalid document-open event"),
                }
            }
            DidChangeTextDocument::METHOD => {
                match decode_notification::<DidChangeTextDocument>(notification) {
                    Ok(params) => {
                        let uri = params.text_document.uri.clone();
                        self.change(params);
                        self.publish(connection, &uri)?;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "ignored an invalid document-change event");
                    }
                }
            }
            DidCloseTextDocument::METHOD => {
                match decode_notification::<DidCloseTextDocument>(notification) {
                    Ok(params) => {
                        self.documents.remove(&params.text_document.uri);
                        publish(
                            connection,
                            PublishDiagnosticsParams::new(
                                params.text_document.uri,
                                Vec::new(),
                                None,
                            ),
                        )?;
                    }
                    Err(error) => tracing::warn!(%error, "ignored an invalid document-close event"),
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn publish(&self, connection: &Connection, uri: &Uri) -> Result<(), Error> {
        match self.document_diagnostics(uri) {
            Ok(diagnostics) => publish(
                connection,
                PublishDiagnosticsParams::new(
                    uri.clone(),
                    diagnostics,
                    self.documents.get(uri).map(|document| document.version),
                ),
            ),
            Err(error) => {
                tracing::warn!(%error, "could not lint document");
                Ok(())
            }
        }
    }

    fn change(&mut self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        if change.range.is_some() {
            tracing::warn!(uri = %params.text_document.uri.as_str(), "ignored an incremental change after requesting full document sync");
            return;
        }
        self.documents.insert(
            params.text_document.uri,
            Document {
                text: change.text,
                version: params.text_document.version,
            },
        );
    }

    fn complete(
        &self,
        params: &lsp_types::CompletionParams,
    ) -> Result<Option<CompletionResponse>, RequestError> {
        let document = &params.text_document_position.text_document.uri;
        let source = self.document(document)?;
        let analysis = Analysis::new(source);
        let offset = analysis
            .lines()
            .offset(params.text_document_position.position);

        Ok(Some(CompletionResponse::List(completion::items(
            &analysis, offset,
        ))))
    }

    fn fold(
        &self,
        params: &FoldingRangeParams,
    ) -> Result<Option<Vec<lsp_types::FoldingRange>>, RequestError> {
        let analysis = Analysis::new(self.document(&params.text_document.uri)?);

        Ok(Some(folding::ranges(&analysis)))
    }

    fn format(
        &self,
        params: &DocumentFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>, RequestError> {
        let document = &params.text_document.uri;
        let Some(project) = self
            .project(document)
            .filter(|project| project.can_format(document))
        else {
            return Ok(None);
        };
        let source = self.document(document)?;
        let arena = LocalArena::new();
        let formatted =
            whim_formatter::format(&arena, source, project.format).map_err(|errors| {
                RequestError::Format {
                    document: document.as_str().to_owned(),
                    diagnostics: errors.render(source, document.as_str()),
                }
            })?;

        if formatted == source {
            return Ok(Some(Vec::new()));
        }

        let analysis = Analysis::new(source);
        Ok(Some(vec![TextEdit {
            range: analysis.lines().range(0, source.len()),
            new_text: formatted.to_owned(),
        }]))
    }

    fn select(
        &self,
        params: &SelectionRangeParams,
    ) -> Result<Option<Vec<lsp_types::SelectionRange>>, RequestError> {
        let analysis = Analysis::new(self.document(&params.text_document.uri)?);

        Ok(Some(selection::ranges(&analysis, &params.positions)))
    }

    fn document(&self, uri: &Uri) -> Result<&str, RequestError> {
        self.documents
            .get(uri)
            .map(|document| document.text.as_str())
            .ok_or_else(|| RequestError::DocumentNotOpen(uri.as_str().to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
enum RequestError {
    #[error("could not read `{document}`: {source}")]
    ReadDocument { document: String, source: IoError },
    #[error("could not lint workspace: {0}")]
    Workspace(String),
    #[error("document `{0}` is not open")]
    DocumentNotOpen(String),
    #[error("could not format `{document}`:\n{diagnostics}")]
    Format {
        document: String,
        diagnostics: String,
    },
}

impl RequestError {
    const fn code(&self) -> ErrorCode {
        match self {
            Self::DocumentNotOpen(_) => ErrorCode::InvalidRequest,
            Self::Format { .. } | Self::ReadDocument { .. } | Self::Workspace(_) => {
                ErrorCode::RequestFailed
            }
        }
    }
}

#[tracing::instrument(level = "debug", skip_all)]
pub(crate) fn serve(configuration: Option<&Path>) -> Result<(), Error> {
    let (connection, threads) = Connection::stdio();
    let mut server = initialize(&connection, configuration)?;
    tracing::debug!("language server initialized");

    let result = server.run(&connection);
    drop(connection);
    let transport = threads.join().map_err(Error::Transport);

    result?;
    transport?;
    tracing::debug!("language server stopped");
    Ok(())
}

fn initialize(connection: &Connection, configuration: Option<&Path>) -> Result<Server, Error> {
    let (id, params) = connection.initialize_start()?;
    let mut roots = Vec::new();
    if let Some(folders) = params
        .get("workspaceFolders")
        .and_then(serde_json::Value::as_array)
    {
        for folder in folders {
            if let Some(uri) = folder
                .get("uri")
                .and_then(serde_json::Value::as_str)
                .and_then(|uri| uri.parse().ok())
                && let Some(path) = diagnostics::file_path(&uri)
            {
                roots.push(path);
            }
        }
    }
    if roots.is_empty()
        && let Some(uri) = params
            .get("rootUri")
            .and_then(serde_json::Value::as_str)
            .and_then(|uri| uri.parse().ok())
        && let Some(path) = diagnostics::file_path(&uri)
    {
        roots.push(path);
    }
    if roots.is_empty()
        && let Some(root) = configuration.and_then(Path::parent)
    {
        roots.push(root.to_owned());
    }
    let result = InitializeResult {
        capabilities: capabilities(),
        server_info: Some(ServerInfo {
            name: "Whim".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }),
    };
    let value = serde_json::to_value(result).map_err(|source| Error::EncodeResponse {
        method: "initialize".to_owned(),
        source,
    })?;
    connection.initialize_finish(id, value)?;

    let mut server = Server::new()?;
    for root in roots {
        server.add_project(connection, root, configuration)?;
    }
    Ok(server)
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        diagnostic_provider: Some(DiagnosticServerCapabilities::Options(DiagnosticOptions {
            identifier: Some("whim".to_owned()),
            inter_file_dependencies: false,
            workspace_diagnostics: true,
            ..DiagnosticOptions::default()
        })),
        position_encoding: Some(PositionEncodingKind::UTF16),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions::default()),
        document_formatting_provider: Some(OneOf::Left(true)),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        selection_range_provider: Some(true.into()),
        ..ServerCapabilities::default()
    }
}

fn respond<R>(
    connection: &Connection,
    request: ServerRequest,
    handler: impl FnOnce(R::Params) -> Result<R::Result, RequestError>,
) -> Result<(), Error>
where
    R: LspRequest,
{
    let id = request.id;
    let params = match serde_json::from_value(request.params) {
        Ok(params) => params,
        Err(error) => {
            return send_error(
                connection,
                id,
                ErrorCode::InvalidParams,
                format!("invalid parameters for `{}`: {error}", R::METHOD),
            );
        }
    };
    let response = match handler(params) {
        Ok(result) => {
            let result = serde_json::to_value(result).map_err(|source| Error::EncodeResponse {
                method: R::METHOD.to_owned(),
                source,
            })?;
            Response {
                id,
                response_result: Ok(result),
            }
        }
        Err(error) => Response {
            id,
            response_result: Err(ResponseError {
                code: error.code() as i32,
                message: error.to_string(),
                data: None,
            }),
        },
    };

    send(connection, response)
}

fn publish(connection: &Connection, params: PublishDiagnosticsParams) -> Result<(), Error> {
    connection
        .sender
        .send(Message::Notification(ServerNotification::new(
            PublishDiagnostics::METHOD.to_owned(),
            params,
        )))
        .map_err(|_disconnected| Error::Disconnected)
}

fn show_error(connection: &Connection, message: String) -> Result<(), Error> {
    tracing::error!("{message}");
    connection
        .sender
        .send(Message::Notification(ServerNotification::new(
            ShowMessage::METHOD.to_owned(),
            ShowMessageParams {
                typ: MessageType::ERROR,
                message,
            },
        )))
        .map_err(|_disconnected| Error::Disconnected)
}

fn send_error(
    connection: &Connection,
    id: RequestId,
    code: ErrorCode,
    message: String,
) -> Result<(), Error> {
    send(connection, Response::new_err(id, code as i32, message))
}

fn send(connection: &Connection, response: Response) -> Result<(), Error> {
    connection
        .sender
        .send(Message::Response(response))
        .map_err(|_disconnected| Error::Disconnected)
}

fn decode_notification<N>(notification: ServerNotification) -> Result<N::Params, serde_json::Error>
where
    N: LspNotification,
    N::Params: DeserializeOwned,
{
    serde_json::from_value(notification.params)
}

#[cfg(test)]
mod tests {
    use lsp_types::TextDocumentSyncCapability;
    use lsp_types::TextDocumentSyncKind;

    use super::capabilities;

    #[test]
    fn the_server_exposes_only_local_editor_features() {
        let capabilities = capabilities();
        assert_eq!(
            capabilities.text_document_sync,
            Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL))
        );
        assert!(capabilities.completion_provider.is_some());
        assert!(capabilities.diagnostic_provider.is_some());
        assert!(capabilities.document_formatting_provider.is_some());
        assert!(capabilities.folding_range_provider.is_some());
        assert!(capabilities.selection_range_provider.is_some());
        assert!(capabilities.definition_provider.is_none());
        assert!(capabilities.hover_provider.is_none());
        assert!(capabilities.references_provider.is_none());
        assert!(capabilities.rename_provider.is_none());
        assert!(capabilities.document_symbol_provider.is_none());
    }
}
