//! Operator precedence levels and their associativity.

use crate::token::kind::TokenKind;

/// The associativity of a precedence level.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Associativity {
    /// Chaining or mixing operators of this level requires parentheses.
    NonAssociative,
    Left,
    Right,
}

/// Operator precedence levels, declared from loosest to tightest.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Precedence {
    Lowest,
    Assignment,
    Coalesce,
    Or,
    And,
    /// `==`, `!=`, `<`, `<=`, `>`, `>=`, `<=>` (non-associative; chaining or
    /// mixing comparisons requires parentheses).
    Comparison,
    TypeOperation,
    Pipe,
    Concat,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    Shift,
    Additive,
    Multiplicative,
    Unary,
    Exponent,
    Postfix,
    Highest,
}

pub trait GetPrecedence {
    fn precedence(&self) -> Precedence;
}

impl Precedence {
    /// The precedence of `kind` when it appears in infix position, or
    /// [`Precedence::Lowest`] if it is not an infix operator.
    #[inline]
    #[must_use]
    pub const fn infix(kind: &TokenKind) -> Self {
        match kind {
            TokenKind::Equal
            | TokenKind::PlusEqual
            | TokenKind::MinusEqual
            | TokenKind::AsteriskEqual
            | TokenKind::SlashEqual
            | TokenKind::PercentEqual
            | TokenKind::AsteriskAsteriskEqual
            | TokenKind::DotEqual
            | TokenKind::AmpersandEqual
            | TokenKind::PipeEqual
            | TokenKind::CaretEqual
            | TokenKind::LeftShiftEqual
            | TokenKind::RightShiftEqual
            | TokenKind::QuestionQuestionEqual
            | TokenKind::AmpersandAmpersandEqual
            | TokenKind::PipePipeEqual => Self::Assignment,
            TokenKind::QuestionQuestion => Self::Coalesce,
            TokenKind::PipePipe => Self::Or,
            TokenKind::AmpersandAmpersand => Self::And,
            TokenKind::EqualEqual
            | TokenKind::BangEqual
            | TokenKind::LessThan
            | TokenKind::LessThanEqual
            | TokenKind::GreaterThan
            | TokenKind::GreaterThanEqual
            | TokenKind::LessThanEqualGreaterThan => Self::Comparison,
            TokenKind::Is | TokenKind::As | TokenKind::Question => Self::TypeOperation,
            TokenKind::PipeGreaterThan => Self::Pipe,
            TokenKind::Dot => Self::Concat,
            TokenKind::Pipe => Self::BitwiseOr,
            TokenKind::Caret => Self::BitwiseXor,
            TokenKind::Ampersand => Self::BitwiseAnd,
            TokenKind::LeftShift | TokenKind::RightShift => Self::Shift,
            TokenKind::Plus | TokenKind::Minus => Self::Additive,
            TokenKind::Asterisk | TokenKind::Slash | TokenKind::Percent => Self::Multiplicative,
            TokenKind::AsteriskAsterisk => Self::Exponent,
            _ => Self::Lowest,
        }
    }

    /// The precedence of `kind` when it appears in prefix position, or
    /// [`Precedence::Lowest`] if it is not a prefix operator.
    #[inline]
    #[must_use]
    pub const fn prefix(kind: &TokenKind) -> Self {
        match kind {
            TokenKind::Bang
            | TokenKind::Tilde
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::PlusPlus
            | TokenKind::MinusMinus => Self::Unary,
            _ => Self::Lowest,
        }
    }

    /// The precedence of `kind` when it appears in postfix position, or
    /// [`Precedence::Lowest`] if it is not a postfix operator.
    #[inline]
    #[must_use]
    pub const fn postfix(kind: &TokenKind) -> Self {
        match kind {
            TokenKind::PlusPlus
            | TokenKind::MinusMinus
            | TokenKind::LeftParenthesis
            | TokenKind::LeftBracket
            | TokenKind::MinusGreaterThan
            | TokenKind::QuestionMinusGreaterThan
            | TokenKind::ColonColon
            | TokenKind::ColonColonLessThan => Self::Postfix,
            _ => Self::Lowest,
        }
    }

    #[inline]
    #[must_use]
    pub const fn associativity(&self) -> Option<Associativity> {
        Some(match self {
            Self::Or
            | Self::And
            | Self::Pipe
            | Self::Concat
            | Self::BitwiseOr
            | Self::BitwiseXor
            | Self::BitwiseAnd
            | Self::Shift
            | Self::Additive
            | Self::Multiplicative
            | Self::Postfix => Associativity::Left,
            Self::Assignment | Self::Coalesce | Self::Unary | Self::Exponent => {
                Associativity::Right
            }
            Self::Comparison | Self::TypeOperation => Associativity::NonAssociative,
            Self::Lowest | Self::Highest => return None,
        })
    }

    #[inline]
    #[must_use]
    pub const fn is_associative(&self) -> bool {
        self.associativity().is_some()
    }

    #[inline]
    #[must_use]
    pub const fn is_non_associative(&self) -> bool {
        matches!(self.associativity(), Some(Associativity::NonAssociative))
    }
}
