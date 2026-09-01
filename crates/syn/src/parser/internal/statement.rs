//! Statement dispatch, blocks, and expression statements.

use crate::arena::Arena;
use crate::arena::Vec;

use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::statement::Block;
use crate::cst::statement::ExpressionStatement;
use crate::cst::statement::FinalLocal;
use crate::cst::statement::Statement;
use crate::cst::statement::Using;
use crate::cst::statement::UsingBinding;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::kind::TokenKind;
use crate::token::precedence::Precedence;

/// The furthest the class-like predicate scans over leading modifiers before
/// giving up and routing to the declaration parser. No real declaration carries
/// this many modifiers, and the value keeps the deepest lookahead the scan
/// performs within the token ring's capacity.
const MODIFIER_SCAN_LIMIT: usize = 6;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_statement(&mut self) -> Result<Statement<'arena>, ParseError> {
        self.enter()?;
        let result = self.parse_statement_inner();
        self.leave();

        result
    }

    fn parse_statement_inner(&mut self) -> Result<Statement<'arena>, ParseError> {
        let Some(kind) = self.peek_kind()? else {
            return Err(self.unexpected(Expected::Description("a statement")));
        };

        let statement = match kind {
            TokenKind::Semicolon => Statement::Noop(self.expect_span(TokenKind::Semicolon)?),
            TokenKind::LeftBrace => Statement::Block(self.parse_block()?),
            TokenKind::HashLeftBracket => return self.parse_attributed_statement(),
            TokenKind::Namespace if self.at_namespace_declaration()? => {
                Statement::Namespace(self.parse_namespace()?)
            }
            TokenKind::Use if self.at_use_declaration()? => Statement::Use(self.parse_use()?),
            TokenKind::Const if self.at_constant_declaration()? => {
                Statement::Constant(self.parse_constant()?)
            }
            kind if (kind.is_modifier()
                || matches!(
                    kind,
                    TokenKind::Class | TokenKind::Interface | TokenKind::Enum
                ))
                && self.at_class_like_declaration()? =>
            {
                return self.parse_class_like_statement();
            }
            TokenKind::Function
                if self
                    .lookahead(1)?
                    .is_some_and(|token| token.kind.is_function_name()) =>
            {
                Statement::Function(self.parse_function()?)
            }
            TokenKind::Type
                if matches!(
                    self.lookahead(1)?.map(|token| token.kind),
                    Some(TokenKind::Identifier)
                ) =>
            {
                Statement::TypeAlias(self.parse_type_alias()?)
            }
            TokenKind::Newtype => Statement::Newtype(self.parse_newtype()?),
            TokenKind::If => Statement::If(self.parse_if()?),
            TokenKind::While => Statement::While(self.parse_while()?),
            TokenKind::Do => Statement::DoWhile(self.parse_do_while()?),
            TokenKind::For => Statement::For(self.parse_for()?),
            TokenKind::Foreach => Statement::Foreach(self.parse_foreach()?),
            TokenKind::Using => Statement::Using(self.parse_using()?),
            TokenKind::Try => Statement::Try(self.parse_try()?),
            TokenKind::Final
                if self
                    .lookahead(1)?
                    .is_some_and(|token| token.kind == TokenKind::Variable) =>
            {
                Statement::FinalLocal(self.parse_final_local()?)
            }
            _ => return self.parse_expression_statement(),
        };

        Ok(statement)
    }

    pub(crate) fn at_namespace_declaration(&mut self) -> Result<bool, ParseError> {
        let names_a_namespace = matches!(
            self.lookahead(1)?.map(|token| token.kind),
            Some(
                TokenKind::Identifier
                    | TokenKind::QualifiedIdentifier
                    | TokenKind::FullyQualifiedIdentifier
            )
        );
        if !names_a_namespace {
            return Ok(false);
        }

        Ok(matches!(
            self.lookahead(2)?.map(|token| token.kind),
            Some(TokenKind::Semicolon | TokenKind::LeftBrace)
        ))
    }

    fn at_use_declaration(&mut self) -> Result<bool, ParseError> {
        Ok(matches!(
            self.lookahead(1)?.map(|token| token.kind),
            Some(
                TokenKind::Identifier
                    | TokenKind::QualifiedIdentifier
                    | TokenKind::FullyQualifiedIdentifier
            )
        ))
    }

    fn at_constant_declaration(&mut self) -> Result<bool, ParseError> {
        Ok(self
            .lookahead(1)?
            .is_some_and(|token| token.kind.is_constant_name()))
    }

    /// Whether the tokens ahead begin a class, interface, or enum declaration,
    /// rather than a bare use of a modifier or class-like keyword as a name. A
    /// class-like keyword (after any leading modifiers) must be followed by a
    /// name.
    fn at_class_like_declaration(&mut self) -> Result<bool, ParseError> {
        let mut offset = 0;
        while offset < MODIFIER_SCAN_LIMIT
            && self
                .lookahead(offset)?
                .is_some_and(|token| token.kind.is_modifier())
        {
            offset += 1;
        }

        if offset == MODIFIER_SCAN_LIMIT {
            return Ok(true);
        }

        let is_class_like = matches!(
            self.lookahead(offset)?.map(|token| token.kind),
            Some(TokenKind::Class | TokenKind::Interface | TokenKind::Enum)
        );

        Ok(is_class_like
            && matches!(
                self.lookahead(offset + 1)?.map(|token| token.kind),
                Some(TokenKind::Identifier)
            ))
    }

    pub(crate) fn parse_block(&mut self) -> Result<Block<'arena>, ParseError> {
        let left_brace = self.expect_span(TokenKind::LeftBrace)?;

        let mut statements = Vec::new_in(self.arena);
        while !self.is_at(TokenKind::RightBrace)? {
            statements.push(self.parse_statement()?);
        }

        let right_brace = self.expect_span(TokenKind::RightBrace)?;

        Ok(Block {
            left_brace,
            statements: statements.leak(),
            right_brace,
        })
    }

    fn parse_using(&mut self) -> Result<Using<'arena>, ParseError> {
        let using = self.expect_keyword(TokenKind::Using)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
        let mut bindings = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);

        loop {
            let target_expression = self.parse_expression_bp(Precedence::Coalesce)?;
            let target = self.expression_to_bind_target(&target_expression)?;
            let equal = self.expect_span(TokenKind::Equal)?;
            let value = self.parse_expression_ref()?;
            bindings.push(UsingBinding {
                target,
                equal,
                value,
            });

            if !self.is_at(TokenKind::Comma)? {
                break;
            }
            commas.push(self.consume()?);
            if self.is_at(TokenKind::RightParenthesis)? {
                break;
            }
        }

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let body = self.parse_block()?;
        Ok(Using {
            using,
            left_parenthesis,
            bindings: TokenSeparatedSequence::new(bindings, commas),
            right_parenthesis,
            body,
        })
    }

    fn parse_final_local(&mut self) -> Result<FinalLocal<'arena>, ParseError> {
        let r#final = self.expect_keyword(TokenKind::Final)?;
        let variable = self.parse_variable()?;
        let equal = self.expect_span(TokenKind::Equal)?;
        let value = self.parse_expression_ref()?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(FinalLocal {
            r#final,
            variable,
            equal,
            value,
            semicolon,
        })
    }

    pub(crate) fn parse_expression_statement(&mut self) -> Result<Statement<'arena>, ParseError> {
        let expression = self.parse_expression_ref()?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(Statement::Expression(ExpressionStatement {
            expression,
            semicolon,
        }))
    }
}
