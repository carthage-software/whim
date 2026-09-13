use std::env::temp_dir;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::id;
use std::time::Duration;

use lsp_server::Connection;
use lsp_server::Message;
use lsp_server::Notification;
use lsp_server::Request;
use lsp_types::DiagnosticSeverity;
use lsp_types::SemanticTokensResult;
use lsp_types::TextEdit;
use lsp_types::Uri;
use lsp_types::WorkspaceDiagnosticReportResult;
use serde_json::Value;
use serde_json::json;

use super::Document;
use super::Server;
use super::diagnostics::file_uri;
use super::initialize;

const SOURCE: &str = "// TODO: track this\nfunction example(): int {\nreturn 1;\n}\n";

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root = temp_dir().join(format!("whim-lsp-{name}-{}", id()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, name: &str, source: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn uri(&self, name: &str) -> Uri {
        file_uri(&self.0.join(name)).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn start(params: Value, configuration: Option<&Path>) -> (Server, Connection, Connection) {
    let (connection, client) = Connection::memory();
    client
        .sender
        .send(Message::Request(Request::new(
            1.into(),
            "initialize".to_owned(),
            params,
        )))
        .unwrap();
    client
        .sender
        .send(Message::Notification(Notification::new(
            "initialized".to_owned(),
            json!({}),
        )))
        .unwrap();
    let server = initialize(&connection, configuration).unwrap();
    let Message::Response(response) = receive(&client) else {
        panic!("expected initialize response");
    };
    assert!(response.response_result.is_ok());
    (server, connection, client)
}

fn receive(client: &Connection) -> Message {
    client
        .receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
}

fn format(server: &Server, uri: &Uri) -> Option<Vec<TextEdit>> {
    server
        .format(
            &serde_json::from_value(json!({
                "textDocument": {"uri": uri},
                "options": {"tabSize": 8, "insertSpaces": true},
            }))
            .unwrap(),
        )
        .unwrap()
}

fn open(server: &mut Server, uri: &Uri) {
    server.documents.insert(
        uri.clone(),
        Document {
            text: SOURCE.to_owned(),
            version: 1,
        },
    );
}

#[test]
fn missing_manifest_reports_an_error_and_keeps_open_file_features() {
    let fixture = Fixture::new("missing-manifest");
    fixture.write("closed.whim", "$x = ;");
    let (mut server, connection, client) = start(json!({"rootUri": fixture.uri("")}), None);
    let Message::Notification(error) = receive(&client) else {
        panic!("expected a missing config error");
    };
    assert_eq!(error.method, "window/showMessage");
    assert_eq!(error.params["type"], 1);
    assert!(
        error.params["message"]
            .as_str()
            .unwrap()
            .contains("whim.toml")
    );
    assert!(
        error.params["message"]
            .as_str()
            .unwrap()
            .contains("whim init")
    );

    let uri = fixture.uri("open.whim");
    open(&mut server, &uri);
    server.publish(&connection, &uri).unwrap();
    let Message::Notification(diagnostics) = receive(&client) else {
        panic!("expected open file diagnostics");
    };
    assert_eq!(diagnostics.method, "textDocument/publishDiagnostics");
    assert_eq!(diagnostics.params["diagnostics"][0]["code"], "tagged-todo");
    assert!(
        format(&server, &uri).unwrap()[0]
            .new_text
            .contains("\n  return 1;\n")
    );
    let Some(SemanticTokensResult::Tokens(tokens)) = server
        .semantic_tokens(&serde_json::from_value(json!({"textDocument": {"uri": uri}})).unwrap())
        .unwrap()
    else {
        panic!("expected semantic tokens");
    };
    assert!(!tokens.data.is_empty());
    assert!(
        server
            .document_diagnostics(&fixture.uri("closed.whim"))
            .unwrap()
            .is_empty()
    );
    let WorkspaceDiagnosticReportResult::Report(report) = server
        .workspace_diagnostics(&serde_json::from_value(json!({"previousResultIds": []})).unwrap())
        .unwrap()
    else {
        panic!("expected a workspace report");
    };
    assert_eq!(report.items.len(), 1);
}

#[test]
fn workspace_configs_control_open_files_and_disk_files() {
    let fixture = Fixture::new("workspace-configs");
    fixture.write(
        "first/whim.toml",
        r#"manifest-version = 1
[format]
include = ["src"]
exclude = ["src/generated", "src/lint-only.whim"]
tab_width = 4
[lint]
include = ["src"]
exclude = ["src/generated", "src/format-only.whim"]
[lint.rules.tagged-todo]
level = "note"
exclude = ["src/rule-skipped.whim"]
"#,
    );
    fixture.write("second/whim.toml", "manifest-version = 1\n[format]\ntab_width = 6\n[lint.rules.tagged-todo]\nenabled = false\n");
    fixture.write("first/src/closed.whim", SOURCE);
    fixture.write("first/src/generated/closed.whim", "$x = ;");
    fixture.write("first/outside.whim", "$x = ;");
    let (mut server, connection, client) = start(
        json!({
            "workspaceFolders": [
                {"uri": fixture.uri("first/src"), "name": "first"},
                {"uri": fixture.uri("second"), "name": "second"},
            ],
        }),
        None,
    );
    for (name, diagnostics, indent) in [
        ("first/src/open.whim", 1, Some(4)),
        ("first/src/generated/open.whim", 0, None),
        ("first/outside.whim", 0, None),
        ("first/vendor/skip.whim", 0, None),
        ("first/src/rule-skipped.whim", 0, Some(4)),
        ("first/src/format-only.whim", 0, Some(4)),
        ("first/src/lint-only.whim", 1, None),
        ("second/open.whim", 0, Some(6)),
    ] {
        let uri = fixture.uri(name);
        open(&mut server, &uri);
        let issues = server.document_diagnostics(&uri).unwrap();
        assert_eq!(issues.len(), diagnostics, "{name}");
        if let Some(issue) = issues.first() {
            assert_eq!(issue.severity, Some(DiagnosticSeverity::INFORMATION));
        }
        server.publish(&connection, &uri).unwrap();
        let Message::Notification(published) = receive(&client) else {
            panic!("expected diagnostics");
        };
        assert_eq!(
            published.params["diagnostics"].as_array().unwrap().len(),
            diagnostics
        );
        match indent {
            Some(indent) => assert!(
                format(&server, &uri).unwrap()[0]
                    .new_text
                    .contains(&format!("\n{}return 1;\n", " ".repeat(indent)))
            ),
            None => assert!(format(&server, &uri).is_none(), "{name}"),
        }
    }
    let closed = server
        .document_diagnostics(&fixture.uri("first/src/closed.whim"))
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].severity, Some(DiagnosticSeverity::INFORMATION));
    let WorkspaceDiagnosticReportResult::Report(report) = server
        .workspace_diagnostics(&serde_json::from_value(json!({"previousResultIds": []})).unwrap())
        .unwrap()
    else {
        panic!("expected a workspace report");
    };
    assert_eq!(report.items.len(), server.documents.len() + 1);
}

#[test]
fn workspace_config_applies_disallowed_symbols_to_lsp_diagnostics() {
    let fixture = Fixture::new("disallowed-symbols");
    fixture.write(
        "whim.toml",
        "manifest-version = 1\n[lint.rules.disallowed-symbols]\nsymbols = [{ name = 'Legacy\\run', level = 'error' }]\n",
    );
    fixture.write("source.whim", "Legacy\\run();\n");
    let (server, _connection, _client) = start(json!({"rootUri": fixture.uri("")}), None);

    let diagnostics = server
        .document_diagnostics(&fixture.uri("source.whim"))
        .unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code,
        Some(lsp_types::NumberOrString::String(
            "disallowed-symbols".to_owned()
        ))
    );
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
}

