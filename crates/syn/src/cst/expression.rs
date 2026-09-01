//! Expressions: the expression tree, plus `throw`, instantiation, and the
//! `name!(...)` language constructs.

use std::ops::ControlFlow;

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::access::Access;
use crate::cst::access::ClassReference;
use crate::cst::array::ArrayAccess;
use crate::cst::array::ArrayAppend;
use crate::cst::array::DictExpression;
use crate::cst::array::TupleExpression;
use crate::cst::array::VecExpression;
use crate::cst::array::VecFillExpression;
use crate::cst::atom::Keyword;
use crate::cst::atom::Literal;
use crate::cst::atom::LiteralInteger;
use crate::cst::atom::Variable;
use crate::cst::call::ArgumentList;
use crate::cst::call::Call;
use crate::cst::call::PartialApplication;
use crate::cst::construct::Construct;
use crate::cst::control_flow::Match;
use crate::cst::function::Closure;
use crate::cst::function::ShortClosure;
use crate::cst::operation::Assignment;
use crate::cst::operation::Binary;
use crate::cst::operation::TypeOperation;
use crate::cst::operation::UnaryPostfix;
use crate::cst::operation::UnaryPrefix;

/// An expression wrapped in parentheses.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Parenthesized<'arena> {
    pub left_parenthesis: Span,
    pub expression: &'arena Expression<'arena>,
    pub right_parenthesis: Span,
}

/// A double-quoted string containing variable or expression interpolations.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct InterpolatedString<'arena> {
    pub opening_quote: Span,
    pub parts: &'arena [InterpolatedStringPart<'arena>],
    pub closing_quote: Span,
}

/// One source-order component of an interpolated string.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum InterpolatedStringPart<'arena> {
    Literal(InterpolatedStringLiteral<'arena>),
    Variable(Variable<'arena>),
    Expression(InterpolatedStringExpression<'arena>),
}

/// Literal bytes between two interpolations.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct InterpolatedStringLiteral<'arena> {
    pub span: Span,
    pub raw: &'arena str,
    pub value: &'arena [u8],
}

/// An arbitrary expression enclosed by `{` and `}` inside a string.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct InterpolatedStringExpression<'arena> {
    pub left_brace: Span,
    pub expression: &'arena Expression<'arena>,
    pub right_brace: Span,
}

