use std::slice;

use annotate_snippets::Group;
use annotate_snippets::Level;
use annotate_snippets::Renderer;
use serde::Serialize;
use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::error::ParseError;

use crate::pipeline::files::Target;
use crate::pipeline::timed;
use crate::service::FileError;

#[derive(Serialize)]
pub(super) struct Diagnostic<'path> {
    path: &'path str,
    code: String,
    #[serde(with = "whim_linter::settings::level")]
    level: Level<'static>,
    message: String,
    span: Option<Span>,
    rendered: Option<String>,
}

impl<'path> Diagnostic<'path> {
    pub(super) fn new(
        path: &'path str,
        span: Option<Span>,
        code: &str,
        level: &Level<'static>,
        message: &str,
    ) -> Self {
        Self {
            path,
            code: code.to_owned(),
            level: level.clone(),
            message: message.to_owned(),
            span,
            rendered: None,
        }
    }
}

pub(super) fn render(diagnostics: Vec<Diagnostic<'_>>, groups: &[Group<'_>]) -> String {
    let renderer = Renderer::plain();
    let mut output = Vec::new();
    for (mut diagnostic, group) in diagnostics.into_iter().zip(groups) {
        if !output.is_empty() {
            output.push(b',');
        }
        diagnostic.rendered = Some(renderer.render(slice::from_ref(group)));
        serde_json::to_writer(&mut output, &diagnostic)
            .expect("lint diagnostics serialize as JSON");
    }
    String::from_utf8(output).expect("JSON output is UTF-8")
}

pub(super) fn syntax(error: ParseError, source: &str, target: &Target) -> FileError {
    FileError::Syntax(timed("render", || {
        let name = target.spelling.to_string_lossy();
        let mut diagnostic = Diagnostic::new(
            &name,
            Some(error.span()),
            "syntax",
            &Level::ERROR,
            &error.to_string(),
        );
        diagnostic.rendered = Some(error.render_with_color(source, &name, false));
        serde_json::to_string(&diagnostic).expect("syntax diagnostics serialize as JSON")
    }))
}

pub(super) fn file_error(target: &Target, code: &str, message: &str) -> String {
    let name = target.spelling.to_string_lossy();
    serde_json::to_string(&Diagnostic::new(&name, None, code, &Level::ERROR, message))
        .expect("file diagnostics serialize as JSON")
}
