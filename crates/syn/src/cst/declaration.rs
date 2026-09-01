//! Declarations: namespaces, use statements, constants, and attributes.

use whim_span::HasSpan;
use whim_span::Position;
use whim_span::Span;

use crate::cst::atom::Identifier;
use crate::cst::atom::Keyword;
use crate::cst::atom::LocalIdentifier;
use crate::cst::call::ArgumentList;
use crate::cst::expression::Expression;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::statement::Block;
use crate::cst::statement::Statement;

/// A namespace declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Namespace<'arena> {
    pub namespace: Keyword<'arena>,
    pub name: Identifier<'arena>,
    pub body: NamespaceBody<'arena>,
}

/// The body of a namespace declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum NamespaceBody<'arena> {
    Implicit(NamespaceImplicitBody<'arena>),
    BraceDelimited(Block<'arena>),
}

/// An implicit namespace body: a semicolon followed by every statement up
/// to the end of the file (or the next namespace declaration).
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NamespaceImplicitBody<'arena> {
    pub semicolon: Span,
    pub statements: &'arena [Statement<'arena>],
}

impl<'arena> Namespace<'arena> {
    #[inline]
    #[must_use]
    pub const fn statements(&self) -> &'arena [Statement<'arena>] {
        match &self.body {
            NamespaceBody::Implicit(body) => body.statements,
            NamespaceBody::BraceDelimited(body) => body.statements,
        }
    }
}

impl HasSpan for Namespace<'_> {
    fn span(&self) -> Span {
        self.namespace.span().join(self.body.span())
    }
}

impl HasSpan for NamespaceBody<'_> {
    fn span(&self) -> Span {
        match self {
            NamespaceBody::Implicit(body) => body.span(),
            NamespaceBody::BraceDelimited(body) => body.span(),
        }
    }
}

impl HasSpan for NamespaceImplicitBody<'_> {
    fn span(&self) -> Span {
        self.statements
            .last()
            .map_or(self.semicolon, |last| self.semicolon.join(last.span()))
    }
}

/// A use statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Use<'arena> {
    pub r#use: Keyword<'arena>,
    pub items: UseItems<'arena>,
    pub semicolon: Span,
}

/// The items of a use statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum UseItems<'arena> {
    Sequence(UseItemSequence<'arena>),
    List(UseItemList<'arena>),
}

/// One or more use items, without a shared namespace prefix.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UseItemSequence<'arena> {
    pub items: TokenSeparatedSequence<'arena, UseItem<'arena>>,
}

/// A brace-delimited list of use items sharing a namespace prefix.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UseItemList<'arena> {
    pub namespace: Identifier<'arena>,
    pub namespace_separator: Span,
    pub left_brace: Span,
    pub items: TokenSeparatedSequence<'arena, UseItem<'arena>>,
    pub right_brace: Span,
}

/// A single imported name, with an optional alias.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UseItem<'arena> {
    pub name: Identifier<'arena>,
    pub alias: Option<UseItemAlias<'arena>>,
}

/// The alias of a use item: the `as Postman` in `use Mailer as Postman;`.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UseItemAlias<'arena> {
    pub r#as: Keyword<'arena>,
    pub identifier: LocalIdentifier<'arena>,
}

impl HasSpan for Use<'_> {
    fn span(&self) -> Span {
        self.r#use.span().join(self.semicolon)
    }
}

impl HasSpan for UseItems<'_> {
    fn span(&self) -> Span {
        match self {
            UseItems::Sequence(items) => items.span(),
            UseItems::List(items) => items.span(),
        }
    }
}

impl HasSpan for UseItemSequence<'_> {
    fn span(&self) -> Span {
        self.items.span(Position::zero())
    }
}

impl HasSpan for UseItemList<'_> {
    fn span(&self) -> Span {
        self.namespace.span().join(self.right_brace)
    }
}

impl HasSpan for UseItem<'_> {
    fn span(&self) -> Span {
        self.alias.as_ref().map_or_else(
            || self.name.span(),
            |alias| self.name.span().join(alias.span()),
        )
    }
}

impl HasSpan for UseItemAlias<'_> {
    fn span(&self) -> Span {
        self.r#as.span().join(self.identifier.span())
    }
}

/// A top-level constant declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Constant<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub r#const: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub equals: Span,
    pub value: &'arena Expression<'arena>,
    pub semicolon: Span,
}

impl HasSpan for Constant<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.semicolon);
        }

        self.r#const.span().join(self.semicolon)
    }
}

/// A list of attributes.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct AttributeList<'arena> {
    pub hash_left_bracket: Span,
    pub attributes: TokenSeparatedSequence<'arena, Attribute<'arena>>,
    pub right_bracket: Span,
}

/// A single attribute.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Attribute<'arena> {
    pub name: Identifier<'arena>,
    pub argument_list: Option<ArgumentList<'arena>>,
}

impl HasSpan for AttributeList<'_> {
    fn span(&self) -> Span {
        self.hash_left_bracket.join(self.right_bracket)
    }
}

impl HasSpan for Attribute<'_> {
    fn span(&self) -> Span {
        self.argument_list.as_ref().map_or_else(
            || self.name.span(),
            |arguments| self.name.span().join(arguments.span()),
        )
    }
}
