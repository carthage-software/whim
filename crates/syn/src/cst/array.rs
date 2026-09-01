//! Collection literals (`vec`, `dict`, tuples) and the `[]` subscript
//! operators (indexing and append).

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Keyword;
use crate::cst::expression::Expression;
use crate::cst::sequence::TokenSeparatedSequence;

/// A vector literal: `vec[1, 2, 3]`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct VecExpression<'arena> {
    pub vec: Keyword<'arena>,
    pub left_bracket: Span,
    pub elements: TokenSeparatedSequence<'arena, VecElement<'arena>>,
    pub right_bracket: Span,
}

/// A filled vector expression: `vec[value; size]`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct VecFillExpression<'arena> {
    pub vec: Keyword<'arena>,
    pub left_bracket: Span,
    pub value: &'arena Expression<'arena>,
    pub semicolon: Span,
    pub size: &'arena Expression<'arena>,
    pub right_bracket: Span,
}

/// A single element of a [`VecExpression`]: a value, or a `...` spread of a
/// vec or tuple whose elements are taken in order.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct VecElement<'arena> {
    pub ellipsis: Option<Span>,
    pub value: &'arena Expression<'arena>,
}

/// A dictionary literal: `dict[1 => 'a', $k => 'b']`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictExpression<'arena> {
    pub dict: Keyword<'arena>,
    pub left_bracket: Span,
    pub entries: TokenSeparatedSequence<'arena, DictEntry<'arena>>,
    pub right_bracket: Span,
}

/// A single entry of a [`DictExpression`]: a `key => value` pair, or a `...`
/// spread contributing every entry of another collection.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum DictEntry<'arena> {
    Pair(DictPair<'arena>),
    Spread(DictSpread<'arena>),
}

/// A `key => value` entry of a [`DictExpression`]. The key is any expression.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictPair<'arena> {
    pub key: &'arena Expression<'arena>,
    pub double_arrow: Span,
    pub value: &'arena Expression<'arena>,
}

/// A `...` spread entry of a [`DictExpression`].
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictSpread<'arena> {
    pub ellipsis: Span,
    pub value: &'arena Expression<'arena>,
}

/// A tuple literal: `(1, 'a', true)`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TupleExpression<'arena> {
    pub left_parenthesis: Span,
    pub elements: TokenSeparatedSequence<'arena, TupleElement<'arena>>,
    pub right_parenthesis: Span,
}

/// A single element of a [`TupleExpression`]: a value, or a trailing `...`
/// rest.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum TupleElement<'arena> {
    Value(&'arena Expression<'arena>),
    Rest(TupleRest<'arena>),
}

/// A `...` element of a [`TupleExpression`], with the target the remainder is
/// bound to, or nothing when a bare `...` discards it.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TupleRest<'arena> {
    pub ellipsis: Span,
    pub value: Option<&'arena Expression<'arena>>,
}

/// An index into a collection: `$collection[$index]`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ArrayAccess<'arena> {
    pub array: &'arena Expression<'arena>,
    pub left_bracket: Span,
    pub index: &'arena Expression<'arena>,
    pub right_bracket: Span,
}

/// An append target: `$vec[]`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ArrayAppend<'arena> {
    pub array: &'arena Expression<'arena>,
    pub left_bracket: Span,
    pub right_bracket: Span,
}

impl HasSpan for VecExpression<'_> {
    fn span(&self) -> Span {
        self.vec.span().join(self.right_bracket)
    }
}

impl HasSpan for VecFillExpression<'_> {
    fn span(&self) -> Span {
        self.vec.span().join(self.right_bracket)
    }
}

impl VecElement<'_> {
    /// Whether the element is a `...` spread rather than a single value.
    #[inline]
    #[must_use]
    pub const fn is_spread(&self) -> bool {
        self.ellipsis.is_some()
    }
}

impl HasSpan for VecElement<'_> {
    fn span(&self) -> Span {
        self.ellipsis.map_or_else(
            || self.value.span(),
            |ellipsis| ellipsis.join(self.value.span()),
        )
    }
}

impl HasSpan for DictExpression<'_> {
    fn span(&self) -> Span {
        self.dict.span().join(self.right_bracket)
    }
}

impl DictEntry<'_> {
    /// Whether the entry is a `...` spread rather than a `key => value` pair.
    #[inline]
    #[must_use]
    pub const fn is_spread(&self) -> bool {
        matches!(self, DictEntry::Spread(_))
    }
}

impl HasSpan for DictEntry<'_> {
    fn span(&self) -> Span {
        match self {
            DictEntry::Pair(pair) => pair.span(),
            DictEntry::Spread(spread) => spread.span(),
        }
    }
}

impl HasSpan for DictPair<'_> {
    fn span(&self) -> Span {
        self.key.span().join(self.value.span())
    }
}

impl HasSpan for DictSpread<'_> {
    fn span(&self) -> Span {
        self.ellipsis.join(self.value.span())
    }
}

impl HasSpan for TupleExpression<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for TupleElement<'_> {
    fn span(&self) -> Span {
        match self {
            TupleElement::Value(value) => value.span(),
            TupleElement::Rest(rest) => rest.span(),
        }
    }
}

impl HasSpan for TupleRest<'_> {
    fn span(&self) -> Span {
        self.value
            .map_or(self.ellipsis, |value| self.ellipsis.join(value.span()))
    }
}

impl HasSpan for ArrayAccess<'_> {
    fn span(&self) -> Span {
        self.array.leftmost_span().join(self.right_bracket)
    }
}

impl HasSpan for ArrayAppend<'_> {
    fn span(&self) -> Span {
        self.array.leftmost_span().join(self.right_bracket)
    }
}
