//! Formatting for language constructs (`require!`, `length!`, `write!`, ...).

use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::atom::LiteralString;
use whim_syn::cst::construct::Construct;
use whim_syn::cst::construct::ConstructArgument;

use crate::document::Document;
use crate::format::Format;
use crate::format::FormatterState;

impl<'arena, A> Format<'arena, A> for ConstructArgument<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        self.value.format(f)
    }
}

impl<'arena, A> Format<'arena, A> for Construct<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            Construct::Require(c) => {
                f.format_construct(c.name.value, &[c.value], c.right_parenthesis.start.offset)
            }
            Construct::RequireOnce(c) => {
                f.format_construct(c.name.value, &[c.value], c.right_parenthesis.start.offset)
            }
            Construct::Length(c) => {
                f.format_construct(c.name.value, &[c.value], c.right_parenthesis.start.offset)
            }
            Construct::Contains(c) => f.format_construct(
                c.name.value,
                &[c.array, c.value],
                c.right_parenthesis.start.offset,
            ),
            Construct::ContainsKey(c) => f.format_construct(
                c.name.value,
                &[c.array, c.key],
                c.right_parenthesis.start.offset,
            ),
            Construct::Clone(c) => {
                let mut parts = f.vec();
                parts.push(f.text(c.name.value));
                parts.push(f.text("!("));
                let object = c.object.format(f);
                parts.push(object);
                for field in c.fields {
                    parts.push(f.text(", "));
                    parts.push(f.text(field.name.value));
                    parts.push(f.text(": "));
                    let value = field.value.format(f);
                    parts.push(value);
                }
                f.take_interior_comments(c.right_parenthesis.start.offset, &mut parts);
                parts.push(f.text(")"));
                Document::Array(parts)
            }
            Construct::Remove(c) => f.format_construct(
                c.name.value,
                &[c.array, c.key],
                c.right_parenthesis.start.offset,
            ),
            Construct::SwapRemove(c) => f.format_construct(
                c.name.value,
                &[c.vector, c.index],
                c.right_parenthesis.start.offset,
            ),
            Construct::RemoveFirst(c) => {
                f.format_construct(c.name.value, &[c.array], c.right_parenthesis.start.offset)
            }
            Construct::RemoveLast(c) => {
                f.format_construct(c.name.value, &[c.array], c.right_parenthesis.start.offset)
            }
            Construct::Assert(c) => match &c.message {
                Some(message) => f.format_construct(
                    c.name.value,
                    &[c.condition, message.value],
                    c.right_parenthesis.start.offset,
                ),
                None => f.format_construct(
                    c.name.value,
                    &[c.condition],
                    c.right_parenthesis.start.offset,
                ),
            },
            Construct::Exit(c) => match c.code {
                Some(code) => {
                    f.format_construct(c.name.value, &[code], c.right_parenthesis.start.offset)
                }
                None => f.format_construct(c.name.value, &[], c.right_parenthesis.start.offset),
            },
            Construct::Panic(c) => {
                format_literal_construct(f, c.name.value, &c.message, c.right_parenthesis)
            }
            Construct::Write(c) => f.format_variadic_construct(
                c.name.value,
                &c.arguments,
                c.right_parenthesis.start.offset,
            ),
            Construct::WriteLine(c) => f.format_variadic_construct(
                c.name.value,
                &c.arguments,
                c.right_parenthesis.start.offset,
            ),
            Construct::WriteError(c) => f.format_variadic_construct(
                c.name.value,
                &c.arguments,
                c.right_parenthesis.start.offset,
            ),
            Construct::WriteErrorLine(c) => f.format_variadic_construct(
                c.name.value,
                &c.arguments,
                c.right_parenthesis.start.offset,
            ),
            Construct::Debug(c) => f.format_variadic_construct(
                c.name.value,
                &c.arguments,
                c.right_parenthesis.start.offset,
            ),
            Construct::Discard(c) => {
                f.format_construct(c.name.value, &[c.value], c.right_parenthesis.start.offset)
            }
            Construct::Drop(c) => {
                let mut parts = f.vec();
                parts.push(f.text(c.name.value));
                parts.push(f.text("!("));
                for (index, variable) in c.variables.iter().enumerate() {
                    if index != 0 {
                        parts.push(f.text(", "));
                    }
                    parts.push(f.text(variable.name));
                }
                f.take_interior_comments(c.right_parenthesis.start.offset, &mut parts);
                parts.push(f.text(")"));
                Document::Array(parts)
            }
            Construct::File(c) => {
                f.format_construct(c.name.value, &[], c.right_parenthesis.start.offset)
            }
            Construct::Directory(c) => {
                f.format_construct(c.name.value, &[], c.right_parenthesis.start.offset)
            }
            Construct::Embed(c) => {
                format_literal_construct(f, c.name.value, &c.path, c.right_parenthesis)
            }
        }
    }
}

fn format_literal_construct<'arena, A>(
    f: &mut FormatterState<'arena, A>,
    name: &'arena str,
    literal: &LiteralString<'arena>,
    right_parenthesis: Span,
) -> Document<'arena, A>
where
    A: Arena,
{
    let span = literal.span;
    let leading = f.print_leading_comments(span);
    let literal = f.format_string_literal(literal);
    let trailing = f.print_trailing_comments(span);
    let literal = f.with_comments(leading, literal, trailing);
    let mut parts = f.vec();
    parts.push(f.text(name));
    parts.push(f.text("!("));
    parts.push(literal);
    f.take_interior_comments(right_parenthesis.start.offset, &mut parts);
    parts.push(f.text(")"));
    Document::Array(parts)
}
