//! The token stream: significant-token lookahead over the lexer.

use crate::unreachable_invariant;

use whim_span::Position;
use whim_span::Span;

use crate::arena::Arena;
use crate::arena::Vec;
use crate::cst::trivia::Trivia;
use crate::cst::trivia::TriviaKind;
use crate::error::Expected;
use crate::error::ParseError;
use crate::lexer::Lexer;
use crate::parser::internal::ring_buffer::RingBuffer;
use crate::token::Token;
use crate::token::kind::TokenKind;

/// Lookahead capacity. Must be a power of two (see [`RingBuffer`]). The
/// Whim grammar needs only a couple of tokens of lookahead; 8 is generous.
const LOOKAHEAD: usize = 8;

#[derive(Debug)]
pub(super) struct TokenStream<'input, 'arena, A>
where
    'input: 'arena,
    A: Arena,
{
    lexer: Lexer<'input, 'arena, A>,
    buffer: RingBuffer<Token<'input>, LOOKAHEAD>,
    trivia: Vec<'arena, Trivia<'arena>, A>,
    position: Position,
}

impl<'input, 'arena, A> TokenStream<'input, 'arena, A>
where
    A: Arena,
{
    #[inline]
    pub(super) fn new(arena: &'arena A, lexer: Lexer<'input, 'arena, A>) -> Self {
        let position = lexer.current_position();

        Self {
            lexer,
            buffer: RingBuffer::new(),
            trivia: Vec::new_in(arena),
            position,
        }
    }

    /// The end position of the most recently consumed significant token, or
    /// the start of the input if nothing has been consumed yet.
    #[inline]
    #[must_use]
    pub(super) const fn current_position(&self) -> Position {
        self.position
    }

    /// Whether every significant token has been consumed.
    #[inline]
    pub(super) fn has_reached_eof(&mut self) -> Result<bool, ParseError> {
        Ok(self.fill(1)? == 0)
    }

    /// Consumes and returns the next significant token; errors at end of file.
    #[inline]
    pub(super) fn consume(&mut self) -> Result<Token<'input>, ParseError> {
        if self.fill(1)? == 0 {
            return Err(self.unexpected_eof(Expected::Description("a token")));
        }

        let Some(token) = self.buffer.pop_front() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("`fill(1)` buffered a token") }
        };
        self.position = token.compute_span().end;

        Ok(token)
    }

    /// Consumes the next token if it is of `kind`, otherwise errors without
    /// consuming, including at end of file.
    #[inline]
    pub(super) fn eat(&mut self, kind: TokenKind) -> Result<Token<'input>, ParseError> {
        match self.peek()? {
            Some(token) if token.kind == kind => self.consume(),
            Some(token) => Err(ParseError::UnexpectedToken(
                Expected::Exactly(kind),
                token.kind,
                token.compute_span(),
            )),
            None => Err(self.unexpected_eof(Expected::Exactly(kind))),
        }
    }

    /// Like [`Self::eat`], returning the consumed token's span.
    #[inline]
    pub(super) fn eat_span(&mut self, kind: TokenKind) -> Result<Span, ParseError> {
        self.eat(kind).map(|token| token.compute_span())
    }

    /// Peeks at the next significant token without consuming it.
    #[inline]
    pub(super) fn peek(&mut self) -> Result<Option<Token<'input>>, ParseError> {
        self.lookahead(0)
    }

    /// Peeks at the `n`th-ahead significant token (0 = next).
    #[inline]
    pub(super) fn lookahead(&mut self, n: usize) -> Result<Option<Token<'input>>, ParseError> {
        if self.fill(n + 1)? <= n {
            return Ok(None);
        }

        Ok(self.buffer.get(n))
    }

    /// Peeks at the kind of the next significant token.
    #[inline]
    pub(super) fn peek_kind(&mut self) -> Result<Option<TokenKind>, ParseError> {
        Ok(self.peek()?.map(|token| token.kind))
    }

    /// Whether the next significant token is of `kind`. `false` at EOF.
    #[inline]
    pub(super) fn is_at(&mut self, kind: TokenKind) -> Result<bool, ParseError> {
        Ok(self.peek_kind()? == Some(kind))
    }

    /// Whether the next token can close a type-argument or type-parameter
    /// list, i.e. it begins with `>` (`>`, `>=`, `>>`, or `>>=`).
    #[inline]
    pub(super) fn is_at_type_list_close(&mut self) -> Result<bool, ParseError> {
        Ok(matches!(
            self.peek_kind()?,
            Some(
                TokenKind::GreaterThan
                    | TokenKind::GreaterThanEqual
                    | TokenKind::RightShift
                    | TokenKind::RightShiftEqual
            )
        ))
    }

    /// Consumes a single leading `>` that closes a type-argument or
    /// type-parameter list, splitting a `>=`/`>>`/`>>=` head token and leaving
    /// the remainder buffered. Returns the span of the consumed `>`.
    pub(super) fn eat_greater_than(&mut self) -> Result<Span, ParseError> {
        let Some(token) = self.peek()? else {
            return Err(self.unexpected_eof(Expected::Exactly(TokenKind::GreaterThan)));
        };

        let close = Span::new(token.start, token.start.forward(1));
        let remainder_kind = match token.kind {
            TokenKind::GreaterThan => {
                self.consume()?;
                return Ok(close);
            }
            TokenKind::RightShift => TokenKind::GreaterThan,
            TokenKind::GreaterThanEqual => TokenKind::Equal,
            TokenKind::RightShiftEqual => TokenKind::GreaterThanEqual,
            other => {
                return Err(ParseError::UnexpectedToken(
                    Expected::Exactly(TokenKind::GreaterThan),
                    other,
                    token.compute_span(),
                ));
            }
        };

        self.buffer.replace_front(Token {
            kind: remainder_kind,
            start: token.start.forward(1),
            value: &token.value[1..],
        });
        self.position = close.end;

        Ok(close)
    }

    /// An error for the given found token (or EOF) against `expected`.
    #[inline]
    #[must_use]
    pub(super) const fn unexpected(
        &self,
        found: Option<Token<'_>>,
        expected: Expected,
    ) -> ParseError {
        match found {
            Some(token) => ParseError::UnexpectedToken(expected, token.kind, token.compute_span()),
            None => self.unexpected_eof(expected),
        }
    }

    #[inline]
    #[must_use]
    const fn unexpected_eof(&self, expected: Expected) -> ParseError {
        ParseError::UnexpectedEndOfFile(expected, self.position)
    }

    /// The trivia collected so far, frozen into an arena slice.
    #[inline]
    #[must_use]
    pub(super) fn take_trivia(self) -> &'arena [Trivia<'arena>] {
        self.trivia.leak()
    }

    #[inline]
    fn fill(&mut self, n: usize) -> Result<usize, ParseError> {
        let n = n.min(LOOKAHEAD);
        while self.buffer.len() < n {
            match self.lexer.advance() {
                Some(Ok(token)) if token.kind.is_trivia() => {
                    if let Some(kind) = trivia_kind(token.kind) {
                        self.trivia.push(Trivia {
                            kind,
                            span: token.compute_span(),
                            value: token.value,
                        });
                    }
                }
                Some(Ok(token)) => self.buffer.push_back(token),
                Some(Err(error)) => return Err(ParseError::from(error)),
                None => break,
            }
        }

        Ok(self.buffer.len())
    }
}

/// Maps a trivia [`TokenKind`] to its [`TriviaKind`], or `None` for a
/// non-trivia kind (which never reaches [`TokenStream::fill`]'s trivia arm).
#[inline]
#[must_use]
const fn trivia_kind(kind: TokenKind) -> Option<TriviaKind> {
    match kind {
        TokenKind::Whitespace => Some(TriviaKind::Whitespace),
        TokenKind::SingleLineComment => Some(TriviaKind::SingleLineComment),
        TokenKind::MultiLineComment => Some(TriviaKind::MultiLineComment),
        TokenKind::DocBlockComment => Some(TriviaKind::DocBlockComment),
        TokenKind::Shebang => Some(TriviaKind::Shebang),
        _ => None,
    }
}
