//! Operators and operations: binary, unary, type operations, and assignment.

use std::mem;
use std::ops::ControlFlow;

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::access::PropertyAccess;
use crate::cst::access::StaticPropertyAccess;
use crate::cst::array::ArrayAccess;
use crate::cst::array::ArrayAppend;
use crate::cst::atom::Keyword;
use crate::cst::atom::Variable;
use crate::cst::expression::Expression;
use crate::cst::expression::LeftmostStep;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::r#type::Type;
use crate::token::precedence::GetPrecedence;
use crate::token::precedence::Precedence;

/// A binary operator.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum BinaryOperator {
    Addition(Span),
    Subtraction(Span),
    Multiplication(Span),
    Division(Span),
    Modulo(Span),
    Exponentiation(Span),
    BitwiseAnd(Span),
    BitwiseOr(Span),
    BitwiseXor(Span),
    LeftShift(Span),
    RightShift(Span),
    NullCoalesce(Span),
    Equal(Span),
    NotEqual(Span),
    LessThan(Span),
    LessThanOrEqual(Span),
    GreaterThan(Span),
    GreaterThanOrEqual(Span),
    Spaceship(Span),
    Pipe(Span),
    StringConcat(Span),
    And(Span),
    Or(Span),
}

/// A binary operation: two operands joined by a [`BinaryOperator`].
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Binary<'arena> {
    pub lhs: &'arena Expression<'arena>,
    pub operator: BinaryOperator,
    pub rhs: &'arena Expression<'arena>,
}

