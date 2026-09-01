//! Formatting for expressions and everything that only appears inside them.

use whim_syn::arena::Arena;
use whim_syn::arena::Vec;

use whim_syn::cst::access::Access;
use whim_syn::cst::access::ClassReference;
use whim_syn::cst::array::ArrayAppend;
use whim_syn::cst::array::DictEntry;
use whim_syn::cst::array::DictExpression;
use whim_syn::cst::array::TupleElement;
use whim_syn::cst::array::TupleExpression;
use whim_syn::cst::array::VecElement;
use whim_syn::cst::array::VecExpression;
use whim_syn::cst::array::VecFillExpression;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::atom::LiteralString;
use whim_syn::cst::atom::LiteralStringKind;
use whim_syn::cst::atom::Variable;
use whim_syn::cst::binding::BindingTarget;
use whim_syn::cst::binding::DictBindingTarget;
use whim_syn::cst::binding::ElementBindingTarget;
use whim_syn::cst::binding::EntryBindingTarget;
use whim_syn::cst::binding::TupleBindingTarget;
use whim_syn::cst::control_flow::Match;
use whim_syn::cst::control_flow::MatchArm;
use whim_syn::cst::expression::Break;
use whim_syn::cst::expression::Continue;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::expression::Instantiation;
use whim_syn::cst::expression::InterpolatedString;
use whim_syn::cst::expression::InterpolatedStringExpression;
use whim_syn::cst::expression::InterpolatedStringLiteral;
use whim_syn::cst::expression::InterpolatedStringPart;
use whim_syn::cst::expression::Return;
use whim_syn::cst::expression::Throw;
use whim_syn::cst::operation::Assignment;
use whim_syn::cst::operation::AssignmentTarget;
use whim_syn::cst::operation::DestructureTarget;
use whim_syn::cst::operation::DictDestructure;
use whim_syn::cst::operation::DictDestructureEntry;
use whim_syn::cst::operation::TupleDestructure;
use whim_syn::cst::operation::TypeOperator;
use whim_syn::cst::operation::UnaryPrefix;
use whim_syn::cst::operation::UnaryPrefixOperator;
use whim_syn::cst::pattern::DictPatternEntry;
use whim_syn::cst::pattern::DictPatternKey;
use whim_syn::cst::pattern::Pattern;
use whim_syn::cst::pattern::TrailingPattern;
use whim_syn::cst::sequence::TokenSeparatedSequence;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeArgumentList;

use crate::document::Document;
use crate::document::Group;
use crate::format::Format;
use crate::format::FormatterState;
use crate::format::spine::format_assignment_target_spine;
use crate::format::spine::format_spine;

impl<'arena, A> FormatterState<'arena, A>
where
    A: Arena,
{
    /// Renders a string literal, preferring single quotes when the content can
    /// be represented single-quoted without reinterpreting any escape.
    pub(in crate::format) fn format_string_literal(
        &self,
        literal: &LiteralString<'arena>,
    ) -> Document<'arena, A> {
        let raw = literal.raw;
        if literal.kind == LiteralStringKind::DoubleQuoted && raw.len() >= 2 {
            let inner = &raw[1..raw.len() - 1];
            if !inner.contains('\\') && !inner.contains('\'') {
                let single_quoted = self.arena.alloc_fmt(format_args!("'{inner}'"));

                return self.text(single_quoted);
            }
        }

        self.text(raw)
    }

    pub(in crate::format) fn format_class_reference(
        &mut self,
        class: &ClassReference<'arena>,
    ) -> Document<'arena, A> {
        match class {
            ClassReference::Named(named) => {
                let identifier = self.text(named.identifier.value());
                match &named.type_arguments {
                    Some(type_arguments) => {
                        let turbofish = self.format_turbofish(type_arguments);
                        self.concat([identifier, turbofish])
                    }
                    None => identifier,
                }
            }
            ClassReference::Self_(keyword)
            | ClassReference::Parent(keyword)
            | ClassReference::Static(keyword) => self.text(keyword.value),
            ClassReference::Expression(expression) => expression.format(self),
        }
    }

    fn format_turbofish(&mut self, list: &TypeArgumentList<'arena>) -> Document<'arena, A> {
        let arguments = self.format_type_argument_list(list);
        self.concat([self.text("::"), arguments])
    }

    /// Formats an optional turbofish, or nothing when absent.
    pub(in crate::format) fn format_optional_turbofish(
        &mut self,
        type_arguments: Option<&TypeArgumentList<'arena>>,
    ) -> Document<'arena, A> {
        match type_arguments {
            Some(list) => self.format_turbofish(list),
            None => self.empty(),
        }
    }
}

