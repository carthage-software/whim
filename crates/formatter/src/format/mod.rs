//! The [`Format`] trait, the [`FormatterState`], and the shared building blocks.

use std::ops::Range;

/// Wraps a node's formatting so it emits its own leading and trailing comments.
macro_rules! wrap {
    ($f:expr, $self:expr, $body:block) => {{
        let span = ::whim_span::HasSpan::span($self);
        let leading = $f.print_leading_comments(span);
        let document = $body;
        let trailing = $f.print_trailing_comments(span);
        $f.with_comments(leading, document, trailing)
    }};
}

mod call;
mod comments;
mod construct;
mod declaration;
mod expression;
mod spine;
mod statement;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::arena::Vec;
use whim_syn::cst::Program;
use whim_syn::cst::atom::Modifier;
use whim_syn::cst::call::Argument;
use whim_syn::cst::call::ArgumentList;
use whim_syn::cst::construct::ConstructArgument;
use whim_syn::cst::declaration::AttributeList;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::sequence::TokenSeparatedSequence;
use whim_syn::cst::trivia::Trivia;
use whim_syn::cst::trivia::TriviaKind;
use whim_syn::cst::r#type::TupleType;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeArgumentList;
use whim_syn::cst::r#type::TypeParameterList;

use crate::document::BreakMode;
use crate::document::Document;
use crate::document::Group;
use crate::document::IfBreak;
use crate::document::Line;

/// Turns a CST node into a layout [`Document`].
trait Format<'arena, A>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A>;

    #[inline]
    fn should_skip(&self) -> bool {
        false
    }

    /// Whether a multiline rendering of this node must be followed by an
    /// empty line when another sibling follows it.
    #[inline]
    fn blank_line_after_if_multiline(&self) -> bool {
        false
    }

    /// Whether this node and the following sibling must always have an empty
    /// line between them.
    #[inline]
    fn blank_line_before(&self, _next: &Self) -> bool {
        false
    }
}

impl<'arena, A, T> Format<'arena, A> for &T
where
    A: Arena,
    T: Format<'arena, A>,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        (**self).format(f)
    }
}

/// The mutable state threaded through a single formatting pass.
pub(super) struct FormatterState<'arena, A>
where
    A: Arena,
{
    arena: &'arena A,
    source_text: &'arena str,
    comments: &'arena [Trivia<'arena>],
    comment_cursor: usize,
    shebang: Option<&'arena str>,
}

