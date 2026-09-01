use std::ops::ControlFlow;

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Identifier;
use crate::cst::atom::Keyword;
use crate::cst::atom::LocalIdentifier;
use crate::cst::atom::Variable;
use crate::cst::expression::Expression;
use crate::cst::expression::LeftmostStep;
use crate::cst::r#type::TypeArgumentList;

/// A class or object used before `::`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ClassReference<'arena> {
    Named(NamedClassReference<'arena>),
    Self_(Keyword<'arena>),
    Parent(Keyword<'arena>),
    Static(Keyword<'arena>),
    Expression(&'arena Expression<'arena>),
}

/// A named class reference with an optional turbofish: `Foo` or `Foo::<int>`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NamedClassReference<'arena> {
    pub identifier: Identifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
}

/// A member access expression.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Access<'arena> {
    Constant(ConstantAccess<'arena>),
    Property(PropertyAccess<'arena>),
    NullSafeProperty(NullSafePropertyAccess<'arena>),
    StaticProperty(StaticPropertyAccess<'arena>),
    ClassConstant(ClassConstantAccess<'arena>),
}

/// A reference to a constant by name: `FOO`, `Bar\BAZ`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ConstantAccess<'arena> {
    pub name: Identifier<'arena>,
}

/// `$object->property`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PropertyAccess<'arena> {
    pub object: &'arena Expression<'arena>,
    pub arrow: Span,
    pub property: LocalIdentifier<'arena>,
}

/// `$object?->property`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NullSafePropertyAccess<'arena> {
    pub object: &'arena Expression<'arena>,
    pub question_mark_arrow: Span,
    pub property: LocalIdentifier<'arena>,
}

/// `Foo::$property`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct StaticPropertyAccess<'arena> {
    pub class: ClassReference<'arena>,
    pub double_colon: Span,
    pub property: Variable<'arena>,
}

/// `Foo::CONSTANT`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ClassConstantAccess<'arena> {
    pub class: ClassReference<'arena>,
    pub double_colon: Span,
    pub constant: LocalIdentifier<'arena>,
}

impl<'arena> ClassReference<'arena> {
    pub(crate) fn leftmost_step(&self) -> LeftmostStep<'arena> {
        match self {
            ClassReference::Expression(expression) => ControlFlow::Continue(expression),
            other => ControlFlow::Break(other.span()),
        }
    }

    /// The span of this reference's leftmost descendant; see
    /// [`Expression::leftmost_span`].
    #[must_use]
    pub fn leftmost_span(&self) -> Span {
        match self.leftmost_step() {
            ControlFlow::Continue(expression) => expression.leftmost_span(),
            ControlFlow::Break(span) => span,
        }
    }
}

impl<'arena> Access<'arena> {
    pub(crate) fn leftmost_step(&self) -> LeftmostStep<'arena> {
        match self {
            Access::Property(access) => ControlFlow::Continue(access.object),
            Access::NullSafeProperty(access) => ControlFlow::Continue(access.object),
            Access::StaticProperty(access) => access.class.leftmost_step(),
            Access::ClassConstant(access) => access.class.leftmost_step(),
            Access::Constant(access) => ControlFlow::Break(access.span()),
        }
    }
}

impl HasSpan for ClassReference<'_> {
    fn span(&self) -> Span {
        match self {
            ClassReference::Named(named) => named.span(),
            ClassReference::Self_(keyword)
            | ClassReference::Parent(keyword)
            | ClassReference::Static(keyword) => keyword.span(),
            ClassReference::Expression(expression) => expression.span(),
        }
    }
}

impl HasSpan for NamedClassReference<'_> {
    fn span(&self) -> Span {
        self.type_arguments.as_ref().map_or_else(
            || self.identifier.span(),
            |arguments| self.identifier.span().join(arguments.span()),
        )
    }
}

impl HasSpan for Access<'_> {
    fn span(&self) -> Span {
        match self {
            Access::Constant(access) => access.span(),
            Access::Property(access) => access.span(),
            Access::NullSafeProperty(access) => access.span(),
            Access::StaticProperty(access) => access.span(),
            Access::ClassConstant(access) => access.span(),
        }
    }
}

impl HasSpan for ConstantAccess<'_> {
    fn span(&self) -> Span {
        self.name.span()
    }
}

impl HasSpan for PropertyAccess<'_> {
    fn span(&self) -> Span {
        self.object.leftmost_span().join(self.property.span())
    }
}

impl HasSpan for NullSafePropertyAccess<'_> {
    fn span(&self) -> Span {
        self.object.leftmost_span().join(self.property.span())
    }
}

impl HasSpan for StaticPropertyAccess<'_> {
    fn span(&self) -> Span {
        self.class.leftmost_span().join(self.property.span())
    }
}

impl HasSpan for ClassConstantAccess<'_> {
    fn span(&self) -> Span {
        self.class.leftmost_span().join(self.constant.span())
    }
}