impl<'arena, A> Format<'arena, A> for Expression<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, { format_expression(self, f) })
    }
}

/// Formats an expression itself, without the comments written around it.
fn format_expression<'arena, A>(
    expression: &Expression<'arena>,
    f: &mut FormatterState<'arena, A>,
) -> Document<'arena, A>
where
    A: Arena,
{
    if let Some(document) = format_spine(expression, f) {
        return document;
    }

    {
        match expression {
            Expression::UnaryPrefix(node) => node.format(f),
            Expression::Assignment(node) => node.format(f),
            Expression::Parenthesized(node) => {
                let inner = node.expression.format(f);
                f.parenthesized(inner)
            }
            Expression::Literal(node) => node.format(f),
            Expression::InterpolatedString(node) => node.format(f),
            Expression::Vec(node) => node.format(f),
            Expression::VecFill(node) => node.format(f),
            Expression::Dict(node) => node.format(f),
            Expression::Tuple(node) => node.format(f),
            Expression::ArrayAppend(node) => node.format(f),
            Expression::Variable(node) => node.format(f),
            Expression::Access(Access::Constant(node)) => f.text(node.name.value()),
            Expression::Closure(node) => node.format(f),
            Expression::ShortClosure(node) => node.format(f),
            Expression::Match(node) => node.format(f),
            Expression::Instantiation(node) => node.format(f),
            Expression::Break(node) => node.format(f),
            Expression::Continue(node) => node.format(f),
            Expression::Return(node) => node.format(f),
            Expression::Throw(node) => node.format(f),
            Expression::Construct(node) => node.format(f),
            Expression::Binary(_)
            | Expression::UnaryPostfix(_)
            | Expression::TypeOperation(_)
            | Expression::ArrayAccess(_)
            | Expression::Access(_)
            | Expression::Call(_)
            | Expression::PartialApplication(_) => {
                unreachable!("expression spine nodes are formatted before the fallback")
            }
        }
    }
}

impl<'arena, A> Format<'arena, A> for Break<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match &self.level {
            Some(level) => f.concat([f.text("break"), f.space(), f.text(level.raw)]),
            None => f.text("break"),
        }
    }
}

impl<'arena, A> Format<'arena, A> for Continue<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match &self.level {
            Some(level) => f.concat([f.text("continue"), f.space(), f.text(level.raw)]),
            None => f.text("continue"),
        }
    }
}

impl<'arena, A> Format<'arena, A> for Return<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let Some(value) = self.value else {
            return f.text("return");
        };
        let value = value.unparenthesized();
        let document = value.format(f);
        if !matches!(value, Expression::Binary(_) | Expression::TypeOperation(_)) {
            return f.concat([f.text("return"), f.space(), document]);
        }

        let mut indented = f.vec();
        indented.push(f.soft_line());
        indented.push(document);

        let mut contents = f.vec();
        contents.push(f.text("return"));
        contents.push(f.ifbreak(f.text(" ("), f.space()));
        contents.push(f.indent_if_break(indented));
        contents.push(f.soft_line());
        contents.push(f.ifbreak(f.text(")"), f.empty()));

        Document::Group(Group::new(contents))
    }
}

impl<'arena, A> Format<'arena, A> for InterpolatedString<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let mut documents = Vec::with_capacity_in(self.parts.len() + 2, f.arena);
        documents.push(f.text("\""));
        for part in self.parts {
            documents.push(part.format(f));
        }
        documents.push(f.text("\""));

        f.concat(documents)
    }
}

impl<'arena, A> Format<'arena, A> for InterpolatedStringPart<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            Self::Literal(literal) => literal.format(f),
            Self::Variable(variable) => variable.format(f),
            Self::Expression(expression) => expression.format(f),
        }
    }
}

impl<'arena, A> Format<'arena, A> for InterpolatedStringLiteral<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.text(self.raw)
    }
}

impl<'arena, A> Format<'arena, A> for InterpolatedStringExpression<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let expression = self.expression.format(f);
        f.concat([f.text("{"), expression, f.text("}")])
    }
}

impl<'arena, A> Format<'arena, A> for Variable<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.text(self.name)
    }
}

