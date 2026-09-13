use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use annotate_snippets::Level;
use lsp_types::Diagnostic;
use lsp_types::DiagnosticSeverity;
use lsp_types::FullDocumentDiagnosticReport;
use lsp_types::NumberOrString;
use lsp_types::Uri;
use lsp_types::WorkspaceDiagnosticParams;
use lsp_types::WorkspaceDiagnosticReport;
use lsp_types::WorkspaceDiagnosticReportResult;
use lsp_types::WorkspaceFullDocumentDiagnosticReport;
use rayon::prelude::*;
use url::Url;

use whim_linter::Linter;
use whim_linter::registry::RuleRegistry;
use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::config::FilePatterns;
use crate::pipeline::files::discover;
use crate::server::Document;
use crate::server::RequestError;
use crate::server::Server;
use crate::server::text::LineIndex;

pub(crate) struct Diagnostics {
    registry: Arc<RuleRegistry>,
    patterns: FilePatterns,
    pub(super) roots: Vec<PathBuf>,
}

impl Diagnostics {
    pub(crate) fn new(registry: RuleRegistry, patterns: FilePatterns, root: PathBuf) -> Self {
        Self {
            registry: Arc::new(registry),
            patterns,
            roots: vec![root],
        }
    }

    fn path(&self, uri: &Uri) -> Option<PathBuf> {
        let path = file_path(uri)?;
        Some(
            self.roots
                .iter()
                .find_map(|root| path.strip_prefix(root).ok())
                .unwrap_or(&path)
                .to_owned(),
        )
    }

    fn document(
        &self,
        arena: &LocalArena,
        uri: &Uri,
        document: Option<&Document>,
    ) -> Result<Vec<Diagnostic>, RequestError> {
        let path = self.path(uri);
        if path
            .as_ref()
            .is_some_and(|path| !self.patterns.includes(path) || self.patterns.excludes(path))
        {
            return Ok(Vec::new());
        }

        let source = match document {
            Some(document) => document.text.as_str(),
            None => &fs::read_to_string(
                file_path(uri)
                    .ok_or_else(|| RequestError::DocumentNotOpen(uri.as_str().to_owned()))?,
            )
            .map_err(|source| RequestError::ReadDocument {
                document: uri.as_str().to_owned(),
                source,
            })?,
        };

        let name = path
            .as_ref()
            .map_or_else(|| uri.as_str().into(), |path| path.to_string_lossy());
        Ok(lint(arena, Arc::clone(&self.registry), &name, source))
    }
}

impl Server {
    pub(super) fn document_diagnostics(&self, uri: &Uri) -> Result<Vec<Diagnostic>, RequestError> {
        self.diagnostics
            .document(&LocalArena::new(), uri, self.documents.get(uri))
    }

    pub(super) fn workspace_diagnostics(
        &self,
        params: &WorkspaceDiagnosticParams,
    ) -> Result<WorkspaceDiagnosticReportResult, RequestError> {
        let mut uris = Vec::new();
        for root in &self.diagnostics.roots {
            let files = discover(&[], root, &self.diagnostics.patterns)
                .map_err(|error| RequestError::Workspace(error.to_string()))?;
            for file in files {
                uris.push(file_uri(&file.path)?);
            }
        }
        uris.extend(self.documents.keys().cloned());
        uris.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        uris.dedup();
        let mut items = uris
            .par_iter()
            .map_init(LocalArena::new, |arena, uri| {
                arena.reset();
                let document = self.documents.get(uri);
                let items = self.diagnostics.document(arena, uri, document)?;
                Ok(WorkspaceFullDocumentDiagnosticReport {
                    uri: uri.clone(),
                    version: document.map(|document| i64::from(document.version)),
                    full_document_diagnostic_report: FullDocumentDiagnosticReport {
                        result_id: None,
                        items,
                    },
                }
                .into())
            })
            .collect::<Result<Vec<_>, RequestError>>()?;
        for previous in &params.previous_result_ids {
            if uris
                .binary_search_by(|uri| uri.as_str().cmp(previous.uri.as_str()))
                .is_err()
            {
                items.push(
                    WorkspaceFullDocumentDiagnosticReport {
                        uri: previous.uri.clone(),
                        version: None,
                        full_document_diagnostic_report: FullDocumentDiagnosticReport::default(),
                    }
                    .into(),
                );
            }
        }
        Ok(WorkspaceDiagnosticReport { items }.into())
    }
}

fn lint(
    arena: &LocalArena,
    registry: Arc<RuleRegistry>,
    path: &str,
    source: &str,
) -> Vec<Diagnostic> {
    let index = LineIndex::new(source);
    let mut diagnostics = Vec::new();
    let mut report = |span: Span, code: &str, level: &Level<'static>, message: &str| {
        diagnostics.push(Diagnostic {
            range: index.range(span.start.offset as usize, span.end.offset as usize),
            severity: Some(if *level == Level::ERROR {
                DiagnosticSeverity::ERROR
            } else if *level == Level::WARNING {
                DiagnosticSeverity::WARNING
            } else if *level == Level::HELP {
                DiagnosticSeverity::HINT
            } else {
                DiagnosticSeverity::INFORMATION
            }),
            code: Some(NumberOrString::String(code.to_owned())),
            source: Some("whim".to_owned()),
            message: message.to_owned(),
            ..Diagnostic::default()
        });
    };
    match parse(arena, source) {
        Ok(program) => {
            Linter::from_registry(arena, registry).lint_with_diagnostics(
                path,
                Path::new(path),
                program,
                Some(&mut report),
            );
        }
        Err(error) => report(error.span(), "syntax", &Level::ERROR, &error.to_string()),
    }
    diagnostics
}

