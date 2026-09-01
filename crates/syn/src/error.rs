use std::error;
use std::fmt;

use whim_span::HasSpan;
use whim_span::Position;
use whim_span::Span;

use crate::parser;
use crate::token::kind::TokenKind;

/// The token kind(s) a parser expected at the point an error was raised.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Expected {
    Exactly(TokenKind),
    OneOf(&'static [TokenKind]),
    Description(&'static str),
}

impl Expected {
    #[inline]
    #[must_use]
    pub const fn kind(kind: TokenKind) -> Self {
        Self::Exactly(kind)
    }
}

/// A fatal error produced while parsing Whim source code.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ParseError {
    /// The lexer failed before the parser could make progress.
    SyntaxError(SyntaxError),
    UnexpectedToken(Expected, TokenKind, Span),
    /// A quoted string contains an escape sequence that cannot denote bytes.
    InvalidStringLiteral(StringLiteralError, Span),
    UnexpectedEndOfFile(Expected, Position),
    InvalidAssignmentTarget(Span),
    InvalidBindTarget(Span),
    /// `_` is reserved for wildcard type and pattern positions and cannot be
    /// used as the name of a declaration or member.
    ReservedIdentifier(Span),
    RecursionLimitExceeded(Span),
    StructuralDepthExceeded(Span),
}

impl ParseError {
    #[inline]
    #[must_use]
    pub const fn position(&self) -> Position {
        match self {
            Self::SyntaxError(error) => error.start(),
            Self::UnexpectedToken(_, _, span)
            | Self::InvalidStringLiteral(_, span)
            | Self::InvalidAssignmentTarget(span)
            | Self::InvalidBindTarget(span)
            | Self::ReservedIdentifier(span)
            | Self::RecursionLimitExceeded(span)
            | Self::StructuralDepthExceeded(span) => span.start,
            Self::UnexpectedEndOfFile(_, position) => *position,
        }
    }
}

impl From<SyntaxError> for ParseError {
    #[inline]
    fn from(error: SyntaxError) -> Self {
        Self::SyntaxError(error)
    }
}

impl HasSpan for ParseError {
    #[inline]
    fn span(&self) -> Span {
        match self {
            Self::SyntaxError(error) => error.span(),
            Self::UnexpectedToken(_, _, span)
            | Self::InvalidStringLiteral(_, span)
            | Self::InvalidAssignmentTarget(span)
            | Self::InvalidBindTarget(span)
            | Self::ReservedIdentifier(span)
            | Self::RecursionLimitExceeded(span)
            | Self::StructuralDepthExceeded(span) => *span,
            Self::UnexpectedEndOfFile(_, position) => Span::new(*position, *position),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SyntaxError(error) => error.fmt(f),
            Self::UnexpectedToken(expected, found, _) => {
                write!(f, "unexpected {}; expected {expected}", found.describe())
            }
            Self::InvalidStringLiteral(error, _) => error.fmt(f),
            Self::UnexpectedEndOfFile(expected, _) => {
                write!(f, "unexpected end of file; expected {expected}")
            }
            Self::InvalidAssignmentTarget(_) => write!(
                f,
                "an assignment target must be a variable, property access, index access, append, \
                 or destructuring pattern"
            ),
            Self::InvalidBindTarget(_) => write!(
                f,
                "a bind target must be a variable or a nested tuple or dictionary binding pattern"
            ),
            Self::ReservedIdentifier(_) => {
                write!(f, "`_` is reserved and cannot be used as an identifier")
            }
            Self::RecursionLimitExceeded(_) => write!(f, "maximum nesting depth exceeded"),
            Self::StructuralDepthExceeded(_) => write!(
                f,
                "a program's syntax tree may be at most {} levels deep; the deepest \
                 path ends here",
                parser::MAX_STRUCTURAL_DEPTH
            ),
        }
    }
}

/// Why a quoted string literal could not be decoded.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum StringLiteralError {
    MalformedUnicodeEscape,
    UnicodeSurrogate,
    UnicodeOutOfRange,
    OctalEscapeOutOfRange,
    MalformedLiteral,
    UnescapedClosingBrace,
}

impl fmt::Display for StringLiteralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedUnicodeEscape => f.write_str(
                "malformed Unicode escape; expected `\\u{` followed by hexadecimal digits and `}`",
            ),
            Self::UnicodeSurrogate => {
                f.write_str("a Unicode escape cannot name a surrogate code point")
            }
            Self::UnicodeOutOfRange => {
                f.write_str("the Unicode escape exceeds the maximum scalar value `10FFFF`")
            }
            Self::OctalEscapeOutOfRange => {
                f.write_str("the octal escape exceeds `\\377`, the largest one-byte value")
            }
            Self::MalformedLiteral => f.write_str("malformed string literal"),
            Self::UnescapedClosingBrace => {
                f.write_str("a literal `}` in a double-quoted string must be escaped as `\\}`")
            }
        }
    }
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exactly(kind) => f.write_str(kind.describe()),
            Self::Description(description) => f.write_str(description),
            Self::OneOf(kinds) => {
                for (index, kind) in kinds.iter().enumerate() {
                    match index {
                        0 => f.write_str(kind.describe())?,
                        _ if index + 1 == kinds.len() => write!(f, ", or {}", kind.describe())?,
                        _ => write!(f, ", {}", kind.describe())?,
                    }
                }

                Ok(())
            }
        }
    }
}

impl error::Error for ParseError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::SyntaxError(error) => Some(error),
            _ => None,
        }
    }
}

/// A fatal error produced while lexing Whim source code.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum SyntaxError {
    /// A byte that cannot begin any token.
    UnrecognizedByte(u8, Position),
    /// A string literal that reached the end of the file before its closing quote.
    UnclosedStringLiteral(Position),
    /// A `/* ... */` comment that reached the end of the file before `*/`.
    UnclosedBlockComment(Position),
    LeadingZeroInIntegerLiteral(Position),
    /// A base-prefixed integer literal (`0x`, `0o`, `0b`) with no valid digit
    /// after the prefix, e.g. `0x`, `0b`, `0o8`, `0xG`. The position is that of
    /// the leading `0`.
    MalformedNumericLiteral(Position),
}

impl SyntaxError {
    /// The position at which the error occurred.
    #[inline]
    #[must_use]
    pub const fn start(&self) -> Position {
        match self {
            Self::UnrecognizedByte(_, position)
            | Self::UnclosedStringLiteral(position)
            | Self::UnclosedBlockComment(position)
            | Self::LeadingZeroInIntegerLiteral(position)
            | Self::MalformedNumericLiteral(position) => *position,
        }
    }
}

impl HasSpan for SyntaxError {
    #[inline]
    fn span(&self) -> Span {
        let position = self.start();

        Span::new(position, position.forward(1))
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnrecognizedByte(byte, _) => {
                write!(f, "unrecognized byte `{}` (0x{byte:02X})", *byte as char)
            }
            Self::UnclosedStringLiteral(_) => write!(f, "unclosed string literal"),
            Self::UnclosedBlockComment(_) => write!(f, "unclosed block comment"),
            Self::LeadingZeroInIntegerLiteral(_) => write!(
                f,
                "decimal integer literals cannot have a leading zero; use `0o` for octal"
            ),
            Self::MalformedNumericLiteral(_) => write!(
                f,
                "malformed numeric literal; a `0x`, `0o`, or `0b` prefix must be \
                 followed by at least one digit of that base"
            ),
        }
    }
}

impl error::Error for SyntaxError {}