impl<'arena, A> Format<'arena, A> for Literal<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            Literal::String(literal) => f.format_string_literal(literal),
            Literal::Integer(literal) => f.text(literal.raw),
            Literal::Float(literal) => f.text(literal.raw),
            Literal::True(keyword) | Literal::False(keyword) | Literal::Null(keyword) => {
                f.text(keyword.value)
            }
        }
    }
}

impl<'arena, A> Format<'arena, A> for UnaryPrefix<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let operand = self.operand.format(f);
        let operator = f.text(self.operator.as_str());

        if needs_space_between_prefix_operators(&self.operator, self.operand) {
            f.concat([operator, f.space(), operand])
        } else {
            f.concat([operator, operand])
        }
    }
}

pub(in crate::format) const fn type_operator_text(operator: TypeOperator<'_>) -> &'static str {
    match operator {
        TypeOperator::Check(_) => "is",
        TypeOperator::Assert(_) => "as",
        TypeOperator::AssertOrNull(..) => "?as",
    }
}

impl<'arena, A> Format<'arena, A> for Assignment<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let target = self.target.format(f);
        let value = self.value.format(f);

        if !matches!(self.value.unparenthesized(), Expression::Binary(_)) {
            return f.concat([
                target,
                f.space(),
                f.text(self.operator.as_str()),
                f.space(),
                value,
            ]);
        }

        let mut value_line = f.vec();
        value_line.push(f.line());
        value_line.push(value);

        let mut contents = f.vec();
        contents.push(target);
        contents.push(f.space());
        contents.push(f.text(self.operator.as_str()));
        contents.push(f.indent_if_break(value_line));

        Document::Group(Group::new(contents))
    }
}

impl<'arena, A> Format<'arena, A> for AssignmentTarget<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        if let Some(document) = format_assignment_target_spine(self, f) {
            return document;
        }

        match self {
            AssignmentTarget::Variable(variable) => f.text(variable.name),
            AssignmentTarget::ArrayAppend(append) => append.format(f),
            AssignmentTarget::Tuple(destructure) => destructure.format(f),
            AssignmentTarget::Dict(destructure) => destructure.format(f),
            AssignmentTarget::Property(_)
            | AssignmentTarget::StaticProperty(_)
            | AssignmentTarget::ArrayIndex(_) => {
                unreachable!("access-like assignment targets are formatted as spines")
            }
        }
    }
}

impl<'arena, A> Format<'arena, A> for ArrayAppend<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let array = self.array.format(f);
        f.concat([array, f.text("[]")])
    }
}

impl<'arena, A> Format<'arena, A> for TupleDestructure<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.format_tuple(
            self.targets.as_slice(),
            self.right_parenthesis.start.offset,
            true,
        )
    }
}

impl<'arena, A> Format<'arena, A> for DictDestructure<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.delimited(
            "dict[",
            self.entries.as_slice(),
            "]",
            self.right_bracket.start.offset,
            false,
        )
    }
}

impl<'arena, A> Format<'arena, A> for DictDestructureEntry<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let key = self.key.format(f);
        let target = self.target.format(f);
        f.concat([key, f.text(" => "), target])
    }
}

impl<'arena, A> Format<'arena, A> for VecExpression<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.delimited(
            "vec[",
            self.elements.as_slice(),
            "]",
            self.right_bracket.start.offset,
            false,
        )
    }
}

impl<'arena, A> Format<'arena, A> for VecFillExpression<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let mut parts = f.vec();
        let value = self.value.format(f);
        let size = self.size.format(f);
        parts.push(f.text("vec["));
        parts.push(value);
        parts.push(f.text("; "));
        parts.push(size);
        f.take_interior_comments(self.right_bracket.start.offset, &mut parts);
        parts.push(f.text("]"));
        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for VecElement<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            let value = self.value.format(f);
            if self.ellipsis.is_some() {
                f.concat([f.text("..."), value])
            } else {
                value
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for DictExpression<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.delimited(
            "dict[",
            self.entries.as_slice(),
            "]",
            self.right_bracket.start.offset,
            false,
        )
    }
}

impl<'arena, A> Format<'arena, A> for DictEntry<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            match self {
                DictEntry::Pair(pair) => {
                    let key = pair.key.format(f);
                    let value = pair.value.format(f);
                    f.concat([key, f.text(" => "), value])
                }
                DictEntry::Spread(spread) => {
                    let value = spread.value.format(f);
                    f.concat([f.text("..."), value])
                }
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for TupleExpression<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.format_tuple(
            self.elements.as_slice(),
            self.right_parenthesis.start.offset,
            true,
        )
    }
}

