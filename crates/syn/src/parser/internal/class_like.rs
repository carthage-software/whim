//! Class-like declarations: classes, interfaces, enums, and their members.

use crate::arena::Arena;
use crate::arena::Vec;

use crate::cst::atom::Modifier;
use crate::cst::class::Class;
use crate::cst::class::ClassLikeConstant;
use crate::cst::class::ClassLikeMember;
use crate::cst::class::Enum;
use crate::cst::class::EnumBackingType;
use crate::cst::class::EnumCase;
use crate::cst::class::EnumCaseValue;
use crate::cst::class::Extends;
use crate::cst::class::Implements;
use crate::cst::class::Interface;
use crate::cst::class::Method;
use crate::cst::class::MethodBody;
use crate::cst::class::Property;
use crate::cst::class::PropertyDefault;
use crate::cst::class::SealedPermissions;
use crate::cst::declaration::AttributeList;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::statement::Statement;
use crate::cst::r#type::NamedType;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::Token;
use crate::token::kind::TokenKind;

impl<'input, 'arena, A> Parser<'input, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_class_like_statement(&mut self) -> Result<Statement<'arena>, ParseError> {
        let attribute_lists = self.empty_slice();

        self.parse_class_like(attribute_lists)
    }

    pub(crate) fn parse_class_like(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Statement<'arena>, ParseError> {
        let modifiers = self.parse_modifiers()?;

        match self.peek_kind()? {
            Some(TokenKind::Class) => Ok(Statement::Class(
                self.parse_class(attribute_lists, modifiers)?,
            )),
            Some(TokenKind::Interface) => {
                self.reject_leading_modifiers(modifiers)?;

                Ok(Statement::Interface(self.parse_interface(attribute_lists)?))
            }
            Some(TokenKind::Enum) => {
                self.reject_leading_modifiers(modifiers)?;

                Ok(Statement::Enum(self.parse_enum(attribute_lists)?))
            }
            _ => Err(self.unexpected(Expected::OneOf(&[
                TokenKind::Class,
                TokenKind::Interface,
                TokenKind::Enum,
            ]))),
        }
    }

    pub(crate) fn parse_class(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
        modifiers: &'arena [Modifier<'arena>],
    ) -> Result<Class<'arena>, ParseError> {
        let class = self.expect_keyword(TokenKind::Class)?;
        let name = self.parse_local_identifier()?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let extends = self.parse_extends()?;
        let implements = self.parse_implements()?;
        let permissions = self.parse_sealed_permissions()?;
        let left_brace = self.expect_span(TokenKind::LeftBrace)?;
        let members = self.parse_class_like_members()?;
        let right_brace = self.expect_span(TokenKind::RightBrace)?;

        Ok(Class {
            attribute_lists,
            modifiers,
            class,
            name,
            type_parameters,
            extends,
            implements,
            permissions,
            left_brace,
            members,
            right_brace,
        })
    }

    pub(crate) fn parse_interface(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Interface<'arena>, ParseError> {
        let interface = self.expect_keyword(TokenKind::Interface)?;
        let name = self.parse_local_identifier()?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let extends = self.parse_extends()?;
        let permissions = self.parse_sealed_permissions()?;
        let left_brace = self.expect_span(TokenKind::LeftBrace)?;
        let members = self.parse_class_like_members()?;
        let right_brace = self.expect_span(TokenKind::RightBrace)?;

        Ok(Interface {
            attribute_lists,
            interface,
            name,
            type_parameters,
            extends,
            permissions,
            left_brace,
            members,
            right_brace,
        })
    }

    pub(crate) fn parse_enum(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Enum<'arena>, ParseError> {
        let r#enum = self.expect_keyword(TokenKind::Enum)?;
        let name = self.parse_local_identifier()?;
        let type_parameters = self.parse_optional_type_parameter_list()?;

        let backing_type = if let Some(colon) = self.eat_optional(TokenKind::Colon)? {
            let r#type = self.parse_type()?;

            Some(EnumBackingType { colon, r#type })
        } else {
            None
        };

        let implements = self.parse_implements()?;
        let left_brace = self.expect_span(TokenKind::LeftBrace)?;
        let members = self.parse_class_like_members()?;
        let right_brace = self.expect_span(TokenKind::RightBrace)?;

        Ok(Enum {
            attribute_lists,
            r#enum,
            name,
            type_parameters,
            backing_type,
            implements,
            left_brace,
            members,
            right_brace,
        })
    }

    pub(crate) fn parse_modifiers(&mut self) -> Result<&'arena [Modifier<'arena>], ParseError> {
        let mut modifiers = Vec::new_in(self.arena);

        while let Some(kind) = self.peek_kind()? {
            if !kind.is_modifier() {
                break;
            }

            let token = self.consume()?;
            modifiers.push(self.modifier_of(token));
        }

        Ok(modifiers.leak())
    }

    fn parse_extends(&mut self) -> Result<Option<Extends<'arena>>, ParseError> {
        if self.is_at(TokenKind::Extends)? {
            let extends = self.expect_keyword(TokenKind::Extends)?;
            let types = self.parse_named_type_sequence()?;

            Ok(Some(Extends { extends, types }))
        } else {
            Ok(None)
        }
    }

    fn parse_sealed_permissions(
        &mut self,
    ) -> Result<Option<SealedPermissions<'arena>>, ParseError> {
        if !self.is_at(TokenKind::For)? {
            return Ok(None);
        }
        let r#for = self.expect_keyword(TokenKind::For)?;
        let mut names = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        loop {
            names.push(self.parse_identifier()?);
            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
            } else {
                break;
            }
        }
        Ok(Some(SealedPermissions {
            r#for,
            types: TokenSeparatedSequence::new(names, commas),
        }))
    }

    fn parse_implements(&mut self) -> Result<Option<Implements<'arena>>, ParseError> {
        if self.is_at(TokenKind::Implements)? {
            let implements = self.expect_keyword(TokenKind::Implements)?;
            let types = self.parse_named_type_sequence()?;

            Ok(Some(Implements { implements, types }))
        } else {
            Ok(None)
        }
    }

    fn parse_named_type_sequence(
        &mut self,
    ) -> Result<TokenSeparatedSequence<'arena, NamedType<'arena>>, ParseError> {
        let mut names = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        loop {
            names.push(self.parse_named_type()?);

            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
            } else {
                break;
            }
        }

        Ok(TokenSeparatedSequence::new(names, commas))
    }

    fn parse_class_like_members(
        &mut self,
    ) -> Result<&'arena [ClassLikeMember<'arena>], ParseError> {
        let mut members = Vec::new_in(self.arena);
        while !self.is_at(TokenKind::RightBrace)? {
            members.push(self.parse_class_like_member()?);
        }

        Ok(members.leak())
    }

    pub(crate) fn parse_class_like_member(
        &mut self,
    ) -> Result<ClassLikeMember<'arena>, ParseError> {
        let attribute_lists = self.parse_attribute_lists()?;

        if self.is_at(TokenKind::Case)? {
            return Ok(ClassLikeMember::EnumCase(
                self.parse_enum_case(attribute_lists)?,
            ));
        }

        let modifiers = self.parse_modifiers()?;

        match self.peek_kind()? {
            Some(TokenKind::Const) => Ok(ClassLikeMember::Constant(
                self.parse_class_like_constant(attribute_lists, modifiers)?,
            )),
            Some(TokenKind::Function) => Ok(ClassLikeMember::Method(
                self.parse_method(attribute_lists, modifiers)?,
            )),
            _ => Ok(ClassLikeMember::Property(
                self.parse_property(attribute_lists, modifiers)?,
            )),
        }
    }

    fn parse_enum_case(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<EnumCase<'arena>, ParseError> {
        let case = self.expect_keyword(TokenKind::Case)?;
        let name = self.parse_member_name()?;

        let value = if let Some(equals) = self.eat_optional(TokenKind::Equal)? {
            let expression = self.parse_expression_ref()?;

            Some(EnumCaseValue { equals, expression })
        } else {
            None
        };

        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(EnumCase {
            attribute_lists,
            case,
            name,
            value,
            semicolon,
        })
    }

    fn parse_class_like_constant(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
        modifiers: &'arena [Modifier<'arena>],
    ) -> Result<ClassLikeConstant<'arena>, ParseError> {
        let r#const = self.expect_keyword(TokenKind::Const)?;

        let r#type = if self.class_constant_has_type()? {
            Some(self.parse_type()?)
        } else {
            None
        };

        let name = self.parse_member_name()?;
        let equals = self.expect_span(TokenKind::Equal)?;
        let value = self.parse_expression_ref()?;
        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(ClassLikeConstant {
            attribute_lists,
            modifiers,
            r#const,
            r#type,
            name,
            equals,
            value,
            semicolon,
        })
    }

    fn parse_method(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
        modifiers: &'arena [Modifier<'arena>],
    ) -> Result<Method<'arena>, ParseError> {
        let function = self.expect_keyword(TokenKind::Function)?;
        let name = self.parse_member_name()?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let parameter_list = self.parse_parameter_list()?;
        let return_type = self.parse_return_type()?;

        let body = if let Some(semicolon) = self.eat_optional(TokenKind::Semicolon)? {
            MethodBody::Abstract(semicolon)
        } else {
            MethodBody::Concrete(self.parse_block()?)
        };

        Ok(Method {
            attribute_lists,
            modifiers,
            function,
            name,
            type_parameters,
            parameter_list,
            return_type,
            body,
        })
    }

    fn parse_property(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
        modifiers: &'arena [Modifier<'arena>],
    ) -> Result<Property<'arena>, ParseError> {
        let r#type = if self.is_at(TokenKind::Variable)? {
            None
        } else {
            Some(self.parse_type()?)
        };

        let variable = self.parse_variable()?;

        let default = if let Some(equals) = self.eat_optional(TokenKind::Equal)? {
            let value = self.parse_expression_ref()?;

            Some(PropertyDefault { equals, value })
        } else {
            None
        };

        let semicolon = self.expect_span(TokenKind::Semicolon)?;

        Ok(Property {
            attribute_lists,
            modifiers,
            r#type,
            variable,
            default,
            semicolon,
        })
    }

    /// Whether a class constant declares an explicit type before its name.
    fn class_constant_has_type(&mut self) -> Result<bool, ParseError> {
        let at_name = self.peek_kind()?.is_some_and(|kind| kind.is_member_name());
        let name_then_equals = at_name
            && matches!(
                self.lookahead(1)?.map(|token| token.kind),
                Some(TokenKind::Equal)
            );

        Ok(!name_then_equals)
    }

    fn reject_leading_modifiers(
        &mut self,
        modifiers: &'arena [Modifier<'arena>],
    ) -> Result<(), ParseError> {
        if modifiers.is_empty() {
            return Ok(());
        }

        Err(self.unexpected(Expected::Description(
            "a class declaration (only classes accept modifiers)",
        )))
    }

    fn modifier_of(&self, token: Token<'input>) -> Modifier<'arena> {
        let keyword = self.keyword_of(token);

        match token.kind {
            TokenKind::Public => Modifier::Public(keyword),
            TokenKind::Protected => Modifier::Protected(keyword),
            TokenKind::Private => Modifier::Private(keyword),
            TokenKind::Static => Modifier::Static(keyword),
            TokenKind::Final => Modifier::Final(keyword),
            TokenKind::Abstract => Modifier::Abstract(keyword),
            TokenKind::Readonly => Modifier::Readonly(keyword),
            kind => unreachable!("not a modifier: {kind:?}"),
        }
    }
}
