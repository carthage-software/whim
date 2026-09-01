//! Atomic nodes: keywords, identifiers, variables, literals, and modifiers.

use std::cmp::Ordering;
use std::hash::Hash;
use std::hash::Hasher;

use whim_span::HasSpan;
use whim_span::Span;

/// A keyword as it appeared in the source, with its span.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Keyword<'arena> {
    pub span: Span,
    pub value: &'arena str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Identifier<'arena> {
    Local(LocalIdentifier<'arena>),
    Qualified(QualifiedIdentifier<'arena>),
    FullyQualified(FullyQualifiedIdentifier<'arena>),
}

/// A local, unqualified identifier.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct LocalIdentifier<'arena> {
    pub span: Span,
    pub value: &'arena str,
}

/// A qualified identifier.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct QualifiedIdentifier<'arena> {
    pub span: Span,
    pub value: &'arena str,
}

/// A fully qualified identifier.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FullyQualifiedIdentifier<'arena> {
    pub span: Span,
    pub value: &'arena str,
}

impl<'arena> Identifier<'arena> {
    #[inline]
    #[must_use]
    pub const fn value(&self) -> &'arena str {
        match &self {
            Identifier::Local(identifier) => identifier.value,
            Identifier::Qualified(identifier) => identifier.value,
            Identifier::FullyQualified(identifier) => identifier.value,
        }
    }

    /// The final segment of the identifier (`Bar` in `Foo\Bar`), or the
    /// whole value when unqualified.
    #[inline]
    #[must_use]
    pub fn last_segment(&self) -> &'arena str {
        let value = self.value();

        value
            .rfind('\\')
            .map_or(value, |position| &value[position + 1..])
    }
}

/// A variable: `$name`.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Variable<'arena> {
    pub span: Span,
    pub name: &'arena str,
}

/// A literal value.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Literal<'arena> {
    String(LiteralString<'arena>),
    Integer(LiteralInteger<'arena>),
    Float(LiteralFloat<'arena>),
    True(Keyword<'arena>),
    False(Keyword<'arena>),
    Null(Keyword<'arena>),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum LiteralStringKind {
    SingleQuoted,
    DoubleQuoted,
}

/// A string literal.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct LiteralString<'arena> {
    pub kind: LiteralStringKind,
    pub span: Span,
    pub raw: &'arena str,
    pub value: &'arena [u8],
}

/// An integer literal.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct LiteralInteger<'arena> {
    pub span: Span,
    pub raw: &'arena str,
    pub value: u64,
}

/// A float literal.
#[derive(Debug, Clone, Copy)]
pub struct LiteralFloat<'arena> {
    pub span: Span,
    pub raw: &'arena str,
    pub value: f64,
}

/// A modifier on a class-like declaration or member.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Modifier<'arena> {
    Public(Keyword<'arena>),
    Protected(Keyword<'arena>),
    Private(Keyword<'arena>),
    Static(Keyword<'arena>),
    Final(Keyword<'arena>),
    Abstract(Keyword<'arena>),
    Readonly(Keyword<'arena>),
}

impl<'arena> Modifier<'arena> {
    #[inline]
    #[must_use]
    pub const fn is_visibility(&self) -> bool {
        matches!(
            self,
            Modifier::Public(_) | Modifier::Protected(_) | Modifier::Private(_)
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_static(&self) -> bool {
        matches!(self, Modifier::Static(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_final(&self) -> bool {
        matches!(self, Modifier::Final(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_abstract(&self) -> bool {
        matches!(self, Modifier::Abstract(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_readonly(&self) -> bool {
        matches!(self, Modifier::Readonly(_))
    }

    #[inline]
    #[must_use]
    pub const fn keyword(&self) -> &Keyword<'arena> {
        match self {
            Modifier::Public(keyword)
            | Modifier::Protected(keyword)
            | Modifier::Private(keyword)
            | Modifier::Static(keyword)
            | Modifier::Final(keyword)
            | Modifier::Abstract(keyword)
            | Modifier::Readonly(keyword) => keyword,
        }
    }

    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Modifier::Public(_) => "public",
            Modifier::Protected(_) => "protected",
            Modifier::Private(_) => "private",
            Modifier::Static(_) => "static",
            Modifier::Final(_) => "final",
            Modifier::Abstract(_) => "abstract",
            Modifier::Readonly(_) => "readonly",
        }
    }
}

impl HasSpan for Keyword<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for Identifier<'_> {
    fn span(&self) -> Span {
        match self {
            Identifier::Local(identifier) => identifier.span(),
            Identifier::Qualified(identifier) => identifier.span(),
            Identifier::FullyQualified(identifier) => identifier.span(),
        }
    }
}

impl HasSpan for LocalIdentifier<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for QualifiedIdentifier<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for FullyQualifiedIdentifier<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for Variable<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for Modifier<'_> {
    fn span(&self) -> Span {
        self.keyword().span()
    }
}

impl HasSpan for Literal<'_> {
    fn span(&self) -> Span {
        match self {
            Literal::String(literal) => literal.span(),
            Literal::Integer(literal) => literal.span(),
            Literal::Float(literal) => literal.span(),
            Literal::True(keyword) | Literal::False(keyword) | Literal::Null(keyword) => {
                keyword.span()
            }
        }
    }
}

impl HasSpan for LiteralString<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for LiteralInteger<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for LiteralFloat<'_> {
    fn span(&self) -> Span {
        self.span
    }
}

impl PartialEq for LiteralFloat<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.span == other.span
            && self.raw == other.raw
            && self.value.to_bits() == other.value.to_bits()
    }
}

impl Eq for LiteralFloat<'_> {}

impl Hash for LiteralFloat<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.span.hash(state);
        self.raw.hash(state);
        self.value.to_bits().hash(state);
    }
}

impl PartialOrd for LiteralFloat<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LiteralFloat<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.span, self.raw, self.value.to_bits()).cmp(&(
            other.span,
            other.raw,
            other.value.to_bits(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use whim_span::Position;
    use whim_span::Span;

    use crate::cst::atom::FullyQualifiedIdentifier;
    use crate::cst::atom::Identifier;
    use crate::cst::atom::LiteralFloat;

    fn identifier(value: &'static str) -> Identifier<'static> {
        Identifier::FullyQualified(FullyQualifiedIdentifier {
            span: Span::new(Position::zero(), Position::new(value.len() as u32)),
            value,
        })
    }

    #[test]
    fn last_segment_returns_the_final_namespace_segment() {
        assert_eq!(
            identifier("\\App\\Service\\Mailer").last_segment(),
            "Mailer"
        );
        assert_eq!(identifier("Foo\\Bar").last_segment(), "Bar");
        assert_eq!(identifier("\\Foo").last_segment(), "Foo");
    }

    #[test]
    fn last_segment_returns_the_whole_value_when_unqualified() {
        assert_eq!(identifier("Mailer").last_segment(), "Mailer");
    }

    #[test]
    fn float_equality_is_reflexive_for_nan() {
        let literal = LiteralFloat {
            span: Span::new(Position::zero(), Position::new(3)),
            raw: "NAN",
            value: f64::NAN,
        };

        assert_eq!(literal, literal);
    }
}