impl<'arena, A> FormatterState<'arena, A>
where
    A: Arena,
{
    /// Creates a state for formatting `program`.
    #[must_use]
    pub(super) fn new(
        arena: &'arena A,
        program: &'arena Program<'arena>,
        source_text: &'arena str,
    ) -> Self {
        let mut comments = Vec::new_in(arena);
        let mut shebang = None;

        for trivia in program.trivia {
            if trivia.kind == TriviaKind::Shebang {
                shebang = Some(trivia.value.trim_ascii_end());
            } else if trivia.kind.is_comment() {
                comments.push(*trivia);
            }
        }

        Self {
            arena,
            source_text,
            comments: comments.leak(),
            comment_cursor: 0,
            shebang,
        }
    }

    #[must_use]
    pub(super) fn format_program(
        &mut self,
        program: &'arena Program<'arena>,
    ) -> Document<'arena, A> {
        program.format(self)
    }

    #[inline]
    const fn vec(&self) -> Vec<'arena, Document<'arena, A>, A> {
        Vec::new_in(self.arena)
    }

    #[inline]
    const fn text(&self, text: &'arena str) -> Document<'arena, A> {
        Document::String(text)
    }

    #[inline]
    const fn empty(&self) -> Document<'arena, A> {
        Document::empty()
    }

    #[inline]
    const fn space(&self) -> Document<'arena, A> {
        Document::space()
    }

    #[inline]
    fn line(&self) -> Document<'arena, A> {
        Document::Line(Line::default())
    }

    #[inline]
    fn soft_line(&self) -> Document<'arena, A> {
        Document::Line(Line::soft())
    }

    #[inline]
    fn hard_line(&self) -> Document<'arena, A> {
        Document::Line(Line::hard())
    }

    #[inline]
    fn concat(
        &self,
        documents: impl IntoIterator<Item = Document<'arena, A>>,
    ) -> Document<'arena, A> {
        let mut parts = self.vec();
        parts.extend(documents);
        Document::Array(parts)
    }

    #[inline]
    const fn indent(&self, contents: Vec<'arena, Document<'arena, A>, A>) -> Document<'arena, A> {
        Document::Indent(contents)
    }

    #[inline]
    const fn indent_if_break(
        &self,
        contents: Vec<'arena, Document<'arena, A>, A>,
    ) -> Document<'arena, A> {
        Document::IndentIfBreak(contents)
    }

    #[inline]
    fn blank_line_after_if_multiline(&self, document: Document<'arena, A>) -> Document<'arena, A> {
        Document::BlankLineAfterIfMultiline(self.arena.alloc(document))
    }

    #[inline]
    fn ifbreak(
        &self,
        break_contents: Document<'arena, A>,
        flat_content: Document<'arena, A>,
    ) -> Document<'arena, A> {
        Document::IfBreak(IfBreak {
            break_contents: self.arena.alloc(break_contents),
            flat_content: self.arena.alloc(flat_content),
        })
    }

    #[inline]
    fn never_break(&self, document: Document<'arena, A>) -> Document<'arena, A> {
        let mut contents = self.vec();
        contents.push(document);
        Document::Group(Group::new(contents).with_break_mode(BreakMode::Never))
    }

    fn delimited<T>(
        &mut self,
        open: &'arena str,
        nodes: &[T],
        close: &'arena str,
        close_offset: u32,
        force: bool,
    ) -> Document<'arena, A>
    where
        T: Format<'arena, A>,
    {
        self.delimited_with_break_mode(
            open,
            nodes,
            close,
            close_offset,
            if force {
                BreakMode::Force
            } else {
                BreakMode::Auto
            },
        )
    }

    fn delimited_with_break_mode<T>(
        &mut self,
        open: &'arena str,
        nodes: &[T],
        close: &'arena str,
        close_offset: u32,
        mut break_mode: BreakMode,
    ) -> Document<'arena, A>
    where
        T: Format<'arena, A>,
    {
        let mut inner = self.vec();
        for (index, node) in nodes.iter().enumerate() {
            if index != 0 {
                inner.push(self.text(","));
                inner.push(self.line());
            }

            let document = node.format(self);
            inner.push(document);
        }

        if self.take_interior_comments(close_offset, &mut inner) {
            break_mode = BreakMode::Force;
        }

        if inner.is_empty() {
            if matches!(break_mode, BreakMode::Auto | BreakMode::Never) {
                return self.concat([self.text(open), self.text(close)]);
            }

            let mut contents = self.vec();
            contents.push(self.text(open));
            contents.push(self.soft_line());
            contents.push(self.text(close));
            return Document::Group(Group::new(contents).with_break_mode(break_mode));
        }

        let mut indented = self.vec();
        indented.push(self.soft_line());
        indented.push(Document::Array(inner));

        let trailing_comma = if nodes.is_empty() {
            self.empty()
        } else {
            self.ifbreak(self.text(","), self.empty())
        };

        let mut contents = self.vec();
        contents.push(self.text(open));
        contents.push(self.indent_if_break(indented));
        contents.push(trailing_comma);
        contents.push(self.soft_line());
        contents.push(self.text(close));

        Document::Group(Group::new(contents).with_break_mode(break_mode))
    }

    fn signature_parameters<T>(&mut self, nodes: &[T], close_offset: u32) -> Document<'arena, A>
    where
        T: Format<'arena, A>,
    {
        self.delimited_with_break_mode("(", nodes, ")", close_offset, BreakMode::Parent)
    }

    fn parenthesized(&self, document: Document<'arena, A>) -> Document<'arena, A> {
        let mut inner = self.vec();
        inner.push(self.soft_line());
        inner.push(document);

        let mut contents = self.vec();
        contents.push(self.text("("));
        contents.push(self.indent_if_break(inner));
        contents.push(self.soft_line());
        contents.push(self.text(")"));
        Document::Group(Group::new(contents))
    }

    /// Joins `nodes` inline with `", "`, never breaking. Used for clauses where
    /// a trailing comma or line break would be invalid (`extends A, B`, the
    /// three clauses of a `for`, `match` arm conditions).
    fn inline_sequence<T>(&mut self, nodes: &[T]) -> Document<'arena, A>
    where
        T: Format<'arena, A>,
    {
        let mut parts = self.vec();
        for (index, node) in nodes.iter().enumerate() {
            if index != 0 {
                parts.push(self.text(", "));
            }

            let document = node.format(self);
            parts.push(document);
        }

        Document::Array(parts)
    }

    fn format_construct(
        &mut self,
        name: &'arena str,
        arguments: &[&'arena Expression<'arena>],
        close_offset: u32,
    ) -> Document<'arena, A> {
        let arguments = self.delimited_with_break_mode(
            "(",
            arguments,
            ")",
            close_offset,
            BreakMode::Independent,
        );
        self.concat([self.text(name), self.text("!"), arguments])
    }

    fn format_variadic_construct(
        &mut self,
        name: &'arena str,
        arguments: &TokenSeparatedSequence<'arena, ConstructArgument<'arena>>,
        close_offset: u32,
    ) -> Document<'arena, A> {
        let arguments = self.delimited_with_break_mode(
            "(",
            arguments.as_slice(),
            ")",
            close_offset,
            BreakMode::Independent,
        );
        self.concat([self.text(name), self.text("!"), arguments])
    }

    fn format_tuple<T>(
        &mut self,
        elements: &[T],
        close_offset: u32,
        single_element_comma: bool,
    ) -> Document<'arena, A>
    where
        T: Format<'arena, A>,
    {
        let mut inner = self.vec();
        for (index, element) in elements.iter().enumerate() {
            if index != 0 {
                inner.push(self.text(","));
                inner.push(self.line());
            }

            let element = element.format(self);
            inner.push(element);
        }

        self.format_tuple_documents(
            inner,
            close_offset,
            elements.len() == 1 && single_element_comma,
        )
    }

    fn format_tuple_type(&mut self, tuple: &TupleType<'arena>) -> Document<'arena, A> {
        let mut inner = self.vec();
        for (index, element) in tuple.elements.iter().enumerate() {
            if index != 0 {
                inner.push(self.text(","));
                inner.push(self.line());
            }

            inner.push(element.format(self));
        }

        if let Some(trailing) = &tuple.trailing_type {
            if !inner.is_empty() {
                inner.push(self.text(","));
                inner.push(self.line());
            }

            let trailing = match trailing.r#type {
                Some(r#type) => {
                    let r#type = r#type.format(self);
                    self.concat([self.text("..."), r#type])
                }
                None => self.text("..."),
            };
            inner.push(trailing);
        }

        self.format_tuple_documents(
            inner,
            tuple.right_parenthesis.start.offset,
            tuple.trailing_type.is_none() && tuple.elements.len() == 1,
        )
    }

    fn format_tuple_documents(
        &mut self,
        mut inner: Vec<'arena, Document<'arena, A>, A>,
        close_offset: u32,
        single_element_comma: bool,
    ) -> Document<'arena, A> {
        let force = self.take_interior_comments(close_offset, &mut inner);
        if inner.is_empty() {
            return self.concat([self.text("("), self.text(")")]);
        }

        let mut indented = self.vec();
        indented.push(self.soft_line());
        indented.push(Document::Array(inner));

        let trailing_comma = if single_element_comma {
            self.text(",")
        } else {
            self.ifbreak(self.text(","), self.empty())
        };

        let mut contents = self.vec();
        contents.push(self.text("("));
        contents.push(self.indent_if_break(indented));
        contents.push(trailing_comma);
        contents.push(self.soft_line());
        contents.push(self.text(")"));

        let group = Group::new(contents);
        let group = if force {
            group.with_break_mode(BreakMode::Force)
        } else {
            group
        };

        Document::Group(group)
    }

    /// Formats a union (`A|B`) or intersection (`A&B`) type from its flattened
    /// members. It stays on one line when it fits (`A|B|C`) and otherwise
    /// breaks with each member after the first on its own indented line
    /// (`| B`). Each member carries its own comments, so a comment between two
    /// members keeps its place and breaks the type.
    fn format_composite_type(
        &mut self,
        members: &[&'arena Type<'arena>],
        flat_separator: &'arena str,
        break_separator: &'arena str,
    ) -> Document<'arena, A> {
        let mut head = self.vec();
        let first = self.format_type_member(members[0]);
        head.push(first);

        let mut tail = self.vec();
        for member in &members[1..] {
            tail.push(self.soft_line());
            let separator = self.ifbreak(self.text(break_separator), self.text(flat_separator));
            tail.push(separator);
            let document = self.format_type_member(member);
            tail.push(document);
        }

        head.push(Document::Array(tail));

        Document::Group(Group::new(head))
    }

    /// Formats one member of a composite type, wrapped so it emits its own
    /// leading and trailing comments.
    fn format_type_member(&mut self, r#type: &'arena Type<'arena>) -> Document<'arena, A> {
        let span = r#type.span();
        let leading = self.print_leading_comments(span);
        let document = r#type.format(self);
        let trailing = self.print_trailing_comments(span);
        self.with_comments(leading, document, trailing)
    }

    fn format_type_argument_list(
        &mut self,
        list: &TypeArgumentList<'arena>,
    ) -> Document<'arena, A> {
        self.delimited(
            "<",
            list.arguments.as_slice(),
            ">",
            list.greater_than.start.offset,
            false,
        )
    }

    fn format_type_parameter_list(
        &mut self,
        list: &TypeParameterList<'arena>,
    ) -> Document<'arena, A> {
        self.delimited(
            "<",
            list.parameters.as_slice(),
            ">",
            list.greater_than.start.offset,
            false,
        )
    }

    #[inline]
    fn inline_token_sequence<T>(
        &mut self,
        sequence: &TokenSeparatedSequence<'arena, T>,
    ) -> Document<'arena, A>
    where
        T: Format<'arena, A>,
    {
        self.inline_sequence(sequence.as_slice())
    }

    #[inline]
    fn format_argument_list(
        &mut self,
        argument_list: &ArgumentList<'arena>,
    ) -> Document<'arena, A> {
        if argument_list.arguments.is_empty() {
            return self.concat([self.text("("), self.text(")")]);
        }

        if let [Argument::Positional(argument)] = argument_list.arguments.as_slice()
            && should_hug_expression(argument.value)
            && !self.comments.iter().any(|comment| {
                comment.span.start.offset >= argument_list.left_parenthesis.end.offset
                    && comment.span.end.offset <= argument_list.right_parenthesis.start.offset
            })
        {
            let argument = argument.value.format(self);
            return self.concat([self.text("("), argument, self.text(")")]);
        }

        let break_mode = if argument_list.arguments.len() >= 2
            && argument_list
                .arguments
                .iter()
                .all(|argument| matches!(argument, Argument::Named(_)))
        {
            BreakMode::Force
        } else {
            BreakMode::Independent
        };

        self.delimited_with_break_mode(
            "(",
            argument_list.arguments.as_slice(),
            ")",
            argument_list.right_parenthesis.start.offset,
            break_mode,
        )
    }

    fn attribute_lists_prefix(&mut self, lists: &[AttributeList<'arena>]) -> Document<'arena, A> {
        let mut parts = self.vec();
        for list in lists {
            let document = list.format(self);
            parts.push(document);
            parts.push(self.hard_line());
        }

        Document::Array(parts)
    }

    fn attribute_lists_inline(&mut self, lists: &[AttributeList<'arena>]) -> Document<'arena, A> {
        let mut parts = self.vec();
        for list in lists {
            let document = list.format(self);
            parts.push(document);
            parts.push(self.space());
        }

        Document::Array(parts)
    }

    /// Modifier keywords in source order, each followed by a space.
    fn modifiers_prefix(&self, modifiers: &[Modifier<'arena>]) -> Document<'arena, A> {
        let mut parts = self.vec();
        for modifier in modifiers {
            parts.push(self.text(modifier.keyword().value));
            parts.push(self.space());
        }

        Document::Array(parts)
    }

    /// Formats a run of sibling nodes, interleaving pending comments and
    /// separating everything with hard lines. A single blank line present in
    /// the source between two items is preserved; two or more collapse to one.
    /// Comments that begin before `close_offset` are flushed at the end so
    /// nothing inside an otherwise-empty delimiter is dropped.
    fn format_sequence<T>(
        &mut self,
        nodes: &[T],
        close_offset: u32,
    ) -> Vec<'arena, Document<'arena, A>, A>
    where
        T: Format<'arena, A> + HasSpan,
    {
        let mut out = self.vec();
        let mut previous_end: Option<u32> = None;
        let mut previous_separation = SequenceSeparation::Preserve;

        for (index, node) in nodes.iter().enumerate() {
            if node.should_skip() {
                continue;
            }

            let next = nodes[index + 1..]
                .iter()
                .find(|candidate| !candidate.should_skip());
            let separation = match next {
                Some(next) if node.blank_line_before(next) => SequenceSeparation::Always,
                Some(_) if node.blank_line_after_if_multiline() => SequenceSeparation::IfMultiline,
                _ => SequenceSeparation::Preserve,
            };

            let start = node.span().start.offset;
            while let Some(comment) = self.take_comment_before(start) {
                let document = self.format_comment(&comment);
                self.push_sequence_item(
                    &mut out,
                    &mut previous_end,
                    &mut previous_separation,
                    comment.span.start.offset..comment.span.end.offset,
                    document,
                    SequenceSeparation::Preserve,
                );
            }

            let mut document = node.format(self);
            let mut end = node.span().end.offset;
            if let Some((suffix, suffix_end)) = self.take_trailing_comment(end) {
                document = self.concat([document, suffix]);
                end = suffix_end;
            }

            self.push_sequence_item(
                &mut out,
                &mut previous_end,
                &mut previous_separation,
                start..end,
                document,
                separation,
            );
        }

        while let Some(comment) = self.take_comment_before(close_offset) {
            let document = self.format_comment(&comment);
            self.push_sequence_item(
                &mut out,
                &mut previous_end,
                &mut previous_separation,
                comment.span.start.offset..comment.span.end.offset,
                document,
                SequenceSeparation::Preserve,
            );
        }

        out
    }

    fn push_sequence_item(
        &self,
        out: &mut Vec<'arena, Document<'arena, A>, A>,
        previous_end: &mut Option<u32>,
        previous_separation: &mut SequenceSeparation,
        range: Range<u32>,
        document: Document<'arena, A>,
        separation: SequenceSeparation,
    ) {
        if let Some(previous) = *previous_end {
            let source_has_blank_line = self.count_newlines(previous, range.start) >= 2;
            if matches!(*previous_separation, SequenceSeparation::IfMultiline)
                && !source_has_blank_line
                && let Some(previous) = out.pop()
            {
                out.push(self.blank_line_after_if_multiline(previous));
            }

            out.push(self.hard_line());
            if source_has_blank_line || matches!(*previous_separation, SequenceSeparation::Always) {
                out.push(self.hard_line());
            }
        }

        out.push(document);
        *previous_end = Some(range.end);
        *previous_separation = separation;
    }

    /// A brace-delimited body of sibling nodes (`{ ... }`), always laid out on
    /// multiple lines when non-empty. Empty bodies collapse to `{}` unless they
    /// hold dangling comments, which are preserved inside. `right_brace` is the
    /// offset of the closing brace, used to flush those comments.
    fn format_braced_sequence<T>(&mut self, nodes: &[T], right_brace: u32) -> Document<'arena, A>
    where
        T: Format<'arena, A> + HasSpan,
    {
        self.format_braced_sequence_with_empty_body(nodes, right_brace, false)
    }

    fn format_expanded_braced_sequence<T>(
        &mut self,
        nodes: &[T],
        right_brace: u32,
    ) -> Document<'arena, A>
    where
        T: Format<'arena, A> + HasSpan,
    {
        self.format_braced_sequence_with_empty_body(nodes, right_brace, true)
    }

    fn format_braced_sequence_with_empty_body<T>(
        &mut self,
        nodes: &[T],
        right_brace: u32,
        expand_empty: bool,
    ) -> Document<'arena, A>
    where
        T: Format<'arena, A> + HasSpan,
    {
        let items = self.format_sequence(nodes, right_brace);
        if items.is_empty() {
            return if expand_empty {
                self.concat([self.text("{"), self.hard_line(), self.text("}")])
            } else {
                self.text("{}")
            };
        }

        let mut indented = self.vec();
        indented.push(self.hard_line());
        indented.push(Document::Array(items));

        self.concat([
            self.text("{"),
            self.indent(indented),
            self.hard_line(),
            self.text("}"),
        ])
    }
}

fn should_hug_expression(expression: &Expression<'_>) -> bool {
    matches!(
        expression.unparenthesized(),
        Expression::Vec(_) | Expression::Dict(_) | Expression::Tuple(_) | Expression::Match(_)
    )
}

#[derive(Clone, Copy)]
enum SequenceSeparation {
    Preserve,
    IfMultiline,
    Always,
}
