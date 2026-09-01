//! Shared leaf builders: keywords, identifiers, variables, and literals.

use crate::arena::Arena;
use crate::arena::Vec;

use crate::cst::atom::FullyQualifiedIdentifier;
use crate::cst::atom::Identifier;
use crate::cst::atom::Keyword;
use crate::cst::atom::Literal;
use crate::cst::atom::LiteralFloat;
use crate::cst::atom::LiteralInteger;
use crate::cst::atom::LiteralString;
use crate::cst::atom::LiteralStringKind;
use crate::cst::atom::LocalIdentifier;
use crate::cst::atom::QualifiedIdentifier;
use crate::cst::atom::Variable;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::Token;
use crate::token::kind::TokenKind;
use crate::utils::parse_literal_float_in;
use crate::utils::parse_literal_integer;
use crate::utils::parse_literal_string_detailed_in;

impl<'input, 'arena, A> Parser<'input, 'arena, A>
where
    A: Arena,
{
    #[inline]
    #[must_use]
    pub(crate) fn empty_slice<T>(&self) -> &'arena [T] {
        Vec::new_in(self.arena).leak()
    }

    #[inline]
    pub(crate) fn keyword_of(&self, token: Token<'input>) -> Keyword<'arena> {
        Keyword {
            span: token.compute_span(),
            value: self.arena.alloc_str(token.value),
        }
    }

    /// Consumes the next token, which must be `kind`, and returns it as a
    /// [`Keyword`].
    #[inline]
    pub(crate) fn expect_keyword(
        &mut self,
        kind: TokenKind,
    ) -> Result<Keyword<'arena>, ParseError> {
        let token = self.expect(kind)?;

        Ok(self.keyword_of(token))
    }

    pub(crate) fn parse_identifier(&mut self) -> Result<Identifier<'arena>, ParseError> {
        let token = self.consume()?;
        let span = token.compute_span();
        let value = self.arena.alloc_str(token.value);

        match token.kind {
            TokenKind::Identifier => Ok(Identifier::Local(LocalIdentifier { span, value })),
            TokenKind::QualifiedIdentifier => {
                Ok(Identifier::Qualified(QualifiedIdentifier { span, value }))
            }
            TokenKind::FullyQualifiedIdentifier => {
                Ok(Identifier::FullyQualified(FullyQualifiedIdentifier {
                    span,
                    value,
                }))
            }
            _ => Err(ParseError::UnexpectedToken(
                Expected::Description("a name"),
                token.kind,
                span,
            )),
        }
    }

    pub(crate) fn parse_local_identifier(&mut self) -> Result<LocalIdentifier<'arena>, ParseError> {
        let token = self.expect(TokenKind::Identifier)?;
        let span = token.compute_span();

        if token.value == "_" {
            return Err(ParseError::ReservedIdentifier(span));
        }

        Ok(LocalIdentifier {
            span,
            value: self.arena.alloc_str(token.value),
        })
    }

    /// Parses a member name after `->`, `?->`, `::`, or a declaring keyword.
    pub(crate) fn parse_member_name(&mut self) -> Result<LocalIdentifier<'arena>, ParseError> {
        self.parse_name_with(TokenKind::is_member_name, "a member name")
    }

    pub(crate) fn parse_function_name(&mut self) -> Result<LocalIdentifier<'arena>, ParseError> {
        self.parse_name_with(TokenKind::is_function_name, "a function name")
    }

    pub(crate) fn parse_constant_name(&mut self) -> Result<LocalIdentifier<'arena>, ParseError> {
        self.parse_name_with(TokenKind::is_constant_name, "a constant name")
    }

    fn parse_name_with(
        &mut self,
        accepts: fn(&TokenKind) -> bool,
        expected: &'static str,
    ) -> Result<LocalIdentifier<'arena>, ParseError> {
        let Some(token) = self.peek()? else {
            return Err(self.unexpected(Expected::Description(expected)));
        };

        if accepts(&token.kind) {
            let token = self.consume()?;
            let span = token.compute_span();

            if token.value == "_" {
                return Err(ParseError::ReservedIdentifier(span));
            }

            return Ok(LocalIdentifier {
                span,
                value: self.arena.alloc_str(token.value),
            });
        }

        Err(ParseError::UnexpectedToken(
            Expected::Description(expected),
            token.kind,
            token.compute_span(),
        ))
    }

    pub(crate) fn parse_integer_literal(&mut self) -> Result<LiteralInteger<'arena>, ParseError> {
        let token = self.expect(TokenKind::LiteralInteger)?;
        let span = token.compute_span();
        let value =
            parse_literal_integer(token.value.as_bytes()).ok_or(ParseError::UnexpectedToken(
                Expected::Description("a valid integer literal"),
                token.kind,
                span,
            ))?;

        Ok(LiteralInteger {
            span,
            raw: self.arena.alloc_str(token.value),
            value,
        })
    }

    pub(crate) fn parse_variable(&mut self) -> Result<Variable<'arena>, ParseError> {
        let token = self.expect(TokenKind::Variable)?;

        Ok(Variable {
            span: token.compute_span(),
            name: self.arena.alloc_str(token.value),
        })
    }

    pub(crate) fn parse_comma_separated_until<T, F>(
        &mut self,
        terminator: TokenKind,
        mut parse: F,
    ) -> Result<TokenSeparatedSequence<'arena, T>, ParseError>
    where
        T: 'arena,
        F: FnMut(&mut Self) -> Result<T, ParseError>,
    {
        let mut items = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);

        while !self.is_at(terminator)? {
            items.push(parse(self)?);
            if !self.is_at(TokenKind::Comma)? {
                break;
            }

            commas.push(self.consume()?);
        }

        Ok(TokenSeparatedSequence::new(items, commas))
    }

    pub(crate) fn literal_of(&self, token: Token<'input>) -> Result<Literal<'arena>, ParseError> {
        let span = token.compute_span();
        let raw = self.arena.alloc_str(token.value);

        Ok(match token.kind {
            TokenKind::True => Literal::True(self.keyword_of(token)),
            TokenKind::False => Literal::False(self.keyword_of(token)),
            TokenKind::Null => Literal::Null(self.keyword_of(token)),
            TokenKind::LiteralInteger => Literal::Integer(LiteralInteger {
                span,
                raw,
                value: parse_literal_integer(token.value.as_bytes()).ok_or(
                    ParseError::UnexpectedToken(
                        Expected::Description("a valid integer literal"),
                        token.kind,
                        span,
                    ),
                )?,
            }),
            TokenKind::LiteralFloat => Literal::Float(LiteralFloat {
                span,
                raw,
                value: parse_literal_float_in(self.arena, token.value.as_bytes()).ok_or(
                    ParseError::UnexpectedToken(
                        Expected::Description("a valid float literal"),
                        token.kind,
                        span,
                    ),
                )?,
            }),
            TokenKind::LiteralString => Literal::String(LiteralString {
                kind: if token.value.starts_with('"') {
                    LiteralStringKind::DoubleQuoted
                } else {
                    LiteralStringKind::SingleQuoted
                },
                span,
                raw,
                value: parse_literal_string_detailed_in(
                    self.arena,
                    token.value.as_bytes(),
                    None,
                    true,
                )
                .map_err(|error| ParseError::InvalidStringLiteral(error, span))?,
            }),
            _ => {
                return Err(ParseError::UnexpectedToken(
                    Expected::Description("a literal"),
                    token.kind,
                    span,
                ));
            }
        })
    }
}