impl<'arena, A> Format<'arena, A> for TupleElement<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            match self {
                TupleElement::Value(value) => value.format(f),
                TupleElement::Rest(rest) => match rest.value {
                    Some(value) => {
                        let value = value.format(f);
                        f.concat([f.text("..."), value])
                    }
                    None => f.text("..."),
                },
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for DestructureTarget<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            match self {
                DestructureTarget::Target(target) => target.format(f),
                DestructureTarget::Default(default) => {
                    let target = default.target.format(f);
                    let value = default.value.format(f);
                    f.concat([target, f.text(" = "), value])
                }
                DestructureTarget::Rest(rest) => match &rest.target {
                    Some(target) => {
                        let target = target.format(f);
                        f.concat([f.text("..."), target])
                    }
                    None => f.text("..."),
                },
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for Match<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let expression = self.expression.format(f);
        let subject = f.parenthesized(expression);
        let arms = f.delimited(
            "{",
            self.arms.as_slice(),
            "}",
            self.right_brace.start.offset,
            true,
        );

        f.concat([f.text("match"), f.space(), subject, f.space(), arms])
    }
}

impl<'arena, A> Format<'arena, A> for MatchArm<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            let pattern = self.pattern.format(f);
            let expression = self.expression.format(f);
            f.concat([pattern, f.text(" => "), expression])
        })
    }
}

impl<'arena, A> Format<'arena, A> for Pattern<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            Pattern::Variable(variable) => variable.format(f),
            Pattern::Type(r#type) if pattern_type_is_wildcard(r#type) => f.text("$_"),
            Pattern::Type(r#type) => r#type.format(f),
            Pattern::Parenthesized(pattern) => {
                let inner = pattern.pattern.format(f);
                f.concat([f.text("("), inner, f.text(")")])
            }
            Pattern::As(pattern) => {
                let (left, right) = if pattern_binds(pattern.right) && !pattern_binds(pattern.left)
                {
                    (pattern.right, pattern.left)
                } else {
                    (pattern.left, pattern.right)
                };
                let left = left.format(f);
                let right = right.format(f);
                f.concat([left, f.text(" @ "), right])
            }
            Pattern::Union(pattern) => {
                let left = pattern.left.format(f);
                let right = pattern.right.format(f);
                f.concat([left, f.text(" | "), right])
            }
            Pattern::Vec(pattern) => {
                let elements =
                    format_pattern_elements(f, &pattern.elements, pattern.trailing.as_ref());
                f.concat([f.text("vec["), elements, f.text("]")])
            }
            Pattern::Dict(pattern) => {
                let entries = f.inline_token_sequence(&pattern.entries);
                let entries = match &pattern.trailing {
                    Some(trailing) if pattern.entries.is_empty() => trailing.format(f),
                    Some(trailing) => {
                        let trailing = trailing.format(f);
                        f.concat([entries, f.text(", "), trailing])
                    }
                    None => entries,
                };
                f.concat([f.text("dict["), entries, f.text("]")])
            }
            Pattern::Tuple(pattern) => {
                let elements =
                    format_pattern_elements(f, &pattern.elements, pattern.trailing.as_ref());
                let trailing_comma = if pattern.elements.len() == 1 && pattern.trailing.is_none() {
                    f.text(",")
                } else {
                    f.empty()
                };
                f.concat([f.text("("), elements, trailing_comma, f.text(")")])
            }
        }
    }
}

fn pattern_type_is_wildcard(r#type: &Type<'_>) -> bool {
    matches!(
        r#type.unparenthesized(),
        Type::Named(named)
            if matches!(
                &named.identifier,
                Identifier::Local(local) if local.value == "_"
            ) && named.type_arguments.is_none()
    )
}

fn pattern_binds(pattern: &Pattern<'_>) -> bool {
    match pattern {
        Pattern::Variable(_) => true,
        Pattern::Parenthesized(pattern) => pattern_binds(pattern.pattern),
        Pattern::As(pattern) => pattern_binds(pattern.left) || pattern_binds(pattern.right),
        Pattern::Union(pattern) => pattern_binds(pattern.left) || pattern_binds(pattern.right),
        Pattern::Vec(pattern) => {
            pattern.elements.iter().any(pattern_binds)
                || pattern
                    .trailing
                    .and_then(|trailing| trailing.pattern)
                    .is_some_and(pattern_binds)
        }
        Pattern::Dict(pattern) => {
            pattern
                .entries
                .iter()
                .any(|entry| pattern_binds(entry.pattern))
                || pattern
                    .trailing
                    .and_then(|trailing| trailing.pattern)
                    .is_some_and(pattern_binds)
        }
        Pattern::Tuple(pattern) => {
            pattern.elements.iter().any(pattern_binds)
                || pattern
                    .trailing
                    .and_then(|trailing| trailing.pattern)
                    .is_some_and(pattern_binds)
        }
        Pattern::Type(_) => false,
    }
}