pub(super) fn file_path(uri: &Uri) -> Option<PathBuf> {
    Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

fn file_uri(path: &Path) -> Result<Uri, RequestError> {
    Url::from_file_path(path)
        .ok()
        .and_then(|url| url.as_str().parse().ok())
        .ok_or_else(|| {
            RequestError::Workspace(format!(
                "could not form a file URI for `{}`",
                path.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use std::env::temp_dir;
    use std::fs;
    use std::path::PathBuf;
    use std::process::id;
    use std::time::Duration;

    use lsp_server::Connection;
    use lsp_server::Message;
    use lsp_server::Notification;
    use lsp_types::DiagnosticSeverity;
    use lsp_types::NumberOrString;
    use lsp_types::Position;
    use lsp_types::PublishDiagnosticsParams;
    use lsp_types::Uri;
    use lsp_types::WorkspaceDiagnosticReportResult;
    use lsp_types::WorkspaceDocumentDiagnosticReport;
    use serde_json::json;
    use whim_formatter::settings::FormatSettings;

    use super::Diagnostics;
    use super::file_uri;
    use crate::config::LintConfiguration;
    use crate::server::Document;
    use crate::server::Server;

    fn server(root: PathBuf) -> Server {
        let settings = LintConfiguration::default();
        Server::new(
            FormatSettings::default(),
            Diagnostics::new(
                settings.registry().unwrap(),
                settings.patterns().unwrap(),
                root,
            ),
        )
    }

    fn published(connection: &Connection) -> PublishDiagnosticsParams {
        let Message::Notification(notification) = connection
            .receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
        else {
            panic!("expected diagnostics");
        };
        assert_eq!(notification.method, "textDocument/publishDiagnostics");
        serde_json::from_value(notification.params).unwrap()
    }

    #[test]
    fn document_changes_publish_lints_syntax_errors_and_clear_stale_diagnostics() {
        let mut server = server(temp_dir());
        let (connection, client) = Connection::memory();
        let uri: Uri = "untitled:source.whim".parse().unwrap();
        server.notification(&connection, Notification::new("textDocument/didOpen".to_owned(), json!({
            "textDocument": { "uri": uri, "languageId": "whim", "version": 1, "text": "'🙂'; $password = 'secret';\r\n" }
        }))).unwrap();
        let report = published(&client);
        assert_eq!(report.version, Some(1));
        let issue = report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("no-literal-password".to_owned()))
            })
            .unwrap();
        assert_eq!(issue.range.start, Position::new(0, 18));
        assert_eq!(issue.range.end, Position::new(0, 26));
        assert_eq!(issue.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(
            server.document_diagnostics(&uri).unwrap(),
            report.diagnostics
        );

        for (version, text, expected) in [(2, "$x = ;", 1), (3, "$x = 1;", 0)] {
            server.notification(&connection, Notification::new("textDocument/didChange".to_owned(), json!({
                "textDocument": { "uri": uri, "version": version }, "contentChanges": [{ "text": text }]
            }))).unwrap();
            let report = published(&client);
            assert_eq!(report.version, Some(version));
            assert_eq!(report.diagnostics.len(), expected);
            if expected != 0 {
                assert_eq!(
                    report.diagnostics[0].code,
                    Some(NumberOrString::String("syntax".to_owned()))
                );
            }
        }
        server
            .notification(
                &connection,
                Notification::new(
                    "textDocument/didClose".to_owned(),
                    json!({ "textDocument": { "uri": uri } }),
                ),
            )
            .unwrap();
        assert!(published(&client).diagnostics.is_empty());
    }

    #[test]
    fn workspace_reports_disk_files_unsaved_text_and_deleted_files() {
        let root = temp_dir().join(format!("whim-lsp-lint-{}", id()));
        fs::create_dir_all(root.join("vendor")).unwrap();
        fs::write(root.join("closed.whim"), "// TODO: track this\n").unwrap();
        fs::write(root.join("open.whim"), "$password = 'secret';").unwrap();
        fs::write(root.join("vendor/skip.whim"), "$password = 'secret';").unwrap();
        let mut server = server(root.clone());
        let open = file_uri(&root.join("open.whim")).unwrap();
        let removed = file_uri(&root.join("removed.whim")).unwrap();
        server.documents.insert(
            open.clone(),
            Document {
                text: "$x = 1;".to_owned(),
                version: 7,
            },
        );
        let params = serde_json::from_value(
            json!({ "previousResultIds": [{"uri": removed, "value": "old"}] }),
        )
        .unwrap();
        let WorkspaceDiagnosticReportResult::Report(report) =
            server.workspace_diagnostics(&params).unwrap()
        else {
            panic!("expected a full report");
        };
        assert_eq!(report.items.len(), 3);
        for item in report.items {
            let WorkspaceDocumentDiagnosticReport::Full(item) = item else {
                panic!("expected a full document");
            };
            if item.uri == open {
                assert_eq!(item.version, Some(7));
                assert!(item.full_document_diagnostic_report.items.is_empty());
            } else if item.uri == removed {
                assert!(item.full_document_diagnostic_report.items.is_empty());
            } else {
                assert!(item.uri.as_str().ends_with("closed.whim"));
                assert_eq!(item.full_document_diagnostic_report.items.len(), 1);
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
