use whim_span::HasSpan;
use whim_span::Span;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum TriviaKind {
    Whitespace,
    SingleLineComment,
    MultiLineComment,
    DocBlockComment,
    Shebang,
}

/// Whitespace, comments, and the shebang line: text with no meaning to the
/// parser, kept so a tool can rebuild the original source.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Trivia<'arena> {
    pub kind: TriviaKind,
    pub span: Span,
    pub value: &'arena str,
}

impl TriviaKind {
    #[inline]
    #[must_use]
    pub const fn is_comment(&self) -> bool {
        matches!(
            self,
            Self::SingleLineComment | Self::MultiLineComment | Self::DocBlockComment
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_whitespace(&self) -> bool {
        matches!(self, Self::Whitespace)
    }
}

impl HasSpan for Trivia<'_> {
    fn span(&self) -> Span {
        self.span
    }
}