impl<'arena, A> Format<'arena, A> for TrailingPattern<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self.pattern {
            Some(pattern) => {
                let pattern = pattern.format(f);
                f.concat([f.text("..."), pattern])
            }
            None => f.text("..."),
        }
    }
}

fn format_pattern_elements<'arena, A>(
    f: &mut FormatterState<'arena, A>,
    elements: &TokenSeparatedSequence<'arena, Pattern<'arena>>,
    trailing: Option<&TrailingPattern<'arena>>,
) -> Document<'arena, A>
where
    A: Arena,
{
    let elements_document = f.inline_token_sequence(elements);
    match trailing {
        Some(trailing) if elements.is_empty() => trailing.format(f),
        Some(trailing) => {
            let trailing = trailing.format(f);
            f.concat([elements_document, f.text(", "), trailing])
        }
        None => elements_document,
    }
}

impl<'arena, A> Format<'arena, A> for DictPatternEntry<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let key = self.key.format(f);
        let pattern = self.pattern.format(f);
        f.concat([key, f.text(" => "), pattern])
    }
}

impl<'arena, A> Format<'arena, A> for DictPatternKey<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            Self::String(literal) => f.format_string_literal(literal),
            Self::Integer { minus, literal } => {
                let literal = f.text(literal.raw);
                if minus.is_some() {
                    f.concat([f.text("-"), literal])
                } else {
                    literal
                }
            }
        }
    }
}

impl<'arena, A> Format<'arena, A> for BindingTarget<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            BindingTarget::Variable(variable) => f.text(variable.name),
            BindingTarget::Tuple(tuple) => tuple.format(f),
            BindingTarget::Dict(dict) => dict.format(f),
        }
    }
}

impl<'arena, A> Format<'arena, A> for TupleBindingTarget<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.format_tuple(
            self.targets.as_slice(),
            self.right_parenthesis.start.offset,
            true,
        )
    }
}

impl<'arena, A> Format<'arena, A> for DictBindingTarget<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.delimited(
            "dict[",
            self.entries.as_slice(),
            "]",
            self.right_bracket.start.offset,
            false,
        )
    }
}

impl<'arena, A> Format<'arena, A> for EntryBindingTarget<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let key = self.key.format(f);
        let target = self.target.format(f);
        f.concat([key, f.text(" => "), target])
    }
}

impl<'arena, A> Format<'arena, A> for ElementBindingTarget<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            match self {
                ElementBindingTarget::Target(target) => target.format(f),
                ElementBindingTarget::Rest(rest) => match &rest.target {
                    Some(target) => {
                        let target = target.format(f);
                        f.concat([f.text("..."), target])
                    }
                    None => f.text("..."),
                },
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for Instantiation<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let class = f.format_class_reference(&self.class);

        let mut parts = f.vec();
        parts.push(f.text("new"));
        parts.push(f.space());
        parts.push(class);
        if let Some(argument_list) = &self.argument_list {
            let arguments = f.format_argument_list(argument_list);
            parts.push(arguments);
        }

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for Throw<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let exception = self.exception.format(f);
        f.concat([f.text("throw"), f.space(), exception])
    }
}

const fn needs_space_between_prefix_operators(
    operator: &UnaryPrefixOperator,
    operand: &Expression<'_>,
) -> bool {
    if !is_sign_or_step(operator) {
        return false;
    }

    match operand.unparenthesized() {
        Expression::UnaryPrefix(inner) => is_sign_or_step(&inner.operator),
        _ => false,
    }
}

#[inline]
const fn is_sign_or_step(operator: &UnaryPrefixOperator) -> bool {
    matches!(
        operator,
        UnaryPrefixOperator::Plus(_)
            | UnaryPrefixOperator::Negation(_)
            | UnaryPrefixOperator::PreIncrement(_)
            | UnaryPrefixOperator::PreDecrement(_)
    )
}
