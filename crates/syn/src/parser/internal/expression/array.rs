//! Parses vec, dict, tuple, and parenthesized expressions.

use crate::arena::Arena;
use crate::arena::Vec;
use whim_span::Span;

use crate::cst::array::DictEntry;
use crate::cst::array::DictExpression;
use crate::cst::array::DictPair;
use crate::cst::array::DictSpread;
use crate::cst::array::TupleElement;
use crate::cst::array::TupleExpression;
use crate::cst::array::TupleRest;
use crate::cst::array::VecElement;
use crate::cst::array::VecExpression;
use crate::cst::array::VecFillExpression;
use crate::cst::expression::Expression;
use crate::cst::expression::Parenthesized;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::kind::TokenKind;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    /// Parses a vector literal `vec[e1, ...e2, e3]` or fill expression
    /// `vec[value; size]`.
    pub(in crate::parser::internal::expression) fn parse_vec_expression(
        &mut self,
    ) -> Result<Expression<'arena>, ParseError> {
        let vec = self.expect_keyword(TokenKind::Vec)?;
        let left_bracket = self.expect_span(TokenKind::LeftBracket)?;

        let mut elements = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        if self.is_at(TokenKind::RightBracket)? {
            let right_bracket = self.expect_span(TokenKind::RightBracket)?;

            return Ok(Expression::Vec(VecExpression {
                vec,
                left_bracket,
                elements: TokenSeparatedSequence::new(elements, commas),
                right_bracket,
            }));
        }

        let ellipsis = self.eat_optional(TokenKind::DotDotDot)?;
        let value = self.parse_expression_ref()?;
        if ellipsis.is_none() && self.is_at(TokenKind::Semicolon)? {
            let semicolon = self.expect_span(TokenKind::Semicolon)?;
            let size = self.parse_expression_ref()?;
            let right_bracket = self.expect_span(TokenKind::RightBracket)?;

            return Ok(Expression::VecFill(VecFillExpression {
                vec,
                left_bracket,
                value,
                semicolon,
                size,
                right_bracket,
            }));
        }
        elements.push(VecElement { ellipsis, value });

        while !self.is_at(TokenKind::RightBracket)? {
            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
                if self.is_at(TokenKind::RightBracket)? {
                    break;
                }
            } else {
                break;
            }

            let ellipsis = self.eat_optional(TokenKind::DotDotDot)?;
            let value = self.parse_expression_ref()?;
            elements.push(VecElement { ellipsis, value });
        }

        let right_bracket = self.expect_span(TokenKind::RightBracket)?;

        Ok(Expression::Vec(VecExpression {
            vec,
            left_bracket,
            elements: TokenSeparatedSequence::new(elements, commas),
            right_bracket,
        }))
    }

    /// Parses a dictionary literal `dict[key => value, ...other]`.
    pub(in crate::parser::internal::expression) fn parse_dict_expression(
        &mut self,
    ) -> Result<DictExpression<'arena>, ParseError> {
        let dict = self.expect_keyword(TokenKind::Dict)?;
        let left_bracket = self.expect_span(TokenKind::LeftBracket)?;

        let mut entries = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        while !self.is_at(TokenKind::RightBracket)? {
            if let Some(ellipsis) = self.eat_optional(TokenKind::DotDotDot)? {
                let value = self.parse_expression_ref()?;
                entries.push(DictEntry::Spread(DictSpread { ellipsis, value }));
            } else {
                let key = self.parse_expression_ref()?;
                let double_arrow = self.expect_span(TokenKind::EqualGreaterThan)?;
                let value = self.parse_expression_ref()?;
                entries.push(DictEntry::Pair(DictPair {
                    key,
                    double_arrow,
                    value,
                }));
            }

            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
            } else {
                break;
            }
        }

        let right_bracket = self.expect_span(TokenKind::RightBracket)?;

        Ok(DictExpression {
            dict,
            left_bracket,
            entries: TokenSeparatedSequence::new(entries, commas),
            right_bracket,
        })
    }

    /// Parses a `(...)` expression: a parenthesized expression `(e)`, or a
    /// tuple `(e,)` / `(e1, e2)`. A comma after the first element makes it
    /// a tuple; a single element needs a trailing comma. `()` is an error.
    pub(in crate::parser::internal::expression) fn parse_parenthesized_or_tuple_expression(
        &mut self,
    ) -> Result<Expression<'arena>, ParseError> {
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        if self.is_at(TokenKind::RightParenthesis)? {
            return Err(self.unexpected(Expected::Description(
                "an expression ( `()` is not valid; use a tuple element or `null` )",
            )));
        }

        let first = self.parse_expression_ref()?;

        self.finish_parenthesized_or_tuple(left_parenthesis, first)
    }

    fn finish_parenthesized_or_tuple(
        &mut self,
        left_parenthesis: Span,
        first: &'arena Expression<'arena>,
    ) -> Result<Expression<'arena>, ParseError> {
        if self.is_at(TokenKind::RightParenthesis)? {
            let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

            return Ok(Expression::Parenthesized(Parenthesized {
                left_parenthesis,
                expression: first,
                right_parenthesis,
            }));
        }

        let mut elements = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        elements.push(TupleElement::Value(first));
        while self.is_at(TokenKind::Comma)? {
            commas.push(self.consume()?);

            if self.is_at(TokenKind::RightParenthesis)? {
                break;
            }

            if let Some(ellipsis) = self.eat_optional(TokenKind::DotDotDot)? {
                let value =
                    if self.is_at(TokenKind::Comma)? || self.is_at(TokenKind::RightParenthesis)? {
                        None
                    } else {
                        Some(self.parse_expression_ref()?)
                    };
                elements.push(TupleElement::Rest(TupleRest { ellipsis, value }));
                continue;
            }

            let value = self.parse_expression_ref()?;
            elements.push(TupleElement::Value(value));
        }

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(Expression::Tuple(TupleExpression {
            left_parenthesis,
            elements: TokenSeparatedSequence::new(elements, commas),
            right_parenthesis,
        }))
    }
}
