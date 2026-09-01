//! The Whim parser.

mod internal;
mod stream;

use whim_span::HasSpan;
use whim_span::Position;
use whim_span::Span;

use crate::arena::Arena;
use crate::arena::Vec;
use crate::cst::Program;
use crate::cst::node::Node;
use crate::cst::walker::deepest_path_in;
use crate::error::Expected;
use crate::error::ParseError;
use crate::input::Input;
use crate::lexer::Lexer;
use crate::parser::stream::TokenStream;
use crate::token::Token;
use crate::token::kind::TokenKind;

/// Maximum nesting depth for expressions, statements, and types, guarding the
/// parser's own stack against pathologically deep input.
pub const MAX_RECURSION_DEPTH: u16 = 256;

/// Maximum depth of the tree a parse returns, measured in nodes on the
/// longest root-to-leaf path ([`crate::cst::walker::deepest_path`] reports it).
pub const MAX_STRUCTURAL_DEPTH: usize = 4096;

/// The parser: an arena reference, the source it owns, the token stream, the
/// nesting depth, and the first error encountered, if any.
#[derive(Debug)]
pub struct Parser<'input, 'arena, A>
where
    'input: 'arena,
    A: Arena,
{
    arena: &'arena A,
    source: &'input str,
    stream: TokenStream<'input, 'arena, A>,
    depth: u16,
    /// When set, a top-level `as` is treated as a separator rather than the
    /// type-assert operator. Used while parsing a `foreach` subject, where
    /// `foreach $items as $item` must stop the subject before `as`.
    no_as: bool,
}

impl<'input, 'arena, A> Parser<'input, 'arena, A>
where
    A: Arena,
{
    #[inline]
    #[must_use]
    pub fn new(arena: &'arena A, source: &'input str) -> Self {
        Self::with_input(arena, source, Input::new(source))
    }

    fn with_input(arena: &'arena A, source: &'input str, input: Input<'input>) -> Self {
        let lexer = Lexer::new(arena, input);
        let stream = TokenStream::new(arena, lexer);

        Self {
            arena,
            source,
            stream,
            depth: 0,
            no_as: false,
        }
    }

    /// Parses the parser's source into a [`Program`], stopping at the first
    /// syntax error.
    ///
    /// # Errors
    ///
    /// Returns the first syntax or structural-depth error.
    pub fn parse(mut self) -> Result<&'arena Program<'arena>, ParseError> {
        let mut statements = Vec::new_in(self.arena);

        loop {
            match self.stream.has_reached_eof() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => {
                    return Err(error);
                }
            }

            statements.push(self.parse_statement()?);
        }

        let trivia = self.stream.take_trivia();
        let program = self.arena.alloc(Program {
            source_text: self.source,
            trivia,
            statements: statements.leak(),
        });

        let deepest = deepest_path_in(self.arena, Node::Program(program));
        if deepest.levels > MAX_STRUCTURAL_DEPTH {
            return Err(ParseError::StructuralDepthExceeded(deepest.end.span()));
        }

        Ok(program)
    }

    #[inline]
    pub(crate) fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_RECURSION_DEPTH {
            self.depth -= 1;
            let span = if let Some(token) = self.stream.peek()? {
                token.compute_span()
            } else {
                let position = self.stream.current_position();
                Span::new(position, position)
            };

            return Err(ParseError::RecursionLimitExceeded(span));
        }

        Ok(())
    }

    #[inline]
    pub(crate) const fn leave(&mut self) {
        self.depth -= 1;
    }

    #[inline]
    pub(crate) fn peek(&mut self) -> Result<Option<Token<'input>>, ParseError> {
        self.stream.peek()
    }

    #[inline]
    pub(crate) fn lookahead(&mut self, n: usize) -> Result<Option<Token<'input>>, ParseError> {
        self.stream.lookahead(n)
    }

    #[inline]
    pub(crate) fn peek_kind(&mut self) -> Result<Option<TokenKind>, ParseError> {
        self.stream.peek_kind()
    }

    #[inline]
    pub(crate) fn is_at(&mut self, kind: TokenKind) -> Result<bool, ParseError> {
        self.stream.is_at(kind)
    }

    #[inline]
    pub(crate) fn consume(&mut self) -> Result<Token<'input>, ParseError> {
        self.stream.consume()
    }

    #[inline]
    pub(crate) fn expect(&mut self, kind: TokenKind) -> Result<Token<'input>, ParseError> {
        self.stream.eat(kind)
    }

    #[inline]
    pub(crate) fn expect_span(&mut self, kind: TokenKind) -> Result<Span, ParseError> {
        self.stream.eat_span(kind)
    }

    #[inline]
    pub(crate) fn is_at_type_list_close(&mut self) -> Result<bool, ParseError> {
        self.stream.is_at_type_list_close()
    }

    #[inline]
    pub(crate) fn expect_type_list_close(&mut self) -> Result<Span, ParseError> {
        self.stream.eat_greater_than()
    }

    #[inline]
    pub(crate) fn eat_optional(&mut self, kind: TokenKind) -> Result<Option<Span>, ParseError> {
        if self.is_at(kind)? {
            Ok(Some(self.consume()?.compute_span()))
        } else {
            Ok(None)
        }
    }

    #[inline]
    pub(crate) fn unexpected(&mut self, expected: Expected) -> ParseError {
        match self.stream.peek() {
            Ok(found) => self.stream.unexpected(found, expected),
            Err(error) => error,
        }
    }
}

/// Parses `source` into a [`Program`] using `arena`.
///
/// # Errors
///
/// Returns the first syntax or structural-depth error.
#[inline]
pub fn parse<'arena, A>(
    arena: &'arena A,
    source: &str,
) -> Result<&'arena Program<'arena>, ParseError>
where
    A: Arena,
{
    let source = arena.alloc_str(source);

    Parser::new(arena, source).parse()
}

/// Parses `fragment` while retaining spans within `source`.
///
/// # Errors
///
/// Returns the first syntax or structural-depth error.
pub fn parse_fragment<'arena, A>(
    arena: &'arena A,
    source: &'arena str,
    fragment: &'arena str,
    start: Position,
) -> Result<&'arena Program<'arena>, ParseError>
where
    A: Arena,
{
    Parser::with_input(arena, source, Input::anchored_at(fragment, start)).parse()
}
