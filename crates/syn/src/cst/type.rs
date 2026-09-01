//! Types as written in source, and type alias declarations.

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Identifier;
use crate::cst::atom::Keyword;
use crate::cst::atom::Literal;
use crate::cst::atom::LiteralFloat;
use crate::cst::atom::LiteralInteger;
use crate::cst::atom::LocalIdentifier;
use crate::cst::declaration::AttributeList;
use crate::cst::sequence::TokenSeparatedSequence;

/// A type, as written in parameter/property/return positions, in a type
/// alias, or on the right-hand side of `is` / `as`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Type<'arena> {
    Named(NamedType<'arena>),
    Literal(Literal<'arena>),
    NegativeLiteral(NegativeLiteralType<'arena>),
    IntegerRange(IntegerRangeType<'arena>),
    Union(UnionType<'arena>),
    Intersection(IntersectionType<'arena>),
    Negated(NegatedType<'arena>),
    Parenthesized(ParenthesizedType<'arena>),
    Function(FunctionType<'arena>),
    Array(ArrayType<'arena>),
    Vec(VecType<'arena>),
    VecShape(VecShapeType<'arena>),
    Dict(DictType<'arena>),
    DictShape(DictShapeType<'arena>),
    Classname(ClassnameType<'arena>),
    Tuple(TupleType<'arena>),
    String(Keyword<'arena>),
    Int(Keyword<'arena>),
    Float(Keyword<'arena>),
    Bool(Keyword<'arena>),
    Void(Keyword<'arena>),
    Mixed(Keyword<'arena>),
    Never(Keyword<'arena>),
    Object(Keyword<'arena>),
    Self_(SelfType<'arena>),
    Parent(Keyword<'arena>),
    Static(Keyword<'arena>),
}

/// A read-only view over a vec, dict, or tuple, with key and value types.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ArrayType<'arena> {
    pub array: Keyword<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
}

/// A negative integer or float literal type, such as `-1` or `-1.5`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum NegativeLiteralType<'arena> {
    Integer {
        minus: Span,
        literal: LiteralInteger<'arena>,
    },
    Float {
        minus: Span,
        literal: LiteralFloat<'arena>,
    },
}

/// An integer range type: `N..`, `N..M`, `N..=M`, `..M`, or `..=M`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct IntegerRangeType<'arena> {
    pub lower: Option<IntegerRangeBound<'arena>>,
    pub operator: IntegerRangeOperator,
    pub upper: Option<IntegerRangeBound<'arena>>,
}

/// One signed integer-literal endpoint of an [`IntegerRangeType`].
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum IntegerRangeBound<'arena> {
    Positive(LiteralInteger<'arena>),
    Negative {
        minus: Span,
        literal: LiteralInteger<'arena>,
    },
}

/// Whether an integer range includes its upper endpoint.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum IntegerRangeOperator {
    Exclusive(Span),
    Inclusive(Span),
}

/// A vector type: bare `vec` (any vector), or `vec<T>` (a vector of `T`).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct VecType<'arena> {
    pub vec: Keyword<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
}

/// The fixed positions and optional tail of a `vec[...]` shape.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct VecShapeType<'arena> {
    pub vec: Keyword<'arena>,
    pub left_bracket: Span,
    pub elements: TokenSeparatedSequence<'arena, Type<'arena>>,
    pub trailing_type: Option<TrailingType<'arena>>,
    pub right_bracket: Span,
}

/// A dictionary type: bare `dict` (any dictionary), or `dict<K, V>` (a
/// dictionary from `K` to `V`).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictType<'arena> {
    pub dict: Keyword<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
}

/// The named entries and optional tail of a `dict[...]` shape.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictShapeType<'arena> {
    pub dict: Keyword<'arena>,
    pub left_bracket: Span,
    pub entries: TokenSeparatedSequence<'arena, DictShapeTypeEntry<'arena>>,
    pub rest: Option<DictShapeRest<'arena>>,
    pub right_bracket: Span,
}

/// One fixed key or homogeneous tail in a dictionary shape.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictShapeTypeEntry<'arena> {
    pub key: Literal<'arena>,
    pub double_arrow: Span,
    pub value: &'arena Type<'arena>,
}

/// The homogeneous `...<K, V>` tail of an open dictionary shape.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictShapeRest<'arena> {
    pub ellipsis: Span,
    pub type_arguments: DictTypeArguments<'arena>,
    pub trailing_comma: Option<Span>,
}

/// The `<K, V>` key/value list of a parametric [`DictType`].
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DictTypeArguments<'arena> {
    pub less_than: Span,
    pub key: &'arena Type<'arena>,
    pub comma: Span,
    pub value: &'arena Type<'arena>,
    pub greater_than: Span,
}

/// A class name type: `classname<T>`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ClassnameType<'arena> {
    pub classname: Keyword<'arena>,
    pub less_than: Span,
    pub inner: &'arena Type<'arena>,
    pub greater_than: Span,
}

/// A tuple type: `(T1, T2)` or `(T1, ...T)`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TupleType<'arena> {
    pub left_parenthesis: Span,
    pub elements: TokenSeparatedSequence<'arena, Type<'arena>>,
    pub trailing_type: Option<TrailingType<'arena>>,
    pub right_parenthesis: Span,
}

/// A tuple type's `...T` tail. An omitted type means `mixed`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TrailingType<'arena> {
    pub ellipsis: Span,
    pub r#type: Option<&'arena Type<'arena>>,
}

/// A named type: an identifier with an optional type-argument list, such as
/// `Mailer` or `Vector<int, string>`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NamedType<'arena> {
    pub identifier: Identifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub member: Option<MemberType<'arena>>,
}

/// A member of a named class-like, such as `SeekWhence::Set`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct MemberType<'arena> {
    pub double_colon: Span,
    pub name: LocalIdentifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
}

/// The enclosing class type, optionally narrowed to one member.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct SelfType<'arena> {
    pub self_: Keyword<'arena>,
    pub member: Option<MemberType<'arena>>,
}

/// A list of type arguments: `<int, string>`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeArgumentList<'arena> {
    pub less_than: Span,
    pub arguments: TokenSeparatedSequence<'arena, TypeArgument<'arena>>,
    pub greater_than: Span,
}

/// A single type argument: a type.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeArgument<'arena> {
    pub r#type: &'arena Type<'arena>,
}

/// A list of type parameters: `<in T: Bound = Default, U>`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeParameterList<'arena> {
    pub less_than: Span,
    pub parameters: TokenSeparatedSequence<'arena, TypeParameter<'arena>>,
    pub greater_than: Span,
}

/// A single type parameter: an optional variance marker, a name, an optional
/// bound, and an optional default.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeParameter<'arena> {
    pub variance: Option<TypeVariance<'arena>>,
    pub name: LocalIdentifier<'arena>,
    pub bound: Option<TypeParameterBound<'arena>>,
    pub default: Option<TypeParameterDefault<'arena>>,
}

/// The variance marker on a [`TypeParameter`]: `in` (contravariant) or `out`
/// (covariant).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum TypeVariance<'arena> {
    In(Keyword<'arena>),
    Out(Keyword<'arena>),
}

/// One or more upper bounds for a type parameter.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeParameterBound<'arena> {
    pub colon: Span,
    pub types: TokenSeparatedSequence<'arena, &'arena Type<'arena>>,
}

/// A type parameter's default: `= Type`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeParameterDefault<'arena> {
    pub equals: Span,
    pub r#type: &'arena Type<'arena>,
}

/// A function type: bare `fn` (any callable), or `fn(T1, =T2): R` (a callable
/// of that shape).
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FunctionType<'arena> {
    pub r#fn: Keyword<'arena>,
    pub signature: Option<FunctionTypeSignature<'arena>>,
}

/// The `(T1, =T2): R` shape of a parametric [`FunctionType`].
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FunctionTypeSignature<'arena> {
    pub left_parenthesis: Span,
    pub parameters: TokenSeparatedSequence<'arena, FunctionTypeParameter<'arena>>,
    pub right_parenthesis: Span,
    pub colon: Span,
    pub return_type: &'arena Type<'arena>,
}

/// A single parameter of a [`FunctionType`]: a type, optionally marked with a
/// leading `=` for a parameter the caller may omit.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FunctionTypeParameter<'arena> {
    pub equals: Option<Span>,
    pub r#type: &'arena Type<'arena>,
}

/// A union type: `A|B`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UnionType<'arena> {
    pub left: &'arena Type<'arena>,
    pub pipe: Span,
    pub right: &'arena Type<'arena>,
}

