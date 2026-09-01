//! Class-like declarations: classes, interfaces, enums, and their members.

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Identifier;
use crate::cst::atom::Keyword;
use crate::cst::atom::LocalIdentifier;
use crate::cst::atom::Modifier;
use crate::cst::atom::Variable;
use crate::cst::declaration::AttributeList;
use crate::cst::expression::Expression;
use crate::cst::function::ParameterList;
use crate::cst::function::ReturnType;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::statement::Block;
use crate::cst::r#type::NamedType;
use crate::cst::r#type::Type;
use crate::cst::r#type::TypeParameterList;

/// A class declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Class<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub modifiers: &'arena [Modifier<'arena>],
    pub class: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub extends: Option<Extends<'arena>>,
    pub implements: Option<Implements<'arena>>,
    pub permissions: Option<SealedPermissions<'arena>>,
    pub left_brace: Span,
    pub members: &'arena [ClassLikeMember<'arena>],
    pub right_brace: Span,
}

/// An interface declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Interface<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub interface: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub extends: Option<Extends<'arena>>,
    pub permissions: Option<SealedPermissions<'arena>>,
    pub left_brace: Span,
    pub members: &'arena [ClassLikeMember<'arena>],
    pub right_brace: Span,
}

/// An enum declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Enum<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub r#enum: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub backing_type: Option<EnumBackingType<'arena>>,
    pub implements: Option<Implements<'arena>>,
    pub left_brace: Span,
    pub members: &'arena [ClassLikeMember<'arena>],
    pub right_brace: Span,
}

/// The backing type of an enum: the `: string` in `enum Suit: string`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct EnumBackingType<'arena> {
    pub colon: Span,
    pub r#type: &'arena Type<'arena>,
}

impl Class<'_> {
    #[inline]
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        self.modifiers.iter().any(Modifier::is_abstract)
    }

    #[inline]
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.modifiers.iter().any(Modifier::is_final)
    }

    #[inline]
    #[must_use]
    pub fn is_readonly(&self) -> bool {
        self.modifiers.iter().any(Modifier::is_readonly)
    }
}

impl Enum<'_> {
    #[inline]
    #[must_use]
    pub const fn is_backed(&self) -> bool {
        self.backing_type.is_some()
    }
}

impl HasSpan for Class<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.right_brace);
        }

        if let Some(modifier) = self.modifiers.first() {
            return modifier.span().join(self.right_brace);
        }

        self.class.span().join(self.right_brace)
    }
}

impl HasSpan for Interface<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.right_brace);
        }

        self.interface.span().join(self.right_brace)
    }
}

impl HasSpan for Enum<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.right_brace);
        }

        self.r#enum.span().join(self.right_brace)
    }
}

impl HasSpan for EnumBackingType<'_> {
    fn span(&self) -> Span {
        self.colon.join(self.r#type.span())
    }
}

/// A member of a class-like declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ClassLikeMember<'arena> {
    Constant(ClassLikeConstant<'arena>),
    EnumCase(EnumCase<'arena>),
    Method(Method<'arena>),
    Property(Property<'arena>),
}

impl ClassLikeMember<'_> {
    #[inline]
    #[must_use]
    pub const fn is_constant(&self) -> bool {
        matches!(self, ClassLikeMember::Constant(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_property(&self) -> bool {
        matches!(self, ClassLikeMember::Property(_))
    }
}

impl HasSpan for ClassLikeMember<'_> {
    fn span(&self) -> Span {
        match self {
            ClassLikeMember::Constant(member) => member.span(),
            ClassLikeMember::EnumCase(member) => member.span(),
            ClassLikeMember::Method(member) => member.span(),
            ClassLikeMember::Property(member) => member.span(),
        }
    }
}

/// A class-like constant declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ClassLikeConstant<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub modifiers: &'arena [Modifier<'arena>],
    pub r#const: Keyword<'arena>,
    pub r#type: Option<&'arena Type<'arena>>,
    pub name: LocalIdentifier<'arena>,
    pub equals: Span,
    pub value: &'arena Expression<'arena>,
    pub semicolon: Span,
}

impl ClassLikeConstant<'_> {
    #[inline]
    #[must_use]
    pub const fn is_typed(&self) -> bool {
        self.r#type.is_some()
    }
}

impl HasSpan for ClassLikeConstant<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.semicolon);
        }

        if let Some(modifier) = self.modifiers.first() {
            return modifier.span().join(self.semicolon);
        }

        self.r#const.span().join(self.semicolon)
    }
}

/// An enum case declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct EnumCase<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub case: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub value: Option<EnumCaseValue<'arena>>,
    pub semicolon: Span,
}

