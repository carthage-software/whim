//! Token kinds and operator precedence for the Whim language.

pub mod kind;
pub mod precedence;

use crate::token::kind::TokenKind;
use whim_span::HasPosition;
use whim_span::Position;
use whim_span::Span;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Token<'input> {
    pub kind: TokenKind,
    pub start: Position,
    pub value: &'input str,
}

impl HasPosition for Token<'_> {
    #[inline]
    fn position(&self) -> Position {
        self.start
    }
}

impl<'arena> Token<'arena> {
    #[inline]
    #[must_use]
    pub const fn new(kind: TokenKind, value: &'arena str, start: Position) -> Self {
        Self { kind, start, value }
    }

    #[inline]
    #[must_use]
    pub const fn compute_span(&self) -> Span {
        let end = Position::new(self.start.offset + self.value.len() as u32);

        Span::new(self.start, end)
    }
}

#[cfg(test)]
mod tests {
    use crate::token::kind::TokenKind;
    use crate::token::precedence::Precedence;

    #[test]
    fn precedence_orders_loosest_to_tightest() {
        assert!(Precedence::Assignment > Precedence::Lowest);
        assert!(Precedence::Coalesce > Precedence::Assignment);
        assert!(Precedence::Or > Precedence::Coalesce);
        assert!(Precedence::And > Precedence::Or);
        assert!(Precedence::Comparison > Precedence::And);
        assert!(Precedence::TypeOperation > Precedence::Comparison);
        assert!(Precedence::Concat > Precedence::TypeOperation);
        assert!(Precedence::BitwiseOr > Precedence::Concat);
        assert!(Precedence::BitwiseXor > Precedence::BitwiseOr);
        assert!(Precedence::BitwiseAnd > Precedence::BitwiseXor);
        assert!(Precedence::Shift > Precedence::BitwiseAnd);
        assert!(Precedence::Additive > Precedence::Shift);
        assert!(Precedence::Multiplicative > Precedence::Additive);
        assert!(Precedence::Unary > Precedence::Multiplicative);
        assert!(Precedence::Exponent > Precedence::Unary);
        assert!(Precedence::Postfix > Precedence::Exponent);
        assert!(Precedence::Highest > Precedence::Postfix);
    }

    #[test]
    fn bitwise_binds_tighter_than_comparison() {
        assert!(
            Precedence::infix(&TokenKind::Ampersand) > Precedence::infix(&TokenKind::EqualEqual)
        );
    }

    #[test]
    fn exponent_binds_tighter_than_unary() {
        assert!(Precedence::Exponent > Precedence::prefix(&TokenKind::Minus));
    }

    #[test]
    fn comparison_is_one_non_associative_level() {
        assert_eq!(
            Precedence::infix(&TokenKind::LessThan),
            Precedence::infix(&TokenKind::EqualEqual)
        );
        assert!(Precedence::Comparison.is_non_associative());
    }

    #[test]
    fn type_operations_share_one_level() {
        assert_eq!(Precedence::infix(&TokenKind::Is), Precedence::TypeOperation);
        assert_eq!(Precedence::infix(&TokenKind::As), Precedence::TypeOperation);
        assert_eq!(
            Precedence::infix(&TokenKind::Question),
            Precedence::TypeOperation
        );
        assert!(Precedence::TypeOperation.is_non_associative());
    }

    #[test]
    fn every_infix_level_has_associativity() {
        let infix_kinds = [
            TokenKind::Equal,
            TokenKind::QuestionQuestion,
            TokenKind::PipePipe,
            TokenKind::AmpersandAmpersand,
            TokenKind::EqualEqual,
            TokenKind::Is,
            TokenKind::PipeGreaterThan,
            TokenKind::Dot,
            TokenKind::Pipe,
            TokenKind::Caret,
            TokenKind::Ampersand,
            TokenKind::LeftShift,
            TokenKind::Plus,
            TokenKind::Asterisk,
            TokenKind::AsteriskAsterisk,
        ];

        for kind in infix_kinds {
            assert!(Precedence::infix(&kind).is_associative(), "{kind:?}");
        }
    }
}
