//! Rendering source-anchored diagnostics.

use annotate_snippets::AnnotationKind;
use annotate_snippets::Group;
use annotate_snippets::Level;
use annotate_snippets::Renderer;
use annotate_snippets::Snippet;

use whim_span::HasSpan;
use whim_span::Span;

use crate::error::ParseError;

/// Renders `entries` (each a span paired with its message) against `source`,
/// labelling the report's origin `origin`. Each entry becomes an "error:"
/// group showing the source line and a caret at the span.
#[must_use]
pub fn render(source: &str, origin: &str, entries: &[(Span, &str)]) -> String {
    render_with_color(source, origin, entries, false)
}

/// Renders `entries` like [`render`], using ANSI terminal styling when
/// `color` is true.
#[must_use]
pub fn render_with_color(
    source: &str,
    origin: &str,
    entries: &[(Span, &str)],
    color: bool,
) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let groups: Vec<Group<'_>> = entries
        .iter()
        .map(|(span, message)| {
            let (start, end) = clamp_span(source, *span);

            Group::with_title(Level::ERROR.primary_title(*message)).element(
                Snippet::source(source)
                    .path(origin)
                    .fold(true)
                    .annotation(AnnotationKind::Primary.span(start..end)),
            )
        })
        .collect();

    if color {
        Renderer::styled().render(&groups)
    } else {
        Renderer::plain().render(&groups)
    }
}

/// Clamps `span` to a byte range that lies within `source` and falls on
/// `char` boundaries, widening a zero-width span to a single caret column where
/// possible so the annotation is always visible.
fn clamp_span(source: &str, span: Span) -> (usize, usize) {
    let length = source.len();
    let mut start = (span.start.offset as usize).min(length);
    let mut end = (span.end.offset as usize).min(length).max(start);

    if start == end && end < length {
        end += 1;
    }

    while start > 0 && !source.is_char_boundary(start) {
        start -= 1;
    }
    while end < length && !source.is_char_boundary(end) {
        end += 1;
    }

    (start, end)
}

impl ParseError {
    /// Renders the error against `source`, labelling the report `origin`.
    #[must_use]
    pub fn render(&self, source: &str, origin: &str) -> String {
        self.render_with_color(source, origin, false)
    }

    /// Renders the parse error with optional ANSI terminal styling.
    #[must_use]
    pub fn render_with_color(&self, source: &str, origin: &str, color: bool) -> String {
        let message = self.to_string();
        render_with_color(
            source,
            origin,
            &[(HasSpan::span(self), message.as_str())],
            color,
        )
    }
}

#[cfg(test)]
mod tests {
    use whim_span::Position;
    use whim_span::Span;

    use crate::diagnostic::render_with_color;

    fn strip_sgr(text: &str) -> String {
        let mut output = String::new();
        let mut bytes = text.bytes();
        while let Some(byte) = bytes.next() {
            if byte == 0x1b && bytes.next() == Some(b'[') {
                for byte in bytes.by_ref() {
                    if byte == b'm' {
                        break;
                    }
                }
            } else {
                output.push(char::from(byte));
            }
        }
        output
    }

    #[test]
    fn styled_and_plain_diagnostics_have_identical_text() {
        let source = "$value = 1;\n";
        let span = Span::new(Position::new(9), Position::new(10));
        let entries = [(span, "example failure")];
        let plain = render_with_color(source, "example.whim", &entries, false);
        let styled = render_with_color(source, "example.whim", &entries, true);
        assert!(styled.contains("\u{1b}["));
        assert_eq!(strip_sgr(&styled), plain);
    }

    #[test]
    fn an_end_of_file_span_points_after_the_last_byte() {
        let source = "$a = 1";
        let end = Position::new(source.len() as u32);
        let rendered = render_with_color(
            source,
            "example.whim",
            &[(Span::new(end, end), "expected `;`")],
            false,
        );
        assert!(rendered.contains("example.whim:1:7"), "{rendered}");
    }
}
