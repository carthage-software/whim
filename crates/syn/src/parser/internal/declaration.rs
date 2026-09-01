//! Top-level declarations: namespaces, use statements, constants, type
//! aliases, and attributes.

use crate::arena::Arena;
use crate::arena::Vec;
use whim_span::HasSpan;

use crate::cst::atom::Identifier;
use crate::cst::declaration::Attribute;
use crate::cst::declaration::AttributeList;
use crate::cst::declaration::Constant;
use crate::cst::declaration::Namespace;
use crate::cst::declaration::NamespaceBody;
use crate::cst::declaration::NamespaceImplicitBody;
use crate::cst::declaration::Use;
use crate::cst::declaration::UseItem;
use crate::cst::declaration::UseItemAlias;
use crate::cst::declaration::UseItemList;
use crate::cst::declaration::UseItemSequence;
use crate::cst::declaration::UseItems;
use crate::cst::expression::Expression;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::statement::ExpressionStatement;
use crate::cst::statement::Statement;
use crate::cst::r#type::Newtype;
use crate::cst::r#type::TypeAlias;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::kind::TokenKind;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_namespace(&mut self) -> Result<Namespace<'arena>, ParseError> {
        let namespace = self.expect_keyword(TokenKind::Namespace)?;
        let name = self.parse_identifier()?;

        if name.value().split('\\').any(|segment| segment == "_") {
            return Err(ParseError::ReservedIdentifier(name.span()));
        }

        let body = if self.is_at(TokenKind::LeftBrace)? {
            NamespaceBody::BraceDelimited(self.parse_block()?)
        } else {
            let semicolon = self.expect_span(TokenKind::Semicolon)?;

            let mut statements = Vec::new_in(self.arena);
            while !self.stream.has_reached_eof()?
                && !(self.is_at(TokenKind::Namespace)? && self.at_namespace_declaration()?)
            {
                statements.push(self.parse_statement()?);
            }

            NamespaceBody::Implicit(NamespaceImplicitBody {
                semicolon,
                statements: statements.leak(),
            })
        };

        Ok(Namespace {
            namespace,
            name,
            body,
        })
    }

    pub(crate) fn parse_use(&mut self) -> Result<Use<'arena>, ParseError> {
        let r#use = self.expect_keyword(TokenKind::Use)?;
        let items = self.parse_use_items()?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(Use {
            r#use,
            items,
            semicolon,
        })
    }

    fn parse_use_items(&mut self) -> Result<UseItems<'arena>, ParseError> {
        let first = self.parse_identifier()?;

        if self.is_at(TokenKind::NamespaceSeparator)?
            && matches!(
                self.lookahead(1)?.map(|token| token.kind),
                Some(TokenKind::LeftBrace)
            )
        {
            let namespace_separator = self.expect_span(TokenKind::NamespaceSeparator)?;
            let left_brace = self.expect_span(TokenKind::LeftBrace)?;
            let items = self.parse_use_item_list()?;
            let right_brace = self.expect_span(TokenKind::RightBrace)?;

            return Ok(UseItems::List(UseItemList {
                namespace: first,
                namespace_separator,
                left_brace,
                items,
                right_brace,
            }));
        }

        let items = self.parse_use_item_sequence(first)?;

        Ok(UseItems::Sequence(UseItemSequence { items }))
    }

    fn parse_use_item_sequence(
        &mut self,
        first: Identifier<'arena>,
    ) -> Result<TokenSeparatedSequence<'arena, UseItem<'arena>>, ParseError> {
        let mut items = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);

        let alias = self.parse_use_item_alias()?;
        items.push(UseItem { name: first, alias });

        while self.is_at(TokenKind::Comma)? {
            commas.push(self.consume()?);
            items.push(self.parse_use_item()?);
        }

        Ok(TokenSeparatedSequence::new(items, commas))
    }

    fn parse_use_item_list(
        &mut self,
    ) -> Result<TokenSeparatedSequence<'arena, UseItem<'arena>>, ParseError> {
        self.parse_comma_separated_until(TokenKind::RightBrace, Self::parse_use_item)
    }

    fn parse_use_item(&mut self) -> Result<UseItem<'arena>, ParseError> {
        let name = self.parse_identifier()?;
        let alias = self.parse_use_item_alias()?;

        Ok(UseItem { name, alias })
    }

    fn parse_use_item_alias(&mut self) -> Result<Option<UseItemAlias<'arena>>, ParseError> {
        if self.is_at(TokenKind::As)? {
            let r#as = self.expect_keyword(TokenKind::As)?;
            let identifier = self.parse_local_identifier()?;

            Ok(Some(UseItemAlias { r#as, identifier }))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn parse_constant(&mut self) -> Result<Constant<'arena>, ParseError> {
        let attribute_lists = self.empty_slice();

        self.parse_constant_with(attribute_lists)
    }

    pub(crate) fn parse_constant_with(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Constant<'arena>, ParseError> {
        let r#const = self.expect_keyword(TokenKind::Const)?;
        let name = self.parse_constant_name()?;
        let equals = self.expect_span(TokenKind::Equal)?;
        let value = self.parse_expression_ref()?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(Constant {
            attribute_lists,
            r#const,
            name,
            equals,
            value,
            semicolon,
        })
    }

    pub(crate) fn parse_type_alias(&mut self) -> Result<TypeAlias<'arena>, ParseError> {
        let attribute_lists = self.empty_slice();

        self.parse_type_alias_with(attribute_lists)
    }

    pub(crate) fn parse_type_alias_with(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<TypeAlias<'arena>, ParseError> {
        let r#type = self.expect_keyword(TokenKind::Type)?;
        let name = self.parse_local_identifier()?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let equals = self.expect_span(TokenKind::Equal)?;
        let aliased = self.parse_type()?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(TypeAlias {
            attribute_lists,
            r#type,
            name,
            type_parameters,
            equals,
            aliased,
            semicolon,
        })
    }

    pub(crate) fn parse_newtype(&mut self) -> Result<Newtype<'arena>, ParseError> {
        let attribute_lists = self.empty_slice();

        self.parse_newtype_with(attribute_lists)
    }

    pub(crate) fn parse_newtype_with(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Newtype<'arena>, ParseError> {
        let newtype = self.expect_keyword(TokenKind::Newtype)?;
        let name = self.parse_local_identifier()?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let equals = self.expect_span(TokenKind::Equal)?;
        let backing = self.parse_type()?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(Newtype {
            attribute_lists,
            newtype,
            name,
            type_parameters,
            equals,
            backing,
            semicolon,
        })
    }

    pub(crate) fn parse_attribute_lists(
        &mut self,
    ) -> Result<&'arena [AttributeList<'arena>], ParseError> {
        let mut lists = Vec::new_in(self.arena);
        while self.is_at(TokenKind::HashLeftBracket)? {
            lists.push(self.parse_attribute_list()?);
        }

        Ok(lists.leak())
    }

    fn parse_attribute_list(&mut self) -> Result<AttributeList<'arena>, ParseError> {
        let hash_left_bracket = self.expect_span(TokenKind::HashLeftBracket)?;

        let attributes =
            self.parse_comma_separated_until(TokenKind::RightBracket, Self::parse_attribute)?;

        let right_bracket = self.expect_span(TokenKind::RightBracket)?;

        Ok(AttributeList {
            hash_left_bracket,
            attributes,
            right_bracket,
        })
    }

    fn parse_attribute(&mut self) -> Result<Attribute<'arena>, ParseError> {
        let name = self.parse_identifier()?;
        let argument_list = if self.is_at(TokenKind::LeftParenthesis)? {
            Some(self.parse_argument_list()?)
        } else {
            None
        };

        Ok(Attribute {
            name,
            argument_list,
        })
    }

    pub(crate) fn parse_attributed_statement(&mut self) -> Result<Statement<'arena>, ParseError> {
        let attribute_lists = self.parse_attribute_lists()?;

        let Some(kind) = self.peek_kind()? else {
            return Err(self.unexpected(Expected::Description(
                "a declaration after an attribute list",
            )));
        };

        match kind {
            TokenKind::Abstract
            | TokenKind::Final
            | TokenKind::Readonly
            | TokenKind::Class
            | TokenKind::Interface
            | TokenKind::Enum => self.parse_class_like(attribute_lists),
            TokenKind::Const => Ok(Statement::Constant(
                self.parse_constant_with(attribute_lists)?,
            )),
            TokenKind::Type
                if matches!(
                    self.lookahead(1)?.map(|token| token.kind),
                    Some(TokenKind::Identifier)
                ) =>
            {
                Ok(Statement::TypeAlias(
                    self.parse_type_alias_with(attribute_lists)?,
                ))
            }
            TokenKind::Newtype => Ok(Statement::Newtype(
                self.parse_newtype_with(attribute_lists)?,
            )),
            TokenKind::Function
                if self
                    .lookahead(1)?
                    .is_some_and(|token| token.kind.is_function_name()) =>
            {
                Ok(Statement::Function(
                    self.parse_function_with(attribute_lists)?,
                ))
            }
            TokenKind::Function | TokenKind::Fn => {
                let expression = self.parse_closure_or_short_closure(attribute_lists)?;
                let expression = self.arena.alloc(expression);
                let semicolon = self.expect_span(TokenKind::Semicolon)?;

                Ok(Statement::Expression(ExpressionStatement {
                    expression,
                    semicolon,
                }))
            }
            _ => Err(self.unexpected(Expected::Description(
                "a declaration after an attribute list",
            ))),
        }
    }

    pub(crate) fn parse_attributed_expression(&mut self) -> Result<Expression<'arena>, ParseError> {
        let attribute_lists = self.parse_attribute_lists()?;

        self.parse_closure_or_short_closure(attribute_lists)
    }

    fn parse_closure_or_short_closure(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Expression<'arena>, ParseError> {
        match self.peek_kind()? {
            Some(TokenKind::Function) => Ok(Expression::Closure(
                self.parse_closure_with(attribute_lists)?,
            )),
            Some(TokenKind::Fn) => Ok(Expression::ShortClosure(
                self.parse_short_closure_with(attribute_lists)?,
            )),
            _ => Err(self.unexpected(Expected::Description(
                "a closure or short closure after an attribute list",
            ))),
        }
    }
}