/// An Whim expression.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Expression<'arena> {
    Binary(Binary<'arena>),
    UnaryPrefix(UnaryPrefix<'arena>),
    UnaryPostfix(UnaryPostfix<'arena>),
    TypeOperation(TypeOperation<'arena>),
    Assignment(Assignment<'arena>),
    Parenthesized(Parenthesized<'arena>),
    Literal(Literal<'arena>),
    InterpolatedString(InterpolatedString<'arena>),
    Vec(VecExpression<'arena>),
    VecFill(VecFillExpression<'arena>),
    Dict(DictExpression<'arena>),
    Tuple(TupleExpression<'arena>),
    ArrayAccess(ArrayAccess<'arena>),
    ArrayAppend(ArrayAppend<'arena>),
    Variable(Variable<'arena>),
    Access(Access<'arena>),
    Call(Call<'arena>),
    PartialApplication(PartialApplication<'arena>),
    Closure(Closure<'arena>),
    ShortClosure(ShortClosure<'arena>),
    Match(Match<'arena>),
    Instantiation(Instantiation<'arena>),
    Break(Break<'arena>),
    Continue(Continue<'arena>),
    Return(Return<'arena>),
    Throw(Throw<'arena>),
    Construct(Construct<'arena>),
}

impl Expression<'_> {
    /// Strips any number of surrounding parentheses.
    #[inline]
    #[must_use]
    pub const fn unparenthesized(&self) -> &Self {
        let mut expression = self;
        while let Expression::Parenthesized(parenthesized) = expression {
            expression = parenthesized.expression;
        }

        expression
    }

    #[inline]
    #[must_use]
    pub const fn is_literal(&self) -> bool {
        matches!(self.unparenthesized(), Expression::Literal(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_variable(&self) -> bool {
        matches!(self.unparenthesized(), Expression::Variable(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_assignment(&self) -> bool {
        matches!(self.unparenthesized(), Expression::Assignment(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_true(&self) -> bool {
        matches!(
            self.unparenthesized(),
            Expression::Literal(Literal::True(_))
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(
            self.unparenthesized(),
            Expression::Literal(Literal::Null(_))
        )
    }
}

impl HasSpan for Parenthesized<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for InterpolatedString<'_> {
    fn span(&self) -> Span {
        self.opening_quote.join(self.closing_quote)
    }
}

impl HasSpan for InterpolatedStringPart<'_> {
    fn span(&self) -> Span {
        match self {
            Self::Literal(literal) => literal.span(),
            Self::Variable(variable) => variable.span(),
            Self::Expression(expression) => expression.span(),
        }
    }
}

impl HasSpan for InterpolatedStringLiteral<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for InterpolatedStringExpression<'_> {
    fn span(&self) -> Span {
        self.left_brace.join(self.right_brace)
    }
}

pub(crate) type LeftmostStep<'arena> = ControlFlow<Span, &'arena Expression<'arena>>;

impl<'arena> Expression<'arena> {
    /// The span of this expression's leftmost descendant, whose start
    /// position is the whole expression's start. Descends the `lhs`/`array`
    /// spine iteratively, since the parser can build one no deeper than the
    /// source is long.
    #[must_use]
    pub fn leftmost_span(&self) -> Span {
        let mut expression = self;
        loop {
            match expression.leftmost_step() {
                ControlFlow::Continue(next) => expression = next,
                ControlFlow::Break(span) => return span,
            }
        }
    }

    fn leftmost_step(&self) -> LeftmostStep<'arena> {
        match self {
            Expression::Binary(binary) => ControlFlow::Continue(binary.lhs),
            Expression::UnaryPostfix(postfix) => ControlFlow::Continue(postfix.operand),
            Expression::TypeOperation(operation) => ControlFlow::Continue(operation.operand),
            Expression::ArrayAccess(access) => ControlFlow::Continue(access.array),
            Expression::ArrayAppend(append) => ControlFlow::Continue(append.array),
            Expression::Assignment(assignment) => assignment.target.leftmost_step(),
            Expression::Access(access) => access.leftmost_step(),
            Expression::Call(call) => call.leftmost_step(),
            Expression::PartialApplication(application) => application.leftmost_step(),
            other => ControlFlow::Break(other.span()),
        }
    }
}

impl HasSpan for Expression<'_> {
    fn span(&self) -> Span {
        match self {
            Expression::Binary(expression) => expression.span(),
            Expression::UnaryPrefix(expression) => expression.span(),
            Expression::UnaryPostfix(expression) => expression.span(),
            Expression::TypeOperation(expression) => expression.span(),
            Expression::Assignment(expression) => expression.span(),
            Expression::Parenthesized(expression) => expression.span(),
            Expression::Literal(expression) => expression.span(),
            Expression::InterpolatedString(expression) => expression.span(),
            Expression::Vec(expression) => expression.span(),
            Expression::VecFill(expression) => expression.span(),
            Expression::Dict(expression) => expression.span(),
            Expression::Tuple(expression) => expression.span(),
            Expression::ArrayAccess(expression) => expression.span(),
            Expression::ArrayAppend(expression) => expression.span(),
            Expression::Variable(expression) => expression.span(),
            Expression::Access(expression) => expression.span(),
            Expression::Call(expression) => expression.span(),
            Expression::PartialApplication(expression) => expression.span(),
            Expression::Closure(expression) => expression.span(),
            Expression::ShortClosure(expression) => expression.span(),
            Expression::Match(expression) => expression.span(),
            Expression::Instantiation(expression) => expression.span(),
            Expression::Break(expression) => expression.span(),
            Expression::Continue(expression) => expression.span(),
            Expression::Return(expression) => expression.span(),
            Expression::Construct(expression) => expression.span(),
            Expression::Throw(expression) => expression.span(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Break<'arena> {
    pub r#break: Keyword<'arena>,
    pub level: Option<LiteralInteger<'arena>>,
}

impl HasSpan for Break<'_> {
    fn span(&self) -> Span {
        self.level.as_ref().map_or_else(
            || self.r#break.span(),
            |level| self.r#break.span().join(level.span()),
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Continue<'arena> {
    pub r#continue: Keyword<'arena>,
    pub level: Option<LiteralInteger<'arena>>,
}

impl HasSpan for Continue<'_> {
    fn span(&self) -> Span {
        self.level.as_ref().map_or_else(
            || self.r#continue.span(),
            |level| self.r#continue.span().join(level.span()),
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Return<'arena> {
    pub r#return: Keyword<'arena>,
    pub value: Option<&'arena Expression<'arena>>,
}

impl HasSpan for Return<'_> {
    fn span(&self) -> Span {
        self.value.map_or_else(
            || self.r#return.span(),
            |value| self.r#return.span().join(value.span()),
        )
    }
}

/// A `throw` expression.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Throw<'arena> {
    pub throw: Keyword<'arena>,
    pub exception: &'arena Expression<'arena>,
}

impl HasSpan for Throw<'_> {
    fn span(&self) -> Span {
        self.throw.span().join(self.exception.span())
    }
}

/// A `new` expression, with an optional argument list.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Instantiation<'arena> {
    pub new: Keyword<'arena>,
    pub class: ClassReference<'arena>,
    pub argument_list: Option<ArgumentList<'arena>>,
}

impl HasSpan for Instantiation<'_> {
    fn span(&self) -> Span {
        let end = self
            .argument_list
            .as_ref()
            .map_or_else(|| self.class.span(), HasSpan::span);

        self.new.span().join(end)
    }
}

#[cfg(test)]
mod tests {
    use whim_span::HasSpan;

    use crate::arena::LocalArena;
    use crate::cst::statement::Statement;
    use crate::parser::parse;
    use crate::unreachable_invariant;

    #[test]
    fn span_covers_a_long_left_nested_chain_without_recursing() {
        let arena = LocalArena::new();
        let source = format!("$a{};", "+1".repeat(1_000));
        let program = match parse(&arena, &source) {
            Ok(program) => program,
            // SAFETY: the fixture source parses.
            Err(_) => unsafe { unreachable_invariant("fixture source parses") },
        };

        let Some(Statement::Expression(statement)) = program.statements.first() else {
            // SAFETY: the fixture has one expression statement.
            unsafe { unreachable_invariant("fixture is a single expression statement") }
        };

        let span = statement.expression.span();
        assert_eq!(span.start.offset, 0, "the span starts at `$a`");
        assert_eq!(
            span.end.offset,
            source.len() as u32 - 1,
            "the span ends at the last `1`, before the semicolon"
        );
    }
}
