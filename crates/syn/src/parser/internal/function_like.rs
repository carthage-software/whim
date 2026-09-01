//! Function-like constructs: functions, closures, and short closures.

use crate::arena::Arena;

use crate::cst::declaration::AttributeList;
use crate::cst::function::Closure;
use crate::cst::function::ClosureUseClause;
use crate::cst::function::Function;
use crate::cst::function::Parameter;
use crate::cst::function::ParameterDefault;
use crate::cst::function::ParameterList;
use crate::cst::function::ReturnType;
use crate::cst::function::ShortClosure;
use crate::cst::function::ShortClosureBody;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::kind::TokenKind;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_function(&mut self) -> Result<Function<'arena>, ParseError> {
        let attribute_lists = self.empty_slice();

        self.parse_function_with(attribute_lists)
    }

    pub(crate) fn parse_function_with(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Function<'arena>, ParseError> {
        let function = self.expect_keyword(TokenKind::Function)?;
        let name = self.parse_function_name()?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let parameter_list = self.parse_parameter_list()?;
        let return_type = self.parse_return_type()?;
        let body = self.parse_block()?;

        Ok(Function {
            attribute_lists,
            function,
            name,
            type_parameters,
            parameter_list,
            return_type,
            body,
        })
    }

    pub(crate) fn parse_closure(&mut self) -> Result<Closure<'arena>, ParseError> {
        let attribute_lists = self.empty_slice();

        self.parse_closure_with(attribute_lists)
    }

    pub(crate) fn parse_closure_with(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<Closure<'arena>, ParseError> {
        let function = self.expect_keyword(TokenKind::Function)?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let parameter_list = self.parse_parameter_list()?;
        let use_clause = if self.is_at(TokenKind::Use)? {
            Some(self.parse_closure_use_clause()?)
        } else {
            None
        };
        let return_type = self.parse_return_type()?;
        let body = self.parse_block()?;

        Ok(Closure {
            attribute_lists,
            function,
            type_parameters,
            parameter_list,
            use_clause,
            return_type,
            body,
        })
    }

    pub(crate) fn parse_short_closure(&mut self) -> Result<ShortClosure<'arena>, ParseError> {
        let attribute_lists = self.empty_slice();

        self.parse_short_closure_with(attribute_lists)
    }

    pub(crate) fn parse_short_closure_with(
        &mut self,
        attribute_lists: &'arena [AttributeList<'arena>],
    ) -> Result<ShortClosure<'arena>, ParseError> {
        let r#fn = self.expect_keyword(TokenKind::Fn)?;
        let type_parameters = self.parse_optional_type_parameter_list()?;
        let parameter_list = self.parse_parameter_list()?;
        let return_type = self.parse_return_type()?;
        let body = match self.peek_kind()? {
            Some(TokenKind::EqualGreaterThan) => {
                let arrow = self.expect_span(TokenKind::EqualGreaterThan)?;
                let expression = self.parse_expression_ref()?;

                ShortClosureBody::Expression { arrow, expression }
            }
            Some(TokenKind::LeftBrace) => ShortClosureBody::Block(self.parse_block()?),
            _ => {
                return Err(self.unexpected(Expected::Description(
                    "`=>` or a block after a short closure signature",
                )));
            }
        };

        Ok(ShortClosure {
            attribute_lists,
            r#fn,
            type_parameters,
            parameter_list,
            return_type,
            body,
        })
    }

    fn parse_closure_use_clause(&mut self) -> Result<ClosureUseClause<'arena>, ParseError> {
        let r#use = self.expect_keyword(TokenKind::Use)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        let variables =
            self.parse_comma_separated_until(TokenKind::RightParenthesis, Self::parse_variable)?;

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(ClosureUseClause {
            r#use,
            left_parenthesis,
            variables,
            right_parenthesis,
        })
    }

    pub(crate) fn parse_parameter_list(&mut self) -> Result<ParameterList<'arena>, ParseError> {
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        let parameters =
            self.parse_comma_separated_until(TokenKind::RightParenthesis, Self::parse_parameter)?;

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(ParameterList {
            left_parenthesis,
            parameters,
            right_parenthesis,
        })
    }

    fn parse_parameter(&mut self) -> Result<Parameter<'arena>, ParseError> {
        let attribute_lists = self.parse_attribute_lists()?;
        let modifiers = self.parse_modifiers()?;

        let r#type = if self.is_at(TokenKind::Variable)? {
            None
        } else {
            Some(self.parse_type()?)
        };

        let variable = self.parse_variable()?;

        let default = if let Some(equals) = self.eat_optional(TokenKind::Equal)? {
            let value = self.parse_expression_ref()?;

            Some(ParameterDefault { equals, value })
        } else {
            None
        };

        Ok(Parameter {
            attribute_lists,
            modifiers,
            r#type,
            variable,
            default,
        })
    }

    pub(crate) fn parse_return_type(&mut self) -> Result<Option<ReturnType<'arena>>, ParseError> {
        if let Some(colon) = self.eat_optional(TokenKind::Colon)? {
            let r#type = self.parse_type()?;

            Ok(Some(ReturnType { colon, r#type }))
        } else {
            Ok(None)
        }
    }
}