/// An intersection type: `A&B`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct IntersectionType<'arena> {
    pub left: &'arena Type<'arena>,
    pub ampersand: Span,
    pub right: &'arena Type<'arena>,
}

/// A negated type: `!T`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NegatedType<'arena> {
    pub bang: Span,
    pub r#type: &'arena Type<'arena>,
}

/// A parenthesized type: `(A&B)|C`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ParenthesizedType<'arena> {
    pub left_parenthesis: Span,
    pub r#type: &'arena Type<'arena>,
    pub right_parenthesis: Span,
}

impl Type<'_> {
    /// This type with any wrapping parentheses removed.
    #[inline]
    #[must_use]
    pub const fn unparenthesized(&self) -> &Self {
        let mut r#type = self;
        while let Type::Parenthesized(parenthesized) = r#type {
            r#type = parenthesized.r#type;
        }

        r#type
    }

    /// The span of this type's leftmost token, found by descending the
    /// union/intersection chain iteratively rather than recursing.
    #[must_use]
    pub fn leftmost_span(&self) -> Span {
        let mut current = self;
        loop {
            match current {
                Type::Union(union) => current = union.left,
                Type::Intersection(intersection) => current = intersection.left,
                other => return other.span(),
            }
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_union(&self) -> bool {
        matches!(self, Type::Union(_))
    }
}

impl HasSpan for Type<'_> {
    fn span(&self) -> Span {
        match self {
            Type::Named(named) => named.span(),
            Type::Literal(literal) => literal.span(),
            Type::NegativeLiteral(literal) => literal.span(),
            Type::IntegerRange(range) => range.span(),
            Type::Union(union) => union.span(),
            Type::Intersection(intersection) => intersection.span(),
            Type::Negated(negated) => negated.span(),
            Type::Parenthesized(parenthesized) => parenthesized.span(),
            Type::Function(function) => function.span(),
            Type::Array(array) => array.span(),
            Type::Vec(vec) => vec.span(),
            Type::VecShape(shape) => shape.span(),
            Type::Dict(dict) => dict.span(),
            Type::DictShape(shape) => shape.span(),
            Type::Classname(classname) => classname.span(),
            Type::Tuple(tuple) => tuple.span(),
            Type::String(keyword)
            | Type::Int(keyword)
            | Type::Float(keyword)
            | Type::Bool(keyword)
            | Type::Void(keyword)
            | Type::Mixed(keyword)
            | Type::Never(keyword)
            | Type::Object(keyword)
            | Type::Parent(keyword)
            | Type::Static(keyword) => keyword.span(),
            Type::Self_(self_type) => self_type.span(),
        }
    }
}

