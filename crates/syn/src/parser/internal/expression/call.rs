//! Parses calls, argument lists, instantiation, and class member access.

use crate::arena::Arena;
use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::access::Access;
use crate::cst::access::ClassConstantAccess;
use crate::cst::access::ClassReference;
use crate::cst::access::ConstantAccess;
use crate::cst::access::NamedClassReference;
use crate::cst::access::StaticPropertyAccess;
use crate::cst::call::ArgumentList;
use crate::cst::call::Call;
use crate::cst::call::Callee;
use crate::cst::call::FunctionCall;
use crate::cst::call::FunctionPartialApplication;
use crate::cst::call::NamedArgument;
use crate::cst::call::NamedPlaceholderArgument;
use crate::cst::call::PartialApplication;
use crate::cst::call::PartialArgument;
use crate::cst::call::PartialArgumentList;
use crate::cst::call::PlaceholderArgument;
use crate::cst::call::PositionalArgument;
use crate::cst::call::StaticMethodCall;
use crate::cst::call::StaticMethodPartialApplication;
use crate::cst::call::VariadicPlaceholderArgument;
use crate::cst::expression::Expression;
use crate::cst::expression::Instantiation;
use crate::cst::expression::Parenthesized;
use crate::cst::r#type::TypeArgumentList;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::parser::internal::expression::ArgumentTail;
use crate::token::kind::TokenKind;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    /// Parses `new ClassReference(args?)`.
    pub(in crate::parser::internal::expression) fn parse_instantiation(
        &mut self,
    ) -> Result<Instantiation<'arena>, ParseError> {
        let new = self.expect_keyword(TokenKind::New)?;
        let class = self.parse_class_reference()?;
        let argument_list = if self.is_at(TokenKind::LeftParenthesis)? {
            Some(self.parse_argument_list()?)
        } else {
            None
        };

        Ok(Instantiation {
            new,
            class,
            argument_list,
        })
    }

    fn parse_class_reference(&mut self) -> Result<ClassReference<'arena>, ParseError> {
        let Some(token) = self.peek()? else {
            return Err(self.unexpected(Expected::Description("a class")));
        };

        match token.kind {
            TokenKind::Self_ => Ok(ClassReference::Self_(
                self.expect_keyword(TokenKind::Self_)?,
            )),
            TokenKind::Parent => Ok(ClassReference::Parent(
                self.expect_keyword(TokenKind::Parent)?,
            )),
            TokenKind::Static => Ok(ClassReference::Static(
                self.expect_keyword(TokenKind::Static)?,
            )),
            TokenKind::Identifier
            | TokenKind::QualifiedIdentifier
            | TokenKind::FullyQualifiedIdentifier => {
                let identifier = self.parse_identifier()?;
                let type_arguments = self.parse_optional_turbofish()?;

                Ok(ClassReference::Named(NamedClassReference {
                    identifier,
                    type_arguments,
                }))
            }
            TokenKind::Variable => {
                let variable = self.parse_variable()?;
                Ok(ClassReference::Expression(
                    self.arena.alloc(Expression::Variable(variable)),
                ))
            }
            TokenKind::LeftParenthesis => {
                let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;
                let inner = self.parse_expression_ref()?;
                let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
                Ok(ClassReference::Expression(self.arena.alloc(
                    Expression::Parenthesized(Parenthesized {
                        left_parenthesis,
                        expression: inner,
                        right_parenthesis,
                    }),
                )))
            }
            _ => Err(ParseError::UnexpectedToken(
                Expected::Description("a class"),
                token.kind,
                token.compute_span(),
            )),
        }
    }

    /// Parses `self` / `parent` / `static` followed by `:: member`.
    pub(in crate::parser::internal::expression) fn parse_static_access(
        &mut self,
        class: ClassReference<'arena>,
    ) -> Result<Expression<'arena>, ParseError> {
        let double_colon = self.expect_span(TokenKind::ColonColon)?;

        self.parse_static_member(class, double_colon)
    }

    /// Parses the member after `::`: a static property (`::$name`), a class
    /// constant (`::NAME`), or a static method call (`::name(args)`).
    pub(in crate::parser::internal::expression) fn parse_static_member(
        &mut self,
        class: ClassReference<'arena>,
        double_colon: Span,
    ) -> Result<Expression<'arena>, ParseError> {
        if self.is_at(TokenKind::Variable)? {
            let property = self.parse_variable()?;

            return Ok(Expression::Access(Access::StaticProperty(
                StaticPropertyAccess {
                    class,
                    double_colon,
                    property,
                },
            )));
        }

        let member = self.parse_member_name()?;
        let type_arguments = self.parse_optional_turbofish()?;

        if self.is_at(TokenKind::LeftParenthesis)? {
            let expression = match self.parse_argument_tail()? {
                ArgumentTail::Full(argument_list) => {
                    Expression::Call(Call::StaticMethod(StaticMethodCall {
                        class,
                        double_colon,
                        method: member,
                        type_arguments,
                        argument_list,
                    }))
                }
                ArgumentTail::Partial(argument_list) => Expression::PartialApplication(
                    PartialApplication::StaticMethod(StaticMethodPartialApplication {
                        class,
                        double_colon,
                        method: member,
                        type_arguments,
                        argument_list,
                    }),
                ),
            };

            return Ok(expression);
        }

        if type_arguments.is_some() {
            return Err(
                self.unexpected(Expected::Description("an argument list after a turbofish"))
            );
        }

        Ok(Expression::Access(Access::ClassConstant(
            ClassConstantAccess {
                class,
                double_colon,
                constant: member,
            },
        )))
    }

    pub(crate) fn parse_argument_list(&mut self) -> Result<ArgumentList<'arena>, ParseError> {
        match self.parse_argument_tail()? {
            ArgumentTail::Full(list) => Ok(list),
            ArgumentTail::Partial(list) => Err(ParseError::UnexpectedToken(
                Expected::Description(
                    "arguments (a placeholder is only valid where a partial application is expected)",
                ),
                TokenKind::Question,
                list.span(),
            )),
        }
    }

    /// Parses a `(...)` argument list, classifying it as full or partial
    /// (first-class callable / partial application) based on placeholders.
    pub(in crate::parser::internal::expression) fn parse_argument_tail(
        &mut self,
    ) -> Result<ArgumentTail<'arena>, ParseError> {
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        let arguments = self.parse_comma_separated_until(
            TokenKind::RightParenthesis,
            Self::parse_partial_argument,
        )?;

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        let list = PartialArgumentList {
            left_parenthesis,
            arguments,
            right_parenthesis,
        };

        if list.has_placeholders() {
            Ok(ArgumentTail::Partial(list))
        } else {
            Ok(ArgumentTail::Full(list.into_argument_list(self.arena)))
        }
    }

    fn parse_partial_argument(&mut self) -> Result<PartialArgument<'arena>, ParseError> {
        if self.is_at(TokenKind::DotDotDot)? {
            let ellipsis = self.expect_span(TokenKind::DotDotDot)?;

            if !self.is_at(TokenKind::RightParenthesis)? {
                return Err(self.unexpected(Expected::kind(TokenKind::RightParenthesis)));
            }

            return Ok(PartialArgument::VariadicPlaceholder(
                VariadicPlaceholderArgument { span: ellipsis },
            ));
        }

        if self.peek_kind()?.is_some_and(|kind| kind.is_member_name())
            && matches!(self.lookahead(1)?.map(|t| t.kind), Some(TokenKind::Colon))
        {
            let name = self.parse_member_name()?;
            let colon = self.expect_span(TokenKind::Colon)?;

            if let Some(question_mark) = self.eat_optional(TokenKind::Question)? {
                return Ok(PartialArgument::NamedPlaceholder(
                    NamedPlaceholderArgument {
                        name,
                        colon,
                        question_mark,
                    },
                ));
            }

            let value = self.parse_expression_ref()?;

            return Ok(PartialArgument::Named(NamedArgument { name, colon, value }));
        }

        if let Some(span) = self.eat_optional(TokenKind::Question)? {
            return Ok(PartialArgument::Placeholder(PlaceholderArgument { span }));
        }

        let value = self.parse_expression_ref()?;

        Ok(PartialArgument::Positional(PositionalArgument { value }))
    }

    /// Builds a function call (or partial application) from `left` followed by
    /// a `(...)` argument list, carrying an optional turbofish.
    pub(in crate::parser::internal::expression) fn parse_function_call(
        &mut self,
        left: Expression<'arena>,
        type_arguments: Option<TypeArgumentList<'arena>>,
    ) -> Result<Expression<'arena>, ParseError> {
        Ok(match self.parse_argument_tail()? {
            ArgumentTail::Full(argument_list) => Expression::Call(Call::Function(FunctionCall {
                function: self.callee_of(left),
                type_arguments,
                argument_list,
            })),
            ArgumentTail::Partial(argument_list) => Expression::PartialApplication(
                PartialApplication::Function(FunctionPartialApplication {
                    function: self.callee_of(left),
                    type_arguments,
                    argument_list,
                }),
            ),
        })
    }

    fn callee_of(&self, expression: Expression<'arena>) -> Callee<'arena> {
        match &expression {
            Expression::Access(Access::Constant(ConstantAccess { name })) => {
                Callee::Identifier(*name)
            }
            _ => Callee::Expression(self.arena.alloc(expression)),
        }
    }

    /// Reinterprets a parsed expression as a class reference for `::` access,
    /// attaching an optional turbofish type-argument list.
    pub(in crate::parser::internal::expression) fn class_reference_of(
        &self,
        expression: Expression<'arena>,
        type_arguments: Option<TypeArgumentList<'arena>>,
    ) -> ClassReference<'arena> {
        match &expression {
            Expression::Access(Access::Constant(ConstantAccess { name })) => {
                ClassReference::Named(NamedClassReference {
                    identifier: *name,
                    type_arguments,
                })
            }
            _ => ClassReference::Expression(self.arena.alloc(expression)),
        }
    }

    /// Parses a turbofish type-argument list (`::<T, U>`) if one begins here.
    pub(in crate::parser::internal::expression) fn parse_optional_turbofish(
        &mut self,
    ) -> Result<Option<TypeArgumentList<'arena>>, ParseError> {
        if self.is_at(TokenKind::ColonColonLessThan)? {
            let token = self.consume()?;
            let less_than = Span::new(token.start.forward(2), token.start.forward(3));

            Ok(Some(self.parse_type_argument_list_body(less_than)?))
        } else {
            Ok(None)
        }
    }

    /// The span of the next token, for error reporting when one is required.
    pub(in crate::parser::internal::expression) fn stream_span(
        &mut self,
    ) -> Result<Span, ParseError> {
        if let Some(token) = self.peek()? {
            Ok(token.compute_span())
        } else {
            let position = self.stream.current_position();
            Ok(Span::new(position, position))
        }
    }
}
