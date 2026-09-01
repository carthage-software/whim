//! Formatting for calls, partial applications, arguments, closures, and short
//! closures.

use whim_syn::arena::Arena;
use whim_syn::cst::call::Argument;
use whim_syn::cst::call::PartialArgument;
use whim_syn::cst::call::PartialArgumentList;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::function::Closure;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::function::ShortClosure;
use whim_syn::cst::function::ShortClosureBody;

use crate::document::BreakMode;
use crate::document::Document;
use crate::document::Group;
use crate::format::Format;
use crate::format::FormatterState;

impl<'arena, A> FormatterState<'arena, A>
where
    A: Arena,
{
    fn format_closure_parameters(
        &mut self,
        parameter_list: &ParameterList<'arena>,
    ) -> Document<'arena, A> {
        let parameters = parameter_list.format(self);
        if parameter_list.parameters.is_empty() {
            self.never_break(parameters)
        } else {
            parameters
        }
    }

    fn format_short_closure_return_value(
        &mut self,
        expression: &Expression<'arena>,
    ) -> Document<'arena, A> {
        if matches!(expression, Expression::Parenthesized(_)) {
            return expression.format(self);
        }

        let expression = expression.format(self);

        let mut indented = self.vec();
        indented.push(self.soft_line());
        indented.push(expression);

        let mut contents = self.vec();
        contents.push(self.ifbreak(self.text("("), self.empty()));
        contents.push(self.indent_if_break(indented));
        contents.push(self.soft_line());
        contents.push(self.ifbreak(self.text(")"), self.empty()));

        Document::Group(Group::new(contents).with_break_mode(BreakMode::Independent))
    }

    pub(in crate::format) fn format_partial_argument_list(
        &mut self,
        argument_list: &PartialArgumentList<'arena>,
    ) -> Document<'arena, A> {
        if argument_list.arguments.is_empty() {
            return self.concat([self.text("("), self.text(")")]);
        }

        self.delimited_with_break_mode(
            "(",
            argument_list.arguments.as_slice(),
            ")",
            argument_list.right_parenthesis.start.offset,
            BreakMode::Independent,
        )
    }
}

impl<'arena, A> Format<'arena, A> for Argument<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            match self {
                Argument::Positional(argument) => argument.value.format(f),
                Argument::Named(argument) => {
                    let value = argument.value.format(f);
                    f.concat([f.text(argument.name.value), f.text(": "), value])
                }
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for PartialArgument<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            match self {
                PartialArgument::Positional(argument) => argument.value.format(f),
                PartialArgument::Named(argument) => {
                    let value = argument.value.format(f);
                    f.concat([f.text(argument.name.value), f.text(": "), value])
                }
                PartialArgument::NamedPlaceholder(argument) => {
                    f.concat([f.text(argument.name.value), f.text(": ?")])
                }
                PartialArgument::Placeholder(_) => f.text("?"),
                PartialArgument::VariadicPlaceholder(_) => f.text("..."),
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for Closure<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_inline(self.attribute_lists);
        let type_parameters = match &self.type_parameters {
            Some(list) => f.format_type_parameter_list(list),
            None => f.empty(),
        };
        let parameters = f.format_closure_parameters(&self.parameter_list);

        let mut parts = f.vec();
        parts.push(attributes);
        parts.push(f.text("function"));
        parts.push(type_parameters);
        parts.push(parameters);

        if let Some(use_clause) = &self.use_clause {
            let variables = f.delimited(
                "(",
                use_clause.variables.as_slice(),
                ")",
                use_clause.right_parenthesis.start.offset,
                false,
            );
            parts.push(f.text(" use "));
            parts.push(variables);
        }

        if let Some(return_type) = &self.return_type {
            let r#type = return_type.r#type.format(f);
            parts.push(f.text(": "));
            parts.push(r#type);
        }

        parts.push(f.space());
        let body = self.body.format(f);
        parts.push(body);

        Document::Group(Group::new(parts))
    }
}

impl<'arena, A> Format<'arena, A> for ShortClosure<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_inline(self.attribute_lists);
        let type_parameters = match &self.type_parameters {
            Some(list) => f.format_type_parameter_list(list),
            None => f.empty(),
        };

        let parameters = f.format_closure_parameters(&self.parameter_list);
        let mut parts = f.vec();
        parts.push(attributes);
        parts.push(f.text("fn"));
        parts.push(type_parameters);
        parts.push(parameters);
        if let Some(return_type) = &self.return_type {
            let r#type = return_type.r#type.format(f);
            parts.push(f.text(": "));
            parts.push(r#type);
        }

        match &self.body {
            ShortClosureBody::Expression { expression, .. } => {
                parts.push(f.text(" => "));
                let value = f.format_short_closure_return_value(expression);
                parts.push(value);
            }
            ShortClosureBody::Block(block) => {
                parts.push(f.space());
                parts.push(block.format(f));
            }
        }

        Document::Group(Group::new(parts))
    }
}