/// The backing value of an enum case: the `= 'H'` in `case Hearts = 'H';`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct EnumCaseValue<'arena> {
    pub equals: Span,
    pub expression: &'arena Expression<'arena>,
}

impl EnumCase<'_> {
    #[inline]
    #[must_use]
    pub const fn is_backed(&self) -> bool {
        self.value.is_some()
    }
}

impl HasSpan for EnumCase<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.semicolon);
        }

        self.case.span().join(self.semicolon)
    }
}

impl HasSpan for EnumCaseValue<'_> {
    fn span(&self) -> Span {
        self.equals.join(self.expression.span())
    }
}

/// The `extends` clause of a class or interface, with one or more types.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Extends<'arena> {
    pub extends: Keyword<'arena>,
    pub types: TokenSeparatedSequence<'arena, NamedType<'arena>>,
}

/// The direct subtypes permitted by a sealed class or interface: `for A, B`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct SealedPermissions<'arena> {
    pub r#for: Keyword<'arena>,
    pub types: TokenSeparatedSequence<'arena, Identifier<'arena>>,
}

/// The `implements` clause of a class or enum, with one or more types.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Implements<'arena> {
    pub implements: Keyword<'arena>,
    pub types: TokenSeparatedSequence<'arena, NamedType<'arena>>,
}

impl HasSpan for Extends<'_> {
    fn span(&self) -> Span {
        let keyword_span = self.extends.span();

        keyword_span.join(self.types.span(keyword_span.end))
    }
}

impl HasSpan for SealedPermissions<'_> {
    fn span(&self) -> Span {
        let keyword_span = self.r#for.span();

        keyword_span.join(self.types.span(keyword_span.end))
    }
}

impl HasSpan for Implements<'_> {
    fn span(&self) -> Span {
        let keyword_span = self.implements.span();

        keyword_span.join(self.types.span(keyword_span.end))
    }
}

/// A method declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Method<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub modifiers: &'arena [Modifier<'arena>],
    pub function: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub parameter_list: ParameterList<'arena>,
    pub return_type: Option<ReturnType<'arena>>,
    pub body: MethodBody<'arena>,
}

/// The body of a method declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum MethodBody<'arena> {
    Abstract(Span),
    Concrete(Block<'arena>),
}

impl Method<'_> {
    #[inline]
    #[must_use]
    pub const fn is_abstract(&self) -> bool {
        matches!(self.body, MethodBody::Abstract(_))
    }

    #[inline]
    #[must_use]
    pub fn is_static(&self) -> bool {
        self.modifiers.iter().any(Modifier::is_static)
    }

    #[inline]
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.modifiers.iter().any(Modifier::is_final)
    }
}

impl HasSpan for Method<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.body.span());
        }

        if let Some(modifier) = self.modifiers.first() {
            return modifier.span().join(self.body.span());
        }

        self.function.span().join(self.body.span())
    }
}

impl HasSpan for MethodBody<'_> {
    fn span(&self) -> Span {
        match self {
            MethodBody::Abstract(semicolon) => *semicolon,
            MethodBody::Concrete(block) => block.span(),
        }
    }
}

/// A property declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Property<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub modifiers: &'arena [Modifier<'arena>],
    pub r#type: Option<&'arena Type<'arena>>,
    pub variable: Variable<'arena>,
    pub default: Option<PropertyDefault<'arena>>,
    pub semicolon: Span,
}

/// The default value of a property: the `= 42` in `public int $foo = 42;`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PropertyDefault<'arena> {
    pub equals: Span,
    pub value: &'arena Expression<'arena>,
}

impl Property<'_> {
    #[inline]
    #[must_use]
    pub const fn is_typed(&self) -> bool {
        self.r#type.is_some()
    }

    #[inline]
    #[must_use]
    pub const fn has_default(&self) -> bool {
        self.default.is_some()
    }

    #[inline]
    #[must_use]
    pub fn is_static(&self) -> bool {
        self.modifiers.iter().any(Modifier::is_static)
    }

    #[inline]
    #[must_use]
    pub fn is_readonly(&self) -> bool {
        self.modifiers.iter().any(Modifier::is_readonly)
    }
}

impl HasSpan for Property<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.semicolon);
        }

        if let Some(modifier) = self.modifiers.first() {
            return modifier.span().join(self.semicolon);
        }

        if let Some(r#type) = self.r#type {
            return r#type.span().join(self.semicolon);
        }

        self.variable.span().join(self.semicolon)
    }
}

impl HasSpan for PropertyDefault<'_> {
    fn span(&self) -> Span {
        self.equals.join(self.value.span())
    }
}
