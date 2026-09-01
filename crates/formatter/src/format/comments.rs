//! Comment attachment and rendering: taking pending comments and emitting
//! them as leading, trailing, or interior documents.

use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::arena::Vec;
use whim_syn::cst::trivia::Trivia;
use whim_syn::cst::trivia::TriviaKind;

use crate::document::Document;
use crate::format::FormatterState;

impl<'arena, A> FormatterState<'arena, A>
where
    A: Arena,
{
    /// Appends the comments written inside a construct that no element of it
    /// claimed, and reports whether the construct must now break.
    pub(in crate::format) fn take_interior_comments(
        &mut self,
        close_offset: u32,
        parts: &mut Vec<'arena, Document<'arena, A>, A>,
    ) -> bool {
        let mut force = false;

        while let Some(comment) = self.take_comment_before(close_offset) {
            let leading_space = !parts.is_empty();
            let document = self.format_comment(&comment);

            if matches!(comment.kind, TriviaKind::SingleLineComment) {
                force = true;

                let mut suffix = self.vec();
                if leading_space {
                    suffix.push(self.space());
                }

                suffix.push(document);
                parts.push(Document::LineSuffix(suffix));
                parts.push(Document::BreakParent);
            } else {
                if leading_space {
                    parts.push(self.space());
                }

                parts.push(document);
            }
        }

        force
    }

    /// The number of line breaks in `source_text[from..to]`.
    pub(in crate::format) fn count_newlines(&self, from: u32, to: u32) -> usize {
        let start = from as usize;
        let end = (to as usize).min(self.source_text.len());
        if start >= end {
            return 0;
        }

        let bytes = &self.source_text.as_bytes()[start..end];
        let mut breaks = 0;
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\r' => {
                    breaks += 1;
                    index += usize::from(bytes.get(index + 1) == Some(&b'\n')) + 1;
                }
                b'\n' => {
                    breaks += 1;
                    index += 1;
                }
                _ => index += 1,
            }
        }
        breaks
    }

    fn column_of(&self, offset: u32) -> usize {
        let offset = (offset as usize).min(self.source_text.len());
        let head = &self.source_text[..offset];

        match head.rfind(['\n', '\r']) {
            Some(index) => head[index + 1..].chars().count(),
            None => head.chars().count(),
        }
    }

    /// Consumes and returns the next pending comment if it begins before `offset`.
    pub(in crate::format) fn take_comment_before(&mut self, offset: u32) -> Option<Trivia<'arena>> {
        let comment = self.comments.get(self.comment_cursor)?;
        if comment.span.start.offset < offset {
            self.comment_cursor += 1;

            return Some(*comment);
        }

        None
    }

    /// If the next pending comment sits on the same source line as `after`,
    /// consumes it and returns a line-suffix document plus the comment's end
    /// offset. Handles the `stmt; // trailing` case.
    pub(in crate::format) fn take_trailing_comment(
        &mut self,
        after: u32,
    ) -> Option<(Document<'arena, A>, u32)> {
        let comment = *self.comments.get(self.comment_cursor)?;
        let start = comment.span.start.offset;
        if start < after || self.count_newlines(after, start) != 0 {
            return None;
        }

        self.comment_cursor += 1;
        let comment_document = self.format_comment(&comment);
        let mut suffix = self.vec();
        suffix.push(self.space());
        suffix.push(comment_document);

        Some((Document::LineSuffix(suffix), comment.span.end.offset))
    }

    /// Renders a single comment, re-indenting block/doc comments.
    pub(in crate::format) fn format_comment(
        &self,
        comment: &Trivia<'arena>,
    ) -> Document<'arena, A> {
        match comment.kind {
            TriviaKind::SingleLineComment => self.text(comment.value.trim_ascii_end()),
            TriviaKind::MultiLineComment | TriviaKind::DocBlockComment => {
                self.format_block_comment(comment.value, self.column_of(comment.span.start.offset))
            }
            _ => self.empty(),
        }
    }

    fn format_block_comment(&self, value: &'arena str, base_column: usize) -> Document<'arena, A> {
        if !value.contains('\n') && !value.contains('\r') {
            return self.text(value.trim_ascii_end());
        }

        let mut parts = self.vec();
        for (index, raw_line) in split_lines(self.arena, value).into_iter().enumerate() {
            let line = strip_columns(raw_line.trim_ascii_end(), base_column);
            if index == 0 {
                parts.push(self.text(raw_line.trim_ascii_end()));
                continue;
            }

            parts.push(self.hard_line());
            let trimmed = line.trim_ascii_start();
            if trimmed.starts_with('*') {
                let aligned = self.arena.alloc_fmt(format_args!(" {trimmed}"));
                parts.push(self.text(aligned));
            } else {
                parts.push(self.text(line));
            }
        }

        Document::Array(parts)
    }

    /// The comments that lead the node at `span`: every pending comment that
    /// ends at or before the node begins. A comment on its own line (or any
    /// line comment) breaks the enclosing group and sits above the node; an
    /// inline block comment stays on the same line ahead of it.
    pub(in crate::format) fn print_leading_comments(
        &mut self,
        span: Span,
    ) -> Option<Document<'arena, A>> {
        let start = span.start.offset;
        let mut parts = self.vec();

        while let Some(comment) = self.comments.get(self.comment_cursor).copied() {
            if comment.span.end.offset > start {
                break;
            }
            self.comment_cursor += 1;

            let document = self.format_comment(&comment);
            parts.push(document);

            let newlines = self.count_newlines(comment.span.end.offset, start);
            if matches!(comment.kind, TriviaKind::SingleLineComment) || newlines > 0 {
                parts.push(Document::BreakParent);
                parts.push(self.hard_line());
                if newlines >= 2 {
                    parts.push(self.hard_line());
                }
            } else {
                parts.push(self.space());
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(Document::Array(parts))
        }
    }

    /// The comments that trail the node at `span`: comments on the same line,
    /// just after it. A line comment becomes a line suffix and breaks the
    /// enclosing group; an inline block comment stays beside the node.
    pub(in crate::format) fn print_trailing_comments(
        &mut self,
        span: Span,
    ) -> Option<Document<'arena, A>> {
        let end = span.end.offset;
        let mut parts = self.vec();

        while let Some(comment) = self.comments.get(self.comment_cursor).copied() {
            let comment_start = comment.span.start.offset;
            if comment_start < end || self.count_newlines(end, comment_start) != 0 {
                break;
            }

            let is_line = matches!(comment.kind, TriviaKind::SingleLineComment);
            let gap = &self.source_text[end as usize..comment_start as usize];
            let separators = gap.bytes().filter(|byte| *byte == b',').count();
            let only_gap = gap
                .bytes()
                .all(|byte| byte.is_ascii_whitespace() || byte == b',');
            if !only_gap || separators > 1 || (separators == 1 && !is_line) {
                break;
            }

            self.comment_cursor += 1;
            let document = self.format_comment(&comment);
            if is_line {
                let mut suffix = self.vec();
                suffix.push(self.space());
                suffix.push(document);
                parts.push(Document::LineSuffix(suffix));
                parts.push(Document::BreakParent);
            } else {
                parts.push(self.space());
                parts.push(document);
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(Document::Array(parts))
        }
    }

    pub(in crate::format) fn with_comments(
        &self,
        leading: Option<Document<'arena, A>>,
        document: Document<'arena, A>,
        trailing: Option<Document<'arena, A>>,
    ) -> Document<'arena, A> {
        match (leading, trailing) {
            (None, None) => document,
            (Some(leading), None) => self.concat([leading, document]),
            (None, Some(trailing)) => self.concat([document, trailing]),
            (Some(leading), Some(trailing)) => self.concat([leading, document, trailing]),
        }
    }
}

fn strip_columns(line: &str, columns: usize) -> &str {
    let bytes = line.as_bytes();
    let mut index = 0;

    while index < columns && index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b'\t')
    {
        index += 1;
    }

    &line[index..]
}

fn split_lines<'arena, A>(arena: &'arena A, value: &'arena str) -> Vec<'arena, &'arena str, A>
where
    A: Arena,
{
    let mut lines = Vec::new_in(arena);
    let bytes = value.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                lines.push(&value[start..index]);
                index += usize::from(bytes.get(index + 1) == Some(&b'\n')) + 1;
                start = index;
            }
            b'\n' => {
                lines.push(&value[start..index]);
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }

    lines.push(&value[start..]);
    lines
}