#[test]
fn invalid_configs_report_errors_without_using_default_filters() {
    let fixture = Fixture::new("invalid-config");
    fixture.write("whim.toml", "[format]\nexclude = [");
    fixture.write("closed.whim", "$x = ;");
    let (mut server, _connection, client) = start(json!({"rootUri": fixture.uri("")}), None);
    let Message::Notification(error) = receive(&client) else {
        panic!("expected a config error");
    };
    assert_eq!(error.method, "window/showMessage");
    assert_eq!(error.params["type"], 1);
    assert!(
        error.params["message"]
            .as_str()
            .unwrap()
            .contains("whim.toml")
    );
    let uri = fixture.uri("open.whim");
    open(&mut server, &uri);
    assert!(format(&server, &uri).is_none());
    assert!(server.document_diagnostics(&uri).unwrap().is_empty());
    let WorkspaceDiagnosticReportResult::Report(report) = server
        .workspace_diagnostics(&serde_json::from_value(json!({"previousResultIds": []})).unwrap())
        .unwrap()
    else {
        panic!("expected a workspace report");
    };
    assert_eq!(report.items.len(), 1);
    assert!(
        server
            .semantic_tokens(
                &serde_json::from_value(json!({"textDocument": {"uri": uri}})).unwrap()
            )
            .unwrap()
            .is_some()
    );
}

#[test]
fn lint_attributes_apply_to_lsp_diagnostics_and_report_invalid_arguments() {
    let fixture = Fixture::new("lint-attributes");
    fixture.write(
        "whim.toml",
        "manifest-version = 1\n[lint.rules.no-debug-symbols]\nenabled = false\n",
    );
    let source = r"
#[Whim\Lint\Allow(rule: 'no-debug-symbols', reason: 'Test fixture')]
function allowed(): void { debug!(1); }
#[Whim\Lint\Warn(reason: 'Test fixture', rule: 'no-debug-symbols')]
function warned(): void { debug!(2); }
#[Whim\Lint\Deny('no-debug-symbols', reason: 'Test fixture')]
function denied(): void { debug!(3); }
#[Whim\Lint\Allow(reason: 'Test fixture', rule: UNKNOWN)]
function broken(): void {}
";
    fixture.write("source.whim", source);
    let (server, _connection, _client) = start(json!({"rootUri": fixture.uri("")}), None);
    let diagnostics = server
        .document_diagnostics(&fixture.uri("source.whim"))
        .unwrap();
    assert_eq!(diagnostics.len(), 3);
    assert_eq!(
        diagnostics[0].code,
        Some(lsp_types::NumberOrString::String("lint-attribute".into()))
    );
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(diagnostics[0].range.start.line, 7);
    assert_eq!(
        diagnostics[0].range.end.character - diagnostics[0].range.start.character,
        7
    );
    assert_eq!(diagnostics[1].severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(diagnostics[2].severity, Some(DiagnosticSeverity::ERROR));
}