impl BinaryOperator {
    #[inline]
    #[must_use]
    pub const fn is_multiplicative(&self) -> bool {
        matches!(
            self,
            Self::Multiplication(_) | Self::Division(_) | Self::Modulo(_)
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_additive(&self) -> bool {
        matches!(self, Self::Addition(_) | Self::Subtraction(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_arithmetic(&self) -> bool {
        if self.is_multiplicative() || self.is_additive() {
            return true;
        }

        matches!(self, Self::Exponentiation(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_bit_shift(&self) -> bool {
        matches!(self, Self::LeftShift(_) | Self::RightShift(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_bitwise(&self) -> bool {
        if self.is_bit_shift() {
            return true;
        }

        matches!(
            self,
            Self::BitwiseAnd(_) | Self::BitwiseOr(_) | Self::BitwiseXor(_)
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_comparison(&self) -> bool {
        matches!(
            self,
            Self::Equal(_)
                | Self::NotEqual(_)
                | Self::LessThan(_)
                | Self::LessThanOrEqual(_)
                | Self::GreaterThan(_)
                | Self::GreaterThanOrEqual(_)
                | Self::Spaceship(_)
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_logical(&self) -> bool {
        matches!(self, Self::And(_) | Self::Or(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_pipe(&self) -> bool {
        matches!(self, Self::Pipe(_))
    }

    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Addition(_) => "+",
            Self::Subtraction(_) => "-",
            Self::Multiplication(_) => "*",
            Self::Division(_) => "/",
            Self::Modulo(_) => "%",
            Self::Exponentiation(_) => "**",
            Self::BitwiseAnd(_) => "&",
            Self::BitwiseOr(_) => "|",
            Self::BitwiseXor(_) => "^",
            Self::LeftShift(_) => "<<",
            Self::RightShift(_) => ">>",
            Self::NullCoalesce(_) => "??",
            Self::Equal(_) => "==",
            Self::NotEqual(_) => "!=",
            Self::LessThan(_) => "<",
            Self::LessThanOrEqual(_) => "<=",
            Self::GreaterThan(_) => ">",
            Self::GreaterThanOrEqual(_) => ">=",
            Self::Spaceship(_) => "<=>",
            Self::Pipe(_) => "|>",
            Self::StringConcat(_) => ".",
            Self::And(_) => "&&",
            Self::Or(_) => "||",
        }
    }

    #[inline]
    #[must_use]
    pub fn is_same_as(&self, other: &Self) -> bool {
        mem::discriminant(self) == mem::discriminant(other)
    }
}

impl GetPrecedence for BinaryOperator {
    fn precedence(&self) -> Precedence {
        match self {
            Self::Exponentiation(_) => Precedence::Exponent,
            Self::Multiplication(_) | Self::Division(_) | Self::Modulo(_) => {
                Precedence::Multiplicative
            }
            Self::Addition(_) | Self::Subtraction(_) => Precedence::Additive,
            Self::LeftShift(_) | Self::RightShift(_) => Precedence::Shift,
            Self::BitwiseAnd(_) => Precedence::BitwiseAnd,
            Self::BitwiseXor(_) => Precedence::BitwiseXor,
            Self::BitwiseOr(_) => Precedence::BitwiseOr,
            Self::StringConcat(_) => Precedence::Concat,
            Self::Equal(_)
            | Self::NotEqual(_)
            | Self::LessThan(_)
            | Self::LessThanOrEqual(_)
            | Self::GreaterThan(_)
            | Self::GreaterThanOrEqual(_)
            | Self::Spaceship(_) => Precedence::Comparison,
            Self::Pipe(_) => Precedence::Pipe,
            Self::And(_) => Precedence::And,
            Self::Or(_) => Precedence::Or,
            Self::NullCoalesce(_) => Precedence::Coalesce,
        }
    }
}

impl HasSpan for BinaryOperator {
    fn span(&self) -> Span {
        match self {
            Self::Addition(span)
            | Self::Subtraction(span)
            | Self::Multiplication(span)
            | Self::Division(span)
            | Self::Modulo(span)
            | Self::Exponentiation(span)
            | Self::BitwiseAnd(span)
            | Self::BitwiseOr(span)
            | Self::BitwiseXor(span)
            | Self::LeftShift(span)
            | Self::RightShift(span)
            | Self::NullCoalesce(span)
            | Self::Equal(span)
            | Self::NotEqual(span)
            | Self::LessThan(span)
            | Self::LessThanOrEqual(span)
            | Self::GreaterThan(span)
            | Self::GreaterThanOrEqual(span)
            | Self::Spaceship(span)
            | Self::Pipe(span)
            | Self::StringConcat(span)
            | Self::And(span)
            | Self::Or(span) => *span,
        }
    }
}

impl HasSpan for Binary<'_> {
    fn span(&self) -> Span {
        self.lhs.leftmost_span().join(self.rhs.span())
    }
}

/// A prefix unary operator. All share one precedence level ([`Precedence::Unary`]).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum UnaryPrefixOperator {
    BitwiseNot(Span),
    Not(Span),
    PreIncrement(Span),
    PreDecrement(Span),
    Plus(Span),
    Negation(Span),
}

/// A postfix unary operator.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum UnaryPostfixOperator {
    PostIncrement(Span),
    PostDecrement(Span),
}

/// A prefix unary operation.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UnaryPrefix<'arena> {
    pub operator: UnaryPrefixOperator,
    pub operand: &'arena Expression<'arena>,
}

/// A postfix unary operation.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UnaryPostfix<'arena> {
    pub operand: &'arena Expression<'arena>,
    pub operator: UnaryPostfixOperator,
}

impl UnaryPrefixOperator {
    #[inline]
    #[must_use]
    pub const fn is_arithmetic(&self) -> bool {
        matches!(
            self,
            Self::Plus(_) | Self::Negation(_) | Self::PreIncrement(_) | Self::PreDecrement(_)
        )
    }

    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BitwiseNot(_) => "~",
            Self::Not(_) => "!",
            Self::PreIncrement(_) => "++",
            Self::PreDecrement(_) => "--",
            Self::Plus(_) => "+",
            Self::Negation(_) => "-",
        }
    }

    #[inline]
    #[must_use]
    pub fn is_same_as(&self, other: &Self) -> bool {
        mem::discriminant(self) == mem::discriminant(other)
    }
}

impl UnaryPostfixOperator {
    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PostIncrement(_) => "++",
            Self::PostDecrement(_) => "--",
        }
    }

    #[inline]
    #[must_use]
    pub fn is_same_as(&self, other: &Self) -> bool {
        mem::discriminant(self) == mem::discriminant(other)
    }
}

impl GetPrecedence for UnaryPrefixOperator {
    fn precedence(&self) -> Precedence {
        Precedence::Unary
    }
}

impl GetPrecedence for UnaryPostfixOperator {
    fn precedence(&self) -> Precedence {
        Precedence::Postfix
    }
}

impl HasSpan for UnaryPrefixOperator {
    fn span(&self) -> Span {
        match self {
            Self::BitwiseNot(span)
            | Self::Not(span)
            | Self::PreIncrement(span)
            | Self::PreDecrement(span)
            | Self::Plus(span)
            | Self::Negation(span) => *span,
        }
    }
}

impl HasSpan for UnaryPostfixOperator {
    fn span(&self) -> Span {
        match self {
            Self::PostIncrement(span) | Self::PostDecrement(span) => *span,
        }
    }
}

impl HasSpan for UnaryPrefix<'_> {
    fn span(&self) -> Span {
        self.operator.span().join(self.operand.span())
    }
}

impl HasSpan for UnaryPostfix<'_> {
    fn span(&self) -> Span {
        self.operand.leftmost_span().join(self.operator.span())
    }
}

/// A type operator: `is`, `as`, or `?as`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum TypeOperator<'arena> {
    Check(Keyword<'arena>),
    /// `$value as T`: evaluates to `T`, or throws.
    Assert(Keyword<'arena>),
    AssertOrNull(Span, Keyword<'arena>),
}

/// A type operation: an expression tested against or asserted to a type.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeOperation<'arena> {
    pub operand: &'arena Expression<'arena>,
    pub operator: TypeOperator<'arena>,
    pub r#type: &'arena Type<'arena>,
}

impl TypeOperator<'_> {
    #[inline]
    #[must_use]
    pub const fn is_check(&self) -> bool {
        matches!(self, Self::Check(_))
    }

    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Check(_) => "is",
            Self::Assert(_) => "as",
            Self::AssertOrNull(_, _) => "?as",
        }
    }
}

impl GetPrecedence for TypeOperator<'_> {
    fn precedence(&self) -> Precedence {
        Precedence::TypeOperation
    }
}

impl HasSpan for TypeOperator<'_> {
    fn span(&self) -> Span {
        match self {
            Self::Check(keyword) | Self::Assert(keyword) => keyword.span(),
            Self::AssertOrNull(question_mark, keyword) => question_mark.join(keyword.span()),
        }
    }
}

impl HasSpan for TypeOperation<'_> {
    fn span(&self) -> Span {
        self.operand.leftmost_span().join(self.r#type.span())
    }
}

/// The left-hand side of an assignment.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum AssignmentTarget<'arena> {
    Variable(Variable<'arena>),
    Property(PropertyAccess<'arena>),
    StaticProperty(StaticPropertyAccess<'arena>),
    ArrayIndex(ArrayAccess<'arena>),
    ArrayAppend(ArrayAppend<'arena>),
    Tuple(TupleDestructure<'arena>),
    Dict(DictDestructure<'arena>),
}

/// An assignment operator.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum AssignmentOperator {
    Assign(Span),
    Addition(Span),
    Subtraction(Span),
    Multiplication(Span),
    Division(Span),
    Modulo(Span),
    Exponentiation(Span),
    Concat(Span),
    BitwiseAnd(Span),
    BitwiseOr(Span),
    BitwiseXor(Span),
    LeftShift(Span),
    RightShift(Span),
    Coalesce(Span),
    LogicalAnd(Span),
    LogicalOr(Span),
}

/// An assignment.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Assignment<'arena> {
    pub target: AssignmentTarget<'arena>,
    pub operator: AssignmentOperator,
    pub value: &'arena Expression<'arena>,
}

/// A destructuring pattern: `($a, $b[0], $c->k) = $value`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TupleDestructure<'arena> {
    pub left_parenthesis: Span,
    pub targets: TokenSeparatedSequence<'arena, DestructureTarget<'arena>>,
    pub right_parenthesis: Span,
}

/// A keyed dictionary destructuring pattern.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictDestructure<'arena> {
    pub dict: Keyword<'arena>,
    pub left_bracket: Span,
    pub entries: TokenSeparatedSequence<'arena, DictDestructureEntry<'arena>>,
    pub right_bracket: Span,
}

/// One key and nested assignment target in a keyed dictionary pattern.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictDestructureEntry<'arena> {
    pub key: &'arena Expression<'arena>,
    pub double_arrow: Span,
    pub target: AssignmentTarget<'arena>,
}

/// One element of a [`TupleDestructure`]: a target that takes one element, or
/// a `...` rest that takes every element past the fixed ones.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum DestructureTarget<'arena> {
    Target(AssignmentTarget<'arena>),
    Default(DestructureDefault<'arena>),
    Rest(DestructureRest<'arena>),
}

/// A destructuring target with a value used when its position is absent.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DestructureDefault<'arena> {
    pub target: AssignmentTarget<'arena>,
    pub equals: Span,
    pub value: &'arena Expression<'arena>,
}

/// The `...` element of a [`TupleDestructure`].
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DestructureRest<'arena> {
    pub ellipsis: Span,
    pub target: Option<AssignmentTarget<'arena>>,
}

impl DestructureTarget<'_> {
    /// Whether this element is the `...` rest.
    #[inline]
    #[must_use]
    pub const fn is_rest(&self) -> bool {
        matches!(self, DestructureTarget::Rest(_))
    }
}

impl AssignmentOperator {
    #[inline]
    #[must_use]
    pub const fn is_assign(&self) -> bool {
        matches!(self, Self::Assign(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_arithmetic(&self) -> bool {
        matches!(
            self,
            Self::Addition(_)
                | Self::Subtraction(_)
                | Self::Multiplication(_)
                | Self::Division(_)
                | Self::Modulo(_)
                | Self::Exponentiation(_)
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_bitwise(&self) -> bool {
        matches!(
            self,
            Self::BitwiseAnd(_)
                | Self::BitwiseOr(_)
                | Self::BitwiseXor(_)
                | Self::LeftShift(_)
                | Self::RightShift(_)
        )
    }

    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Assign(_) => "=",
            Self::Addition(_) => "+=",
            Self::Subtraction(_) => "-=",
            Self::Multiplication(_) => "*=",
            Self::Division(_) => "/=",
            Self::Modulo(_) => "%=",
            Self::Exponentiation(_) => "**=",
            Self::Concat(_) => ".=",
            Self::BitwiseAnd(_) => "&=",
            Self::BitwiseOr(_) => "|=",
            Self::BitwiseXor(_) => "^=",
            Self::LeftShift(_) => "<<=",
            Self::RightShift(_) => ">>=",
            Self::Coalesce(_) => "??=",
            Self::LogicalAnd(_) => "&&=",
            Self::LogicalOr(_) => "||=",
        }
    }
}

impl AssignmentTarget<'_> {
    #[inline]
    #[must_use]
    pub const fn is_variable(&self) -> bool {
        matches!(self, Self::Variable(_))
    }
}

impl GetPrecedence for AssignmentOperator {
    fn precedence(&self) -> Precedence {
        Precedence::Assignment
    }
}

impl HasSpan for AssignmentOperator {
    fn span(&self) -> Span {
        match self {
            Self::Assign(span)
            | Self::Addition(span)
            | Self::Subtraction(span)
            | Self::Multiplication(span)
            | Self::Division(span)
            | Self::Modulo(span)
            | Self::Exponentiation(span)
            | Self::Concat(span)
            | Self::BitwiseAnd(span)
            | Self::BitwiseOr(span)
            | Self::BitwiseXor(span)
            | Self::LeftShift(span)
            | Self::RightShift(span)
            | Self::Coalesce(span)
            | Self::LogicalAnd(span)
            | Self::LogicalOr(span) => *span,
        }
    }
}

impl HasSpan for Assignment<'_> {
    fn span(&self) -> Span {
        self.target.leftmost_span().join(self.value.span())
    }
}

impl<'arena> AssignmentTarget<'arena> {
    pub(crate) fn leftmost_step(&self) -> LeftmostStep<'arena> {
        match self {
            AssignmentTarget::Property(target) => ControlFlow::Continue(target.object),
            AssignmentTarget::StaticProperty(target) => target.class.leftmost_step(),
            AssignmentTarget::ArrayIndex(target) => ControlFlow::Continue(target.array),
            AssignmentTarget::ArrayAppend(target) => ControlFlow::Continue(target.array),
            AssignmentTarget::Variable(target) => ControlFlow::Break(target.span()),
            AssignmentTarget::Tuple(target) => ControlFlow::Break(target.span()),
            AssignmentTarget::Dict(target) => ControlFlow::Break(target.span()),
        }
    }

