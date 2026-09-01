//! Control flow: if, match, loops, and try.

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Keyword;
use crate::cst::atom::Variable;
use crate::cst::expression::Expression;
use crate::cst::operation::AssignmentTarget;
use crate::cst::pattern::Pattern;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::statement::Block;
use crate::cst::r#type::Type;

/// An `if` statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct If<'arena> {
    pub r#if: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub condition: &'arena Expression<'arena>,
    pub right_parenthesis: Span,
    pub body: Block<'arena>,
    pub r#else: Option<Else<'arena>>,
}

/// The `else` clause of an [`If`] statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Else<'arena> {
    pub r#else: Keyword<'arena>,
    pub body: ElseBody<'arena>,
}

/// The body of an [`Else`] clause: either a chained `if` or a block.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ElseBody<'arena> {
    If(&'arena If<'arena>),
    Block(Block<'arena>),
}

/// A `match` expression.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Match<'arena> {
    pub r#match: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub expression: &'arena Expression<'arena>,
    pub right_parenthesis: Span,
    pub left_brace: Span,
    pub arms: TokenSeparatedSequence<'arena, MatchArm<'arena>>,
    pub right_brace: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct MatchArm<'arena> {
    pub pattern: &'arena Pattern<'arena>,
    pub double_arrow: Span,
    pub expression: &'arena Expression<'arena>,
}

/// A `while` loop.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct While<'arena> {
    pub r#while: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub condition: &'arena Expression<'arena>,
    pub right_parenthesis: Span,
    pub body: Block<'arena>,
}

/// A `do`-`while` loop.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DoWhile<'arena> {
    pub r#do: Keyword<'arena>,
    pub body: Block<'arena>,
    pub r#while: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub condition: &'arena Expression<'arena>,
    pub right_parenthesis: Span,
    pub semicolon: Span,
}

/// A `for` loop.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct For<'arena> {
    pub r#for: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub initializations: TokenSeparatedSequence<'arena, Expression<'arena>>,
    pub initializations_semicolon: Span,
    pub conditions: TokenSeparatedSequence<'arena, Expression<'arena>>,
    pub conditions_semicolon: Span,
    pub increments: TokenSeparatedSequence<'arena, Expression<'arena>>,
    pub right_parenthesis: Span,
    pub body: Block<'arena>,
}

/// A `foreach` loop.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Foreach<'arena> {
    pub foreach: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub expression: &'arena Expression<'arena>,
    pub r#as: Keyword<'arena>,
    pub target: ForeachTarget<'arena>,
    pub right_parenthesis: Span,
    pub body: Block<'arena>,
}

/// The target of a [`Foreach`] loop.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ForeachTarget<'arena> {
    Value(ForeachValueTarget<'arena>),
    KeyValue(ForeachKeyValueTarget<'arena>),
}

/// A foreach target that only binds the value: `foreach ($items as $value)`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ForeachValueTarget<'arena> {
    pub value: &'arena AssignmentTarget<'arena>,
}

/// A foreach target that binds both the key and the value:
/// `foreach ($items as $key => $value)`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ForeachKeyValueTarget<'arena> {
    pub key: &'arena AssignmentTarget<'arena>,
    pub double_arrow: Span,
    pub value: &'arena AssignmentTarget<'arena>,
}

impl<'arena> ForeachTarget<'arena> {
    /// The key target, if the loop binds one.
    #[inline]
    #[must_use]
    pub const fn key(&self) -> Option<&'arena AssignmentTarget<'arena>> {
        match self {
            ForeachTarget::Value(_) => None,
            ForeachTarget::KeyValue(target) => Some(target.key),
        }
    }

    /// The value target.
    #[inline]
    #[must_use]
    pub const fn value(&self) -> &'arena AssignmentTarget<'arena> {
        match self {
            ForeachTarget::Value(target) => target.value,
            ForeachTarget::KeyValue(target) => target.value,
        }
    }
}