impl HasSpan for SelfType<'_> {
    fn span(&self) -> Span {
        self.member.as_ref().map_or_else(
            || self.self_.span(),
            |member| self.self_.span().join(member.span()),
        )
    }
}

impl HasSpan for NegativeLiteralType<'_> {
    fn span(&self) -> Span {
        match self {
            NegativeLiteralType::Integer { minus, literal } => minus.join(literal.span),
            NegativeLiteralType::Float { minus, literal } => minus.join(literal.span),
        }
    }
}

impl HasSpan for IntegerRangeType<'_> {
    fn span(&self) -> Span {
        let start = self
            .lower
            .as_ref()
            .map_or_else(|| self.operator.span(), HasSpan::span);
        let end = self
            .upper
            .as_ref()
            .map_or_else(|| self.operator.span(), HasSpan::span);

        start.join(end)
    }
}

impl HasSpan for IntegerRangeBound<'_> {
    fn span(&self) -> Span {
        match self {
            IntegerRangeBound::Positive(literal) => literal.span,
            IntegerRangeBound::Negative { minus, literal } => minus.join(literal.span),
        }
    }
}

impl HasSpan for IntegerRangeOperator {
    fn span(&self) -> Span {
        match self {
            Self::Exclusive(span) | Self::Inclusive(span) => *span,
        }
    }
}

impl HasSpan for NamedType<'_> {
    fn span(&self) -> Span {
        let span = self.type_arguments.as_ref().map_or_else(
            || self.identifier.span(),
            |arguments| self.identifier.span().join(arguments.span()),
        );

        self.member
            .as_ref()
            .map_or(span, |member| span.join(member.span()))
    }
}

impl HasSpan for MemberType<'_> {
    fn span(&self) -> Span {
        self.type_arguments.as_ref().map_or_else(
            || self.name.span(),
            |arguments| self.name.span().join(arguments.span()),
        )
    }
}

impl HasSpan for TypeArgumentList<'_> {
    fn span(&self) -> Span {
        self.less_than.join(self.greater_than)
    }
}

impl HasSpan for TypeArgument<'_> {
    fn span(&self) -> Span {
        self.r#type.span()
    }
}

impl HasSpan for TypeParameterList<'_> {
    fn span(&self) -> Span {
        self.less_than.join(self.greater_than)
    }
}

impl HasSpan for TypeParameter<'_> {
    fn span(&self) -> Span {
        let start = self
            .variance
            .as_ref()
            .map_or_else(|| self.name.span(), HasSpan::span);
        let end = match (&self.default, &self.bound) {
            (Some(default), _) => default.r#type.span(),
            (None, Some(bound)) => bound.span(),
            (None, None) => self.name.span(),
        };

        start.join(end)
    }
}