    /// The span of this target's leftmost descendant; see
    /// [`Expression::leftmost_span`].
    #[must_use]
    pub fn leftmost_span(&self) -> Span {
        match self.leftmost_step() {
            ControlFlow::Continue(expression) => expression.leftmost_span(),
            ControlFlow::Break(span) => span,
        }
    }
}

impl HasSpan for AssignmentTarget<'_> {
    fn span(&self) -> Span {
        match self {
            AssignmentTarget::Variable(target) => target.span(),
            AssignmentTarget::Property(target) => target.span(),
            AssignmentTarget::StaticProperty(target) => target.span(),
            AssignmentTarget::ArrayIndex(target) => target.span(),
            AssignmentTarget::ArrayAppend(target) => target.span(),
            AssignmentTarget::Tuple(target) => target.span(),
            AssignmentTarget::Dict(target) => target.span(),
        }
    }
}

impl HasSpan for DestructureTarget<'_> {
    fn span(&self) -> Span {
        match self {
            DestructureTarget::Target(target) => target.span(),
            DestructureTarget::Default(default) => default.span(),
            DestructureTarget::Rest(rest) => rest.span(),
        }
    }
}

impl HasSpan for DestructureDefault<'_> {
    fn span(&self) -> Span {
        self.target.span().join(self.value.span())
    }
}

impl HasSpan for DestructureRest<'_> {
    fn span(&self) -> Span {
        self.target
            .as_ref()
            .map_or(self.ellipsis, |target| self.ellipsis.join(target.span()))
    }
}

impl HasSpan for TupleDestructure<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for DictDestructure<'_> {
    fn span(&self) -> Span {
        self.dict.span().join(self.right_bracket)
    }
}

impl HasSpan for DictDestructureEntry<'_> {
    fn span(&self) -> Span {
        self.key.span().join(self.target.span())
    }
}

#[cfg(test)]
mod tests {
    use whim_span::Position;
    use whim_span::Span;

    use crate::cst::operation::BinaryOperator;

    fn span(offset: u32) -> Span {
        Span::new(Position::new(offset), Position::new(offset + 1))
    }

    #[test]
    fn is_same_as_ignores_span() {
        let first = BinaryOperator::Addition(span(0));
        let second = BinaryOperator::Addition(span(10));
        assert!(first.is_same_as(&second), "same variant, different spans");

        let subtraction = BinaryOperator::Subtraction(span(0));
        assert!(!first.is_same_as(&subtraction));
    }
}
