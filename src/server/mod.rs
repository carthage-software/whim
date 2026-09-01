mod analysis;
mod completion;
mod error;
mod folding;
mod highlight;
mod selection;
mod semantic;
mod text;

use std::collections::HashMap;

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
use lsp_types::DidChangeTextDocumentParams;
use lsp_types::DocumentFormattingParams;
use lsp_types::DocumentHighlightParams;
use lsp_types::FoldingRangeParams;
use lsp_types::FoldingRangeProviderCapability;
use lsp_types::InitializeResult;
use lsp_types::OneOf;
use lsp_types::PositionEncodingKind;
use lsp_types::SelectionRangeParams;
use lsp_types::SemanticTokensFullOptions;
use lsp_types::SemanticTokensOptions;
use lsp_types::SemanticTokensParams;
use lsp_types::SemanticTokensResult;
use lsp_types::ServerCapabilities;
use lsp_types::ServerInfo;
use lsp_types::TextDocumentSyncCapability;
use lsp_types::TextDocumentSyncKind;
use lsp_types::TextEdit;
use lsp_types::Uri;
use lsp_types::WorkDoneProgressOptions;
use lsp_types::notification::DidChangeTextDocument;
use lsp_types::notification::DidCloseTextDocument;
use lsp_types::notification::DidOpenTextDocument;
use lsp_types::notification::Notification as LspNotification;
use lsp_types::request::Completion;
use lsp_types::request::DocumentHighlightRequest;
use lsp_types::request::FoldingRangeRequest;
use lsp_types::request::Formatting;
use lsp_types::request::Request as LspRequest;
use lsp_types::request::SelectionRangeRequest;
use lsp_types::request::SemanticTokensFullRequest;
use serde::de::DeserializeOwned;
use whim_formatter::settings::FormatSettings;
use whim_syn::arena::LocalArena;

use crate::server::analysis::Analysis;
pub(crate) use crate::server::error::Error;

struct Server {
    documents: HashMap<Uri, String>,
    format: FormatSettings,
}

impl Server {
    fn new(format: FormatSettings) -> Self {
        Self {
            documents: HashMap::new(),
            format,
        }
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
                Message::Notification(notification) => self.notification(notification),
                Message::Response(response) => {
                    tracing::debug!(?response, "ignored an unexpected client response");
                }
            }
        }

        Ok(())
    }

    fn request(&self, connection: &Connection, request: ServerRequest) -> Result<(), Error> {
        match request.method.as_str() {
            Completion::METHOD => {
                respond::<Completion>(connection, request, |params| self.complete(&params))
            }
            DocumentHighlightRequest::METHOD => {
                respond::<DocumentHighlightRequest>(connection, request, |params| {
                    self.highlight(&params)
                })
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
            SemanticTokensFullRequest::METHOD => {
                respond::<SemanticTokensFullRequest>(connection, request, |params| {
                    self.semantic_tokens(&params)
                })
            }
            _ => send_error(
                connection,
                request.id,
                ErrorCode::MethodNotFound,
                format!("unsupported language-server request `{}`", request.method),
            ),
        }
    }

    fn notification(&mut self, notification: ServerNotification) {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                match decode_notification::<DidOpenTextDocument>(notification) {
                    Ok(params) => {
                        self.documents
                            .insert(params.text_document.uri, params.text_document.text);
                    }
                    Err(error) => tracing::warn!(%error, "ignored an invalid document-open event"),
                }
            }
            DidChangeTextDocument::METHOD => {
                match decode_notification::<DidChangeTextDocument>(notification) {
                    Ok(params) => self.change(params),
                    Err(error) => {
                        tracing::warn!(%error, "ignored an invalid document-change event");
                    }
                }
            }
            DidCloseTextDocument::METHOD => {
                match decode_notification::<DidCloseTextDocument>(notification) {
                    Ok(params) => {
                        self.documents.remove(&params.text_document.uri);
                    }
                    Err(error) => tracing::warn!(%error, "ignored an invalid document-close event"),
                }
            }
            _ => {}
        }
    }

    fn change(&mut self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };

        if change.range.is_some() {
            tracing::warn!(
                uri = %params.text_document.uri.as_str(),
                "ignored an incremental change after requesting full document sync"
            );
            return;
        }

        self.documents.insert(params.text_document.uri, change.text);
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

    fn highlight(
        &self,
        params: &DocumentHighlightParams,
    ) -> Result<Option<Vec<lsp_types::DocumentHighlight>>, RequestError> {
        let position = params.text_document_position_params.position;
        let document = &params.text_document_position_params.text_document.uri;
        let analysis = Analysis::new(self.document(document)?);

        Ok(Some(highlight::occurrences(&analysis, position)))
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
        let source = self.document(document)?;
        let arena = LocalArena::new();
        let formatted = whim_formatter::format(&arena, source, self.format).map_err(|errors| {
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

    fn semantic_tokens(
        &self,
        params: &SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>, RequestError> {
        let analysis = Analysis::new(self.document(&params.text_document.uri)?);

        Ok(Some(SemanticTokensResult::Tokens(semantic::tokens(
            &analysis,
        ))))
    }

    fn document(&self, uri: &Uri) -> Result<&str, RequestError> {
        self.documents
            .get(uri)
            .map(String::as_str)
            .ok_or_else(|| RequestError::DocumentNotOpen(uri.as_str().to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
enum RequestError {
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
            Self::Format { .. } => ErrorCode::RequestFailed,
        }
    }
}

#[tracing::instrument(level = "debug", skip_all)]
pub(crate) fn serve(format: FormatSettings) -> Result<(), Error> {
    let (connection, threads) = Connection::stdio();
    initialize(&connection)?;
    tracing::debug!("language server initialized");

    let result = Server::new(format).run(&connection);
    drop(connection);
    let transport = threads.join().map_err(Error::Transport);

    result?;
    transport?;
    tracing::debug!("language server stopped");
    Ok(())
}

fn initialize(connection: &Connection) -> Result<(), Error> {
    let (id, _) = connection.initialize_start()?;
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

    Ok(())
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(PositionEncodingKind::UTF16),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions::default()),
        document_highlight_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        selection_range_provider: Some(true.into()),
        semantic_tokens_provider: Some(
            SemanticTokensOptions {
                work_done_progress_options: WorkDoneProgressOptions::default(),
                legend: semantic::legend(),
                range: None,
                full: Some(SemanticTokensFullOptions::Bool(true)),
            }
            .into(),
        ),
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
        assert!(capabilities.document_formatting_provider.is_some());
        assert!(capabilities.document_highlight_provider.is_some());
        assert!(capabilities.folding_range_provider.is_some());
        assert!(capabilities.selection_range_provider.is_some());
        assert!(capabilities.semantic_tokens_provider.is_some());
        assert!(capabilities.definition_provider.is_none());
        assert!(capabilities.hover_provider.is_none());
        assert!(capabilities.references_provider.is_none());
        assert!(capabilities.rename_provider.is_none());
        assert!(capabilities.document_symbol_provider.is_none());
    }
}
