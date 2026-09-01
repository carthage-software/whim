//! Control-flow parsers: `if`, `match`, loops, `try`, and jumps.

use std::mem;

use crate::arena::Arena;
use crate::arena::Vec;

use crate::cst::atom::Literal;
use crate::cst::atom::Variable;
use crate::cst::control_flow::DoWhile;
use crate::cst::control_flow::Else;
use crate::cst::control_flow::ElseBody;
use crate::cst::control_flow::For;
use crate::cst::control_flow::Foreach;
use crate::cst::control_flow::ForeachKeyValueTarget;
use crate::cst::control_flow::ForeachTarget;
use crate::cst::control_flow::ForeachValueTarget;
use crate::cst::control_flow::If;
use crate::cst::control_flow::Match;
use crate::cst::control_flow::MatchArm;
use crate::cst::control_flow::Try;
use crate::cst::control_flow::TryCatchClause;
use crate::cst::control_flow::TryCatchGuard;
use crate::cst::control_flow::TryElseClause;
use crate::cst::control_flow::TryFinallyClause;
use crate::cst::control_flow::While;
use crate::cst::expression::Expression;
use crate::cst::pattern::AsPattern;
use crate::cst::pattern::DictPattern;
use crate::cst::pattern::DictPatternEntry;
use crate::cst::pattern::DictPatternKey;
use crate::cst::pattern::ParenthesizedPattern;
use crate::cst::pattern::Pattern;
use crate::cst::pattern::TrailingPattern;
use crate::cst::pattern::TuplePattern;
use crate::cst::pattern::UnionPattern;
use crate::cst::pattern::VecPattern;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::kind::TokenKind;
use crate::token::precedence::Precedence;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_match(&mut self) -> Result<Expression<'arena>, ParseError> {
        let r#match = self.expect_keyword(TokenKind::Match)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        let expression = self.parse_expression_ref()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let left_brace = self.expect_span(TokenKind::LeftBrace)?;

        let arms =
            self.parse_comma_separated_until(TokenKind::RightBrace, Self::parse_match_arm)?;

        let right_brace = self.expect_span(TokenKind::RightBrace)?;

        Ok(Expression::Match(Match {
            r#match,
            left_parenthesis,
            expression,
            right_parenthesis,
            left_brace,
            arms,
            right_brace,
        }))
    }

    fn parse_match_arm(&mut self) -> Result<MatchArm<'arena>, ParseError> {
        let pattern = self.parse_match_pattern()?;
        let double_arrow = self.expect_span(TokenKind::EqualGreaterThan)?;
        let expression = self.parse_expression_ref()?;

        Ok(MatchArm {
            pattern,
            double_arrow,
            expression,
        })
    }

    fn parse_match_pattern(&mut self) -> Result<&'arena Pattern<'arena>, ParseError> {
        self.enter()?;
        let result = self.parse_union_pattern();
        self.leave();
        result
    }

    fn parse_union_pattern(&mut self) -> Result<&'arena Pattern<'arena>, ParseError> {
        let mut left = self.parse_as_pattern()?;
        while self.is_at(TokenKind::Pipe)? {
            let pipe = self.expect_span(TokenKind::Pipe)?;
            let right = self.parse_as_pattern()?;
            left = self
                .arena
                .alloc(Pattern::Union(UnionPattern { left, pipe, right }));
        }

        Ok(left)
    }

    fn parse_as_pattern(&mut self) -> Result<&'arena Pattern<'arena>, ParseError> {
        let left = self.parse_primary_pattern()?;
        let Some(at) = self.eat_optional(TokenKind::At)? else {
            return Ok(left);
        };
        let right = self.parse_union_pattern()?;

        Ok(self.arena.alloc(Pattern::As(AsPattern { left, at, right })))
    }

    fn parse_primary_pattern(&mut self) -> Result<&'arena Pattern<'arena>, ParseError> {
        if self.is_at(TokenKind::Variable)? {
            let variable = self.parse_variable()?;
            return Ok(self.arena.alloc(Pattern::Variable(variable)));
        }

        if self.is_at(TokenKind::LeftParenthesis)? {
            return self.parse_parenthesized_or_tuple_pattern();
        }

        if self.is_at(TokenKind::Vec)?
            && self
                .lookahead(1)?
                .is_some_and(|token| token.kind == TokenKind::LeftBracket)
        {
            return self.parse_vec_pattern();
        }

        if self.is_at(TokenKind::Dict)?
            && self
                .lookahead(1)?
                .is_some_and(|token| token.kind == TokenKind::LeftBracket)
        {
            return self.parse_dict_pattern();
        }

        let r#type = self.parse_intersection_type()?;
        Ok(self.arena.alloc(Pattern::Type(r#type.clone())))
    }

    fn parse_parenthesized_or_tuple_pattern(
        &mut self,
    ) -> Result<&'arena Pattern<'arena>, ParseError> {
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        if self.is_at(TokenKind::RightParenthesis)? {
            return Err(self.unexpected(Expected::Description("a match pattern")));
        }

        if self.is_at(TokenKind::DotDotDot)? {
            let trailing = self.parse_trailing_pattern(TokenKind::RightParenthesis)?;
            let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
            return Ok(self.arena.alloc(Pattern::Tuple(TuplePattern {
                left_parenthesis,
                elements: TokenSeparatedSequence::new(
                    Vec::new_in(self.arena),
                    Vec::new_in(self.arena),
                ),
                trailing: Some(trailing),
                right_parenthesis,
            })));
        }

        let first = self.parse_match_pattern()?;
        if self.is_at(TokenKind::RightParenthesis)? {
            let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
            return Ok(self
                .arena
                .alloc(Pattern::Parenthesized(ParenthesizedPattern {
                    left_parenthesis,
                    pattern: first,
                    right_parenthesis,
                })));
        }

        let mut elements = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        let mut trailing = None;
        elements.push(first.clone());
        while self.is_at(TokenKind::Comma)? {
            commas.push(self.consume()?);
            if self.is_at(TokenKind::RightParenthesis)? {
                break;
            }
            if self.is_at(TokenKind::DotDotDot)? {
                trailing = Some(self.parse_trailing_pattern(TokenKind::RightParenthesis)?);
                break;
            }
            elements.push(self.parse_match_pattern()?.clone());
        }
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(self.arena.alloc(Pattern::Tuple(TuplePattern {
            left_parenthesis,
            elements: TokenSeparatedSequence::new(elements, commas),
            trailing,
            right_parenthesis,
        })))
    }

    fn parse_vec_pattern(&mut self) -> Result<&'arena Pattern<'arena>, ParseError> {
        let vec = self.expect_keyword(TokenKind::Vec)?;
        let left_bracket = self.expect_span(TokenKind::LeftBracket)?;
        let mut elements = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        let mut trailing = None;
        while !self.is_at(TokenKind::RightBracket)? {
            if self.is_at(TokenKind::DotDotDot)? {
                trailing = Some(self.parse_trailing_pattern(TokenKind::RightBracket)?);
                if self.is_at(TokenKind::Comma)? {
                    commas.push(self.consume()?);
                }
                break;
            }
            elements.push(self.parse_match_pattern()?.clone());
            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
            } else {
                break;
            }
        }
        let right_bracket = self.expect_span(TokenKind::RightBracket)?;

        Ok(self.arena.alloc(Pattern::Vec(VecPattern {
            vec,
            left_bracket,
            elements: TokenSeparatedSequence::new(elements, commas),
            trailing,
            right_bracket,
        })))
    }

    fn parse_dict_pattern(&mut self) -> Result<&'arena Pattern<'arena>, ParseError> {
        let dict = self.expect_keyword(TokenKind::Dict)?;
        let left_bracket = self.expect_span(TokenKind::LeftBracket)?;
        let mut entries = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        let mut trailing = None;
        while !self.is_at(TokenKind::RightBracket)? {
            if self.is_at(TokenKind::DotDotDot)? {
                trailing = Some(self.parse_trailing_pattern(TokenKind::RightBracket)?);
                if self.is_at(TokenKind::Comma)? {
                    commas.push(self.consume()?);
                }
                break;
            }
            let key = if self.is_at(TokenKind::Minus)? {
                DictPatternKey::Integer {
                    minus: Some(self.expect_span(TokenKind::Minus)?),
                    literal: self.parse_integer_literal()?,
                }
            } else {
                let token = self.consume()?;
                if !matches!(
                    token.kind,
                    TokenKind::LiteralInteger | TokenKind::LiteralString
                ) {
                    return Err(ParseError::UnexpectedToken(
                        Expected::Description("a string or integer dictionary pattern key"),
                        token.kind,
                        token.compute_span(),
                    ));
                }
                match self.literal_of(token)? {
                    Literal::Integer(literal) => DictPatternKey::Integer {
                        minus: None,
                        literal,
                    },
                    Literal::String(literal) => DictPatternKey::String(literal),
                    _ => unreachable!("the token kind permits only integer and string literals"),
                }
            };
            let double_arrow = self.expect_span(TokenKind::EqualGreaterThan)?;
            let pattern = self.parse_match_pattern()?;
            entries.push(DictPatternEntry {
                key,
                double_arrow,
                pattern,
            });
            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
            } else {
                break;
            }
        }
        let right_bracket = self.expect_span(TokenKind::RightBracket)?;

        Ok(self.arena.alloc(Pattern::Dict(DictPattern {
            dict,
            left_bracket,
            entries: TokenSeparatedSequence::new(entries, commas),
            trailing,
            right_bracket,
        })))
    }

    fn parse_trailing_pattern(
        &mut self,
        closing: TokenKind,
    ) -> Result<TrailingPattern<'arena>, ParseError> {
        let ellipsis = self.expect_span(TokenKind::DotDotDot)?;
        let pattern = if self.is_at(TokenKind::Comma)? || self.is_at(closing)? {
            None
        } else {
            Some(self.parse_match_pattern()?)
        };

        Ok(TrailingPattern { ellipsis, pattern })
    }

    pub(crate) fn parse_if(&mut self) -> Result<If<'arena>, ParseError> {
        let r#if = self.expect_keyword(TokenKind::If)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        let condition = self.parse_expression_ref()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let body = self.parse_block()?;

        let r#else = if self.is_at(TokenKind::Else)? {
            let keyword = self.expect_keyword(TokenKind::Else)?;
            let body = if self.is_at(TokenKind::If)? {
                let nested = self.parse_if()?;
                ElseBody::If(self.arena.alloc(nested))
            } else {
                ElseBody::Block(self.parse_block()?)
            };

            Some(Else {
                r#else: keyword,
                body,
            })
        } else {
            None
        };

        Ok(If {
            r#if,
            left_parenthesis,
            condition,
            right_parenthesis,
            body,
            r#else,
        })
    }

    pub(crate) fn parse_while(&mut self) -> Result<While<'arena>, ParseError> {
        let r#while = self.expect_keyword(TokenKind::While)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        let condition = self.parse_expression_ref()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let body = self.parse_block()?;

        Ok(While {
            r#while,
            left_parenthesis,
            condition,
            right_parenthesis,
            body,
        })
    }

    pub(crate) fn parse_do_while(&mut self) -> Result<DoWhile<'arena>, ParseError> {
        let r#do = self.expect_keyword(TokenKind::Do)?;
        let body = self.parse_block()?;
        let r#while = self.expect_keyword(TokenKind::While)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        let condition = self.parse_expression_ref()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(DoWhile {
            r#do,
            body,
            r#while,
            left_parenthesis,
            condition,
            right_parenthesis,
            semicolon,
        })
    }

    pub(crate) fn parse_for(&mut self) -> Result<For<'arena>, ParseError> {
        let r#for = self.expect_keyword(TokenKind::For)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        let initializations = self.parse_expression_list_until(TokenKind::Semicolon)?;
        let initializations_semicolon = self.expect_span(TokenKind::Semicolon)?;
        let conditions = self.parse_expression_list_until(TokenKind::Semicolon)?;
        let conditions_semicolon = self.expect_span(TokenKind::Semicolon)?;
        let increments = self.parse_expression_list_until(TokenKind::RightParenthesis)?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let body = self.parse_block()?;

        Ok(For {
            r#for,
            left_parenthesis,
            initializations,
            initializations_semicolon,
            conditions,
            conditions_semicolon,
            increments,
            right_parenthesis,
            body,
        })
    }

    /// Parses a `foreach` loop. The target(s) are restricted assignment
    /// targets, so destructuring works through the target grammar. The subject
    /// stops before `as`, which separates it from the target; a parenthesized
    /// or tuple subject (`foreach ($items) as $x`, `foreach (a, b) as $x`) is
    /// an ordinary expression, not a header wrapper.
    pub(crate) fn parse_foreach(&mut self) -> Result<Foreach<'arena>, ParseError> {
        let foreach = self.expect_keyword(TokenKind::Foreach)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        let saved = mem::replace(&mut self.no_as, true);
        let expression = self.parse_expression_bp(Precedence::Lowest);
        self.no_as = saved;
        let expression = self.arena.alloc(expression?);

        let r#as = self.expect_keyword(TokenKind::As)?;

        let first = self.parse_expression()?;
        let first_target = self.expression_to_assignment_target(&first)?;

        let target = if let Some(double_arrow) = self.eat_optional(TokenKind::EqualGreaterThan)? {
            let value = self.parse_expression()?;
            let value_target = self.expression_to_assignment_target(&value)?;

            ForeachTarget::KeyValue(ForeachKeyValueTarget {
                key: self.arena.alloc(first_target),
                double_arrow,
                value: self.arena.alloc(value_target),
            })
        } else {
            ForeachTarget::Value(ForeachValueTarget {
                value: self.arena.alloc(first_target),
            })
        };

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let body = self.parse_block()?;

        Ok(Foreach {
            foreach,
            left_parenthesis,
            expression,
            r#as,
            target,
            right_parenthesis,
            body,
        })
    }

    pub(crate) fn parse_try(&mut self) -> Result<Try<'arena>, ParseError> {
        let r#try = self.expect_keyword(TokenKind::Try)?;
        let block = self.parse_block()?;

        let mut catch_clauses = Vec::new_in(self.arena);
        while self.is_at(TokenKind::Catch)? {
            catch_clauses.push(self.parse_catch_clause()?);
        }

        let else_clause = if self.is_at(TokenKind::Else)? {
            let r#else = self.expect_keyword(TokenKind::Else)?;
            let block = self.parse_block()?;

            Some(TryElseClause { r#else, block })
        } else {
            None
        };

        let finally_clause = if self.is_at(TokenKind::Finally)? {
            let r#finally = self.expect_keyword(TokenKind::Finally)?;
            let block = self.parse_block()?;

            Some(TryFinallyClause { r#finally, block })
        } else {
            None
        };

        Ok(Try {
            r#try,
            block,
            catch_clauses: catch_clauses.leak(),
            else_clause,
            finally_clause,
        })
    }

    fn parse_catch_clause(&mut self) -> Result<TryCatchClause<'arena>, ParseError> {
        let r#catch = self.expect_keyword(TokenKind::Catch)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        let r#type = self.parse_type()?;
        let variable = self.parse_optional_catch_variable()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let guard = if self.is_at(TokenKind::If)? {
            let r#if = self.expect_keyword(TokenKind::If)?;
            let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
            let condition = self.parse_expression_ref()?;
            let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
            Some(TryCatchGuard {
                r#if,
                left_parenthesis,
                condition,
                right_parenthesis,
            })
        } else {
            None
        };
        let block = self.parse_block()?;

        Ok(TryCatchClause {
            r#catch,
            left_parenthesis,
            r#type,
            variable,
            right_parenthesis,
            guard,
            block,
        })
    }

    fn parse_optional_catch_variable(&mut self) -> Result<Option<Variable<'arena>>, ParseError> {
        if self.is_at(TokenKind::Variable)? {
            Ok(Some(self.parse_variable()?))
        } else {
            Ok(None)
        }
    }

    fn parse_expression_list_until(
        &mut self,
        terminator: TokenKind,
    ) -> Result<TokenSeparatedSequence<'arena, Expression<'arena>>, ParseError> {
        self.parse_comma_separated_until(terminator, Self::parse_expression)
    }
}