/// A `try` statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Try<'arena> {
    pub r#try: Keyword<'arena>,
    pub block: Block<'arena>,
    pub catch_clauses: &'arena [TryCatchClause<'arena>],
    pub else_clause: Option<TryElseClause<'arena>>,
    pub finally_clause: Option<TryFinallyClause<'arena>>,
}

/// A `catch` clause of a [`Try`] statement: `catch (NotFound $e) { }`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TryCatchClause<'arena> {
    pub r#catch: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub r#type: &'arena Type<'arena>,
    pub variable: Option<Variable<'arena>>,
    pub right_parenthesis: Span,
    pub guard: Option<TryCatchGuard<'arena>>,
    pub block: Block<'arena>,
}

/// The optional `if (condition)` guard of a [`TryCatchClause`].
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TryCatchGuard<'arena> {
    pub r#if: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub condition: &'arena Expression<'arena>,
    pub right_parenthesis: Span,
}

/// The success-only `else` clause of a [`Try`] statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TryElseClause<'arena> {
    pub r#else: Keyword<'arena>,
    pub block: Block<'arena>,
}

/// A `finally` clause of a [`Try`] statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TryFinallyClause<'arena> {
    pub r#finally: Keyword<'arena>,
    pub block: Block<'arena>,
}

impl HasSpan for If<'_> {
    fn span(&self) -> Span {
        let end = self
            .r#else
            .as_ref()
            .map_or_else(|| self.body.span(), HasSpan::span);

        self.r#if.span().join(end)
    }
}

impl HasSpan for Else<'_> {
    fn span(&self) -> Span {
        self.r#else.span().join(self.body.span())
    }
}

impl HasSpan for ElseBody<'_> {
    fn span(&self) -> Span {
        match self {
            ElseBody::If(r#if) => r#if.span(),
            ElseBody::Block(block) => block.span(),
        }
    }
}

impl HasSpan for Match<'_> {
    fn span(&self) -> Span {
        self.r#match.span().join(self.right_brace)
    }
}

impl HasSpan for MatchArm<'_> {
    fn span(&self) -> Span {
        self.pattern.span().join(self.expression.span())
    }
}

impl HasSpan for While<'_> {
    fn span(&self) -> Span {
        self.r#while.span().join(self.body.span())
    }
}

impl HasSpan for DoWhile<'_> {
    fn span(&self) -> Span {
        self.r#do.span().join(self.semicolon)
    }
}

impl HasSpan for For<'_> {
    fn span(&self) -> Span {
        self.r#for.span().join(self.body.span())
    }
}

impl HasSpan for Foreach<'_> {
    fn span(&self) -> Span {
        self.foreach.span().join(self.body.span())
    }
}

impl HasSpan for ForeachTarget<'_> {
    fn span(&self) -> Span {
        match self {
            ForeachTarget::Value(target) => target.span(),
            ForeachTarget::KeyValue(target) => target.span(),
        }
    }
}

impl HasSpan for ForeachValueTarget<'_> {
    fn span(&self) -> Span {
        self.value.span()
    }
}

impl HasSpan for ForeachKeyValueTarget<'_> {
    fn span(&self) -> Span {
        self.key.span().join(self.value.span())
    }
}

impl HasSpan for Try<'_> {
    fn span(&self) -> Span {
        let end = if let Some(finally) = &self.finally_clause {
            finally.span()
        } else if let Some(r#else) = &self.else_clause {
            r#else.span()
        } else if let Some(catch) = self.catch_clauses.last() {
            catch.span()
        } else {
            self.block.span()
        };

        self.r#try.span().join(end)
    }
}

impl HasSpan for TryCatchClause<'_> {
    fn span(&self) -> Span {
        self.r#catch.span().join(self.block.span())
    }
}

impl HasSpan for TryCatchGuard<'_> {
    fn span(&self) -> Span {
        self.r#if.span().join(self.right_parenthesis)
    }
}

impl HasSpan for TryElseClause<'_> {
    fn span(&self) -> Span {
        self.r#else.span().join(self.block.span())
    }
}

impl HasSpan for TryFinallyClause<'_> {
    fn span(&self) -> Span {
        self.r#finally.span().join(self.block.span())
    }
}
