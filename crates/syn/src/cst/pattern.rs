use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Keyword;
use crate::cst::atom::LiteralInteger;
use crate::cst::atom::LiteralString;
use crate::cst::atom::Variable;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::r#type::Type;

impl HasSpan for Pattern<'_> {
    fn span(&self) -> Span {
        match self {
            Self::Variable(variable) => variable.span,
            Self::Parenthesized(pattern) => pattern.span(),
            Self::As(pattern) => pattern.span(),
            Self::Union(pattern) => pattern.left.span().join(pattern.right.span()),
            Self::Vec(pattern) => pattern.vec.span().join(pattern.right_bracket),
            Self::Dict(pattern) => pattern.dict.span().join(pattern.right_bracket),
            Self::Tuple(pattern) => pattern.left_parenthesis.join(pattern.right_parenthesis),
            Self::Type(pattern) => pattern.span(),
        }
    }
}

impl HasSpan for ParenthesizedPattern<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for AsPattern<'_> {
    fn span(&self) -> Span {
        self.left.span().join(self.right.span())
    }
}

impl HasSpan for UnionPattern<'_> {
    fn span(&self) -> Span {
        self.left.span().join(self.right.span())
    }
}

impl HasSpan for VecPattern<'_> {
    fn span(&self) -> Span {
        self.vec.span().join(self.right_bracket)
    }
}

impl HasSpan for DictPattern<'_> {
    fn span(&self) -> Span {
        self.dict.span().join(self.right_bracket)
    }
}

impl HasSpan for DictPatternEntry<'_> {
    fn span(&self) -> Span {
        self.key.span().join(self.pattern.span())
    }
}

impl HasSpan for DictPatternKey<'_> {
    fn span(&self) -> Span {
        match self {
            Self::String(literal) => literal.span,
            Self::Integer { minus, literal } => {
                minus.map_or(literal.span, |minus| minus.join(literal.span))
            }
        }
    }
}

impl HasSpan for TuplePattern<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for TrailingPattern<'_> {
    fn span(&self) -> Span {
        self.pattern
            .map_or(self.ellipsis, |pattern| self.ellipsis.join(pattern.span()))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Pattern<'arena> {
    Variable(Variable<'arena>),
    Parenthesized(ParenthesizedPattern<'arena>),
    As(AsPattern<'arena>),
    Union(UnionPattern<'arena>),
    Vec(VecPattern<'arena>),
    Dict(DictPattern<'arena>),
    Tuple(TuplePattern<'arena>),
    Type(Type<'arena>),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ParenthesizedPattern<'arena> {
    pub left_parenthesis: Span,
    pub pattern: &'arena Pattern<'arena>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UnionPattern<'arena> {
    pub left: &'arena Pattern<'arena>,
    pub pipe: Span,
    pub right: &'arena Pattern<'arena>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct AsPattern<'arena> {
    pub left: &'arena Pattern<'arena>,
    pub at: Span,
    pub right: &'arena Pattern<'arena>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct VecPattern<'arena> {
    pub vec: Keyword<'arena>,
    pub left_bracket: Span,
    pub elements: TokenSeparatedSequence<'arena, Pattern<'arena>>,
    pub trailing: Option<TrailingPattern<'arena>>,
    pub right_bracket: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictPattern<'arena> {
    pub dict: Keyword<'arena>,
    pub left_bracket: Span,
    pub entries: TokenSeparatedSequence<'arena, DictPatternEntry<'arena>>,
    pub trailing: Option<TrailingPattern<'arena>>,
    pub right_bracket: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictPatternEntry<'arena> {
    pub key: DictPatternKey<'arena>,
    pub double_arrow: Span,
    pub pattern: &'arena Pattern<'arena>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum DictPatternKey<'arena> {
    String(LiteralString<'arena>),
    Integer {
        minus: Option<Span>,
        literal: LiteralInteger<'arena>,
    },
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TuplePattern<'arena> {
    pub left_parenthesis: Span,
    pub elements: TokenSeparatedSequence<'arena, Pattern<'arena>>,
    pub trailing: Option<TrailingPattern<'arena>>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TrailingPattern<'arena> {
    pub ellipsis: Span,
    pub pattern: Option<&'arena Pattern<'arena>>,
}