impl HasSpan for TypeVariance<'_> {
    fn span(&self) -> Span {
        match self {
            Self::In(keyword) | Self::Out(keyword) => keyword.span(),
        }
    }
}

impl HasSpan for TypeParameterBound<'_> {
    fn span(&self) -> Span {
        self.colon.join(self.types.span(self.colon.end))
    }
}

impl HasSpan for TypeParameterDefault<'_> {
    fn span(&self) -> Span {
        self.equals.join(self.r#type.span())
    }
}

impl HasSpan for FunctionType<'_> {
    fn span(&self) -> Span {
        self.signature.as_ref().map_or_else(
            || self.r#fn.span(),
            |signature| self.r#fn.span().join(signature.return_type.span()),
        )
    }
}

impl HasSpan for FunctionTypeSignature<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.return_type.span())
    }
}

impl HasSpan for FunctionTypeParameter<'_> {
    fn span(&self) -> Span {
        self.equals.map_or_else(
            || self.r#type.span(),
            |equals| equals.join(self.r#type.span()),
        )
    }
}

impl HasSpan for ClassnameType<'_> {
    fn span(&self) -> Span {
        self.classname.span().join(self.greater_than)
    }
}

impl HasSpan for VecType<'_> {
    fn span(&self) -> Span {
        self.type_arguments.as_ref().map_or_else(
            || self.vec.span(),
            |arguments| self.vec.span().join(arguments.span()),
        )
    }
}

impl HasSpan for ArrayType<'_> {
    fn span(&self) -> Span {
        self.type_arguments.as_ref().map_or_else(
            || self.array.span(),
            |arguments| self.array.span().join(arguments.span()),
        )
    }
}

impl HasSpan for VecShapeType<'_> {
    fn span(&self) -> Span {
        self.vec.span().join(self.right_bracket)
    }
}

impl HasSpan for DictType<'_> {
    fn span(&self) -> Span {
        self.type_arguments.as_ref().map_or_else(
            || self.dict.span(),
            |arguments| self.dict.span().join(arguments.span()),
        )
    }
}

impl HasSpan for DictShapeType<'_> {
    fn span(&self) -> Span {
        self.dict.span().join(self.right_bracket)
    }
}

impl HasSpan for DictShapeTypeEntry<'_> {
    fn span(&self) -> Span {
        self.key.span().join(self.value.span())
    }
}

impl HasSpan for DictShapeRest<'_> {
    fn span(&self) -> Span {
        self.ellipsis.join(self.type_arguments.greater_than)
    }
}

impl HasSpan for TupleType<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for TrailingType<'_> {
    fn span(&self) -> Span {
        self.r#type
            .map_or(self.ellipsis, |r#type| self.ellipsis.join(r#type.span()))
    }
}

impl HasSpan for UnionType<'_> {
    fn span(&self) -> Span {
        self.left.leftmost_span().join(self.right.span())
    }
}

impl HasSpan for IntersectionType<'_> {
    fn span(&self) -> Span {
        self.left.leftmost_span().join(self.right.span())
    }
}

impl HasSpan for NegatedType<'_> {
    fn span(&self) -> Span {
        self.bang.join(self.r#type.span())
    }
}

impl HasSpan for ParenthesizedType<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

/// A type alias declaration: a name bound to a type.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TypeAlias<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub r#type: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub equals: Span,
    pub aliased: &'arena Type<'arena>,
    pub semicolon: Span,
}

impl HasSpan for TypeAlias<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.semicolon);
        }

        self.r#type.span().join(self.semicolon)
    }
}

/// A nominal type backed by another runtime-checkable type.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Newtype<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub newtype: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub equals: Span,
    pub backing: &'arena Type<'arena>,
    pub semicolon: Span,
}

impl HasSpan for Newtype<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.semicolon);
        }

        self.newtype.span().join(self.semicolon)
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
    fn span_covers_a_long_union_chain_without_recursing() {
        let arena = LocalArena::new();
        let source = format!("type T = int{};", "|int".repeat(1_000));
        let program = match parse(&arena, &source) {
            Ok(program) => program,
            // SAFETY: the fixture source parses.
            Err(_) => unsafe { unreachable_invariant("fixture source parses") },
        };

        let Some(Statement::TypeAlias(alias)) = program.statements.first() else {
            // SAFETY: the fixture has one type alias.
            unsafe { unreachable_invariant("fixture is a single type alias") }
        };

        let span = alias.aliased.span();
        assert_eq!(
            span.start.offset,
            "type T = ".len() as u32,
            "the span starts at the first `int`"
        );
        assert_eq!(
            span.end.offset,
            source.len() as u32 - 1,
            "the span ends at the last `int`, before the semicolon"
        );
    }
}
