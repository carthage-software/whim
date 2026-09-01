use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Keyword;
use crate::cst::atom::Variable;
use crate::cst::expression::Expression;
use crate::cst::sequence::TokenSeparatedSequence;

/// A non-mutating local binding target used by a `using` statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum BindingTarget<'arena> {
    Variable(Variable<'arena>),
    Tuple(TupleBindingTarget<'arena>),
    Dict(DictBindingTarget<'arena>),
}

/// A nested tuple binding target.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TupleBindingTarget<'arena> {
    pub left_parenthesis: Span,
    pub targets: TokenSeparatedSequence<'arena, ElementBindingTarget<'arena>>,
    pub right_parenthesis: Span,
}

/// A keyed dictionary binding target.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictBindingTarget<'arena> {
    pub dict: Keyword<'arena>,
    pub left_bracket: Span,
    pub entries: TokenSeparatedSequence<'arena, EntryBindingTarget<'arena>>,
    pub right_bracket: Span,
}

/// One key and nested local binding in a keyed dictionary target.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct EntryBindingTarget<'arena> {
    pub key: &'arena Expression<'arena>,
    pub double_arrow: Span,
    pub target: BindingTarget<'arena>,
}

/// One fixed or rest element of a tuple binding target.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ElementBindingTarget<'arena> {
    Target(BindingTarget<'arena>),
    Rest(TrailingBindingTarget<'arena>),
}

/// A trailing rest target, optionally bound to a variable or nested tuple.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TrailingBindingTarget<'arena> {
    pub ellipsis: Span,
    pub target: Option<BindingTarget<'arena>>,
}

impl HasSpan for BindingTarget<'_> {
    fn span(&self) -> Span {
        match self {
            Self::Variable(variable) => variable.span(),
            Self::Tuple(tuple) => tuple.span(),
            Self::Dict(dict) => dict.span(),
        }
    }
}

impl HasSpan for TupleBindingTarget<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for DictBindingTarget<'_> {
    fn span(&self) -> Span {
        self.dict.span().join(self.right_bracket)
    }
}

impl HasSpan for EntryBindingTarget<'_> {
    fn span(&self) -> Span {
        self.key.span().join(self.target.span())
    }
}

impl HasSpan for ElementBindingTarget<'_> {
    fn span(&self) -> Span {
        match self {
            Self::Target(target) => target.span(),
            Self::Rest(rest) => rest.span(),
        }
    }
}

impl HasSpan for TrailingBindingTarget<'_> {
    fn span(&self) -> Span {
        self.target
            .as_ref()
            .map_or(self.ellipsis, |target| self.ellipsis.join(target.span()))
    }
}
