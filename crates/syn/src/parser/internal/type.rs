//! The type parser.

use crate::arena::Arena;
use crate::arena::Vec;
use whim_span::Span;

use crate::cst::atom::Literal;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::r#type::ArrayType;
use crate::cst::r#type::ClassnameType;
use crate::cst::r#type::DictShapeRest;
use crate::cst::r#type::DictShapeType;
use crate::cst::r#type::DictShapeTypeEntry;
use crate::cst::r#type::DictType;
use crate::cst::r#type::DictTypeArguments;
use crate::cst::r#type::FunctionType;
use crate::cst::r#type::FunctionTypeParameter;
use crate::cst::r#type::FunctionTypeSignature;
use crate::cst::r#type::IntegerRangeBound;
use crate::cst::r#type::IntegerRangeOperator;
use crate::cst::r#type::IntegerRangeType;
use crate::cst::r#type::IntersectionType;
use crate::cst::r#type::MemberType;
use crate::cst::r#type::NamedType;
use crate::cst::r#type::NegatedType;
use crate::cst::r#type::NegativeLiteralType;
use crate::cst::r#type::ParenthesizedType;
use crate::cst::r#type::SelfType;
use crate::cst::r#type::TrailingType;
use crate::cst::r#type::TupleType;
use crate::cst::r#type::Type;
use crate::cst::r#type::TypeArgument;
use crate::cst::r#type::TypeArgumentList;
use crate::cst::r#type::TypeParameter;
use crate::cst::r#type::TypeParameterBound;
use crate::cst::r#type::TypeParameterDefault;
use crate::cst::r#type::TypeParameterList;
use crate::cst::r#type::TypeVariance;
use crate::cst::r#type::UnionType;
use crate::cst::r#type::VecType;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::kind::TokenKind;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_type(&mut self) -> Result<&'arena Type<'arena>, ParseError> {
        self.enter()?;
        let result = self.parse_type_inner();
        self.leave();

        result
    }

    fn parse_type_inner(&mut self) -> Result<&'arena Type<'arena>, ParseError> {
        let mut left = self.parse_intersection_type()?;

        while self.is_at(TokenKind::Pipe)? {
            let pipe = self.expect_span(TokenKind::Pipe)?;
            let right = self.parse_intersection_type()?;
            left = self
                .arena
                .alloc(Type::Union(UnionType { left, pipe, right }));
        }

        Ok(left)
    }

    pub(crate) fn parse_intersection_type(&mut self) -> Result<&'arena Type<'arena>, ParseError> {
        let mut left = self.parse_negated_type()?;

        while self.is_at(TokenKind::Ampersand)? {
            let ampersand = self.expect_span(TokenKind::Ampersand)?;
            let right = self.parse_negated_type()?;
            left = self.arena.alloc(Type::Intersection(IntersectionType {
                left,
                ampersand,
                right,
            }));
        }

        Ok(left)
    }

    /// Parses unary negation without recursive descent through a long prefix.
    fn parse_negated_type(&mut self) -> Result<&'arena Type<'arena>, ParseError> {
        let mut prefixes = Vec::new_in(self.arena);
        while self.is_at(TokenKind::Bang)? {
            self.enter()?;
            prefixes.push(self.expect_span(TokenKind::Bang)?);
        }

        let r#type = self.parse_type_atom();
        for _ in 0..prefixes.len() {
            self.leave();
        }
        let mut r#type = r#type?;
        for bang in prefixes.into_iter().rev() {
            r#type = self
                .arena
                .alloc(Type::Negated(NegatedType { bang, r#type }));
        }

        Ok(r#type)
    }

    fn parse_type_atom(&mut self) -> Result<&'arena Type<'arena>, ParseError> {
        let Some(token) = self.peek()? else {
            return Err(self.unexpected(Expected::Description("a type")));
        };

        let r#type = match token.kind {
            TokenKind::LeftParenthesis => self.parse_parenthesized_or_tuple_type()?,
            TokenKind::Fn => Type::Function(self.parse_function_type()?),
            TokenKind::Array => Type::Array(self.parse_array_type()?),
            TokenKind::Vec => self.parse_vec_or_shape_type()?,
            TokenKind::Dict => self.parse_dict_or_shape_type()?,
            TokenKind::Classname => Type::Classname(self.parse_classname_type()?),
            TokenKind::Self_ => Type::Self_(self.parse_self_type()?),
            TokenKind::Parent => Type::Parent(self.expect_keyword(TokenKind::Parent)?),
            TokenKind::Static => Type::Static(self.expect_keyword(TokenKind::Static)?),
            TokenKind::String => Type::String(self.expect_keyword(TokenKind::String)?),
            TokenKind::Int => Type::Int(self.expect_keyword(TokenKind::Int)?),
            TokenKind::Float => Type::Float(self.expect_keyword(TokenKind::Float)?),
            TokenKind::Bool => Type::Bool(self.expect_keyword(TokenKind::Bool)?),
            TokenKind::Void => Type::Void(self.expect_keyword(TokenKind::Void)?),
            TokenKind::Mixed => Type::Mixed(self.expect_keyword(TokenKind::Mixed)?),
            TokenKind::Never => Type::Never(self.expect_keyword(TokenKind::Never)?),
            TokenKind::Object => Type::Object(self.expect_keyword(TokenKind::Object)?),
            TokenKind::Minus => self.parse_negative_numeric_type()?,
            TokenKind::LiteralInteger => {
                let bound = IntegerRangeBound::Positive(self.parse_integer_literal()?);
                self.parse_integer_literal_or_range(bound)?
            }
            TokenKind::DotDot | TokenKind::DotDotEqual => {
                Type::IntegerRange(self.parse_integer_range(None, true)?)
            }
            TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::LiteralFloat
            | TokenKind::LiteralString => {
                let token = self.consume()?;
                Type::Literal(self.literal_of(token)?)
            }
            TokenKind::Identifier
            | TokenKind::QualifiedIdentifier
            | TokenKind::FullyQualifiedIdentifier => Type::Named(self.parse_named_type()?),
            _ => {
                return Err(ParseError::UnexpectedToken(
                    Expected::Description("a type"),
                    token.kind,
                    token.compute_span(),
                ));
            }
        };

        Ok(self.arena.alloc(r#type))
    }

    fn parse_negative_numeric_type(&mut self) -> Result<Type<'arena>, ParseError> {
        let minus = self.expect_span(TokenKind::Minus)?;
        let Some(token) = self.peek()? else {
            return Err(self.unexpected(Expected::Description(
                "an integer or float literal after `-`",
            )));
        };
        match token.kind {
            TokenKind::LiteralInteger => {
                let literal = self.parse_integer_literal()?;
                self.parse_integer_literal_or_range(IntegerRangeBound::Negative { minus, literal })
            }
            TokenKind::LiteralFloat => {
                let token = self.consume()?;
                let Literal::Float(literal) = self.literal_of(token)? else {
                    unreachable!("the consumed token is a float literal");
                };

                Ok(Type::NegativeLiteral(NegativeLiteralType::Float {
                    minus,
                    literal,
                }))
            }
            _ => Err(ParseError::UnexpectedToken(
                Expected::Description("an integer or float literal after `-`"),
                token.kind,
                token.compute_span(),
            )),
        }
    }

    fn parse_integer_literal_or_range(
        &mut self,
        bound: IntegerRangeBound<'arena>,
    ) -> Result<Type<'arena>, ParseError> {
        if self.is_at(TokenKind::DotDot)? || self.is_at(TokenKind::DotDotEqual)? {
            return Ok(Type::IntegerRange(
                self.parse_integer_range(Some(bound), false)?,
            ));
        }

        Ok(match bound {
            IntegerRangeBound::Positive(literal) => Type::Literal(Literal::Integer(literal)),
            IntegerRangeBound::Negative { minus, literal } => {
                Type::NegativeLiteral(NegativeLiteralType::Integer { minus, literal })
            }
        })
    }

    fn parse_integer_range(
        &mut self,
        lower: Option<IntegerRangeBound<'arena>>,
        upper_required: bool,
    ) -> Result<IntegerRangeType<'arena>, ParseError> {
        let operator = if self.is_at(TokenKind::DotDotEqual)? {
            IntegerRangeOperator::Inclusive(self.expect_span(TokenKind::DotDotEqual)?)
        } else {
            IntegerRangeOperator::Exclusive(self.expect_span(TokenKind::DotDot)?)
        };
        let upper = if upper_required || self.integer_range_bound_starts()? {
            Some(self.parse_integer_range_bound()?)
        } else {
            None
        };

        Ok(IntegerRangeType {
            lower,
            operator,
            upper,
        })
    }

    fn integer_range_bound_starts(&mut self) -> Result<bool, ParseError> {
        Ok(matches!(
            self.peek()?.map(|token| token.kind),
            Some(TokenKind::Minus | TokenKind::LiteralInteger)
        ))
    }

    fn parse_integer_range_bound(&mut self) -> Result<IntegerRangeBound<'arena>, ParseError> {
        let minus = self.eat_optional(TokenKind::Minus)?;
        if !self.is_at(TokenKind::LiteralInteger)? {
            return Err(self.unexpected(Expected::Description("an integer literal range bound")));
        }
        let literal = self.parse_integer_literal()?;

        Ok(minus.map_or(IntegerRangeBound::Positive(literal), |minus| {
            IntegerRangeBound::Negative { minus, literal }
        }))
    }

    pub(crate) fn parse_named_type(&mut self) -> Result<NamedType<'arena>, ParseError> {
        let identifier = self.parse_identifier()?;
        let type_arguments = self.parse_optional_type_argument_list()?;
        let member = if self.is_at(TokenKind::ColonColon)? {
            let double_colon = self.expect_span(TokenKind::ColonColon)?;
            Some(MemberType {
                double_colon,
                name: self.parse_local_identifier()?,
                type_arguments: self.parse_optional_type_argument_list()?,
            })
        } else {
            None
        };

        Ok(NamedType {
            identifier,
            type_arguments,
            member,
        })
    }

    fn parse_self_type(&mut self) -> Result<SelfType<'arena>, ParseError> {
        let self_ = self.expect_keyword(TokenKind::Self_)?;
        let member = if self.is_at(TokenKind::ColonColon)? {
            let double_colon = self.expect_span(TokenKind::ColonColon)?;
            Some(MemberType {
                double_colon,
                name: self.parse_local_identifier()?,
                type_arguments: self.parse_optional_type_argument_list()?,
            })
        } else {
            None
        };

        Ok(SelfType { self_, member })
    }

    pub(crate) fn parse_optional_type_argument_list(
        &mut self,
    ) -> Result<Option<TypeArgumentList<'arena>>, ParseError> {
        if self.is_at(TokenKind::LessThan)? {
            Ok(Some(self.parse_type_argument_list()?))
        } else {
            Ok(None)
        }
    }

    /// Parses a `<T, U>` type-argument list. The opening `<` is either a bare
    /// `LessThan` (type position) or has already been consumed as part of a
    /// turbofish `::<` (expression position); the caller passes the opening
    /// span.
    fn parse_type_argument_list(&mut self) -> Result<TypeArgumentList<'arena>, ParseError> {
        let less_than = self.expect_span(TokenKind::LessThan)?;

        self.parse_type_argument_list_body(less_than)
    }

    /// Parses the body of a type-argument list after its opening `<` (span
    /// `less_than`) has been consumed. Shared by the bare and turbofish forms.
    pub(crate) fn parse_type_argument_list_body(
        &mut self,
        less_than: Span,
    ) -> Result<TypeArgumentList<'arena>, ParseError> {
        if self.is_at_type_list_close()? {
            return Err(self.unexpected(Expected::Description(
                "at least one type argument: an empty `<>` list is not allowed",
            )));
        }

        let mut arguments = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        while !self.is_at_type_list_close()? {
            let r#type = self.parse_type()?;
            arguments.push(TypeArgument { r#type });

            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
            } else {
                break;
            }
        }

        let greater_than = self.expect_type_list_close()?;

        Ok(TypeArgumentList {
            less_than,
            arguments: TokenSeparatedSequence::new(arguments, commas),
            greater_than,
        })
    }

    pub(crate) fn parse_optional_type_parameter_list(
        &mut self,
    ) -> Result<Option<TypeParameterList<'arena>>, ParseError> {
        if self.is_at(TokenKind::LessThan)? {
            Ok(Some(self.parse_type_parameter_list()?))
        } else {
            Ok(None)
        }
    }

    fn parse_type_parameter_list(&mut self) -> Result<TypeParameterList<'arena>, ParseError> {
        let less_than = self.expect_span(TokenKind::LessThan)?;

        if self.is_at_type_list_close()? {
            return Err(self.unexpected(Expected::Description(
                "at least one type parameter: an empty `<>` list is not allowed",
            )));
        }

        let mut parameters = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        while !self.is_at_type_list_close()? {
            parameters.push(self.parse_type_parameter()?);

            if self.is_at(TokenKind::Comma)? {
                commas.push(self.consume()?);
            } else {
                break;
            }
        }

        let greater_than = self.expect_type_list_close()?;

        Ok(TypeParameterList {
            less_than,
            parameters: TokenSeparatedSequence::new(parameters, commas),
            greater_than,
        })
    }

    fn parse_type_parameter(&mut self) -> Result<TypeParameter<'arena>, ParseError> {
        let variance = if self.is_at(TokenKind::In)? {
            Some(TypeVariance::In(self.expect_keyword(TokenKind::In)?))
        } else if self.is_at(TokenKind::Out)? {
            Some(TypeVariance::Out(self.expect_keyword(TokenKind::Out)?))
        } else {
            None
        };

        let name = self.parse_local_identifier()?;

        let bound = if let Some(colon) = self.eat_optional(TokenKind::Colon)? {
            let mut types = Vec::new_in(self.arena);
            let mut pluses = Vec::new_in(self.arena);
            types.push(self.parse_type()?);
            while self.is_at(TokenKind::Plus)? {
                pluses.push(self.consume()?);
                types.push(self.parse_type()?);
            }
            Some(TypeParameterBound {
                colon,
                types: TokenSeparatedSequence::new(types, pluses),
            })
        } else {
            None
        };

        let default = if let Some(equals) = self.eat_optional(TokenKind::Equal)? {
            Some(TypeParameterDefault {
                equals,
                r#type: self.parse_type()?,
            })
        } else {
            None
        };

        Ok(TypeParameter {
            variance,
            name,
            bound,
            default,
        })
    }

    fn parse_function_type(&mut self) -> Result<FunctionType<'arena>, ParseError> {
        let r#fn = self.expect_keyword(TokenKind::Fn)?;
        if !self.is_at(TokenKind::LeftParenthesis)? {
            return Ok(FunctionType {
                r#fn,
                signature: None,
            });
        }
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        let parameters = self.parse_comma_separated_until(
            TokenKind::RightParenthesis,
            Self::parse_function_type_parameter,
        )?;

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        let colon = self.expect_span(TokenKind::Colon)?;
        let return_type = self.parse_type()?;

        Ok(FunctionType {
            r#fn,
            signature: Some(FunctionTypeSignature {
                left_parenthesis,
                parameters,
                right_parenthesis,
                colon,
                return_type,
            }),
        })
    }

    fn parse_function_type_parameter(
        &mut self,
    ) -> Result<FunctionTypeParameter<'arena>, ParseError> {
        let equals = self.eat_optional(TokenKind::Equal)?;
        let r#type = self.parse_type()?;

        Ok(FunctionTypeParameter { equals, r#type })
    }

    fn parse_array_type(&mut self) -> Result<ArrayType<'arena>, ParseError> {
        let array = self.expect_keyword(TokenKind::Array)?;
        let type_arguments = self.parse_optional_type_argument_list()?;

        Ok(ArrayType {
            array,
            type_arguments,
        })
    }

    /// Parses a `vec` or `vec<T>` type. Bracketed forms are fixed-shape types.
    fn parse_vec_type(&mut self) -> Result<VecType<'arena>, ParseError> {
        let vec = self.expect_keyword(TokenKind::Vec)?;
        let type_arguments = self.parse_optional_type_argument_list()?;
        Ok(VecType {
            vec,
            type_arguments,
        })
    }

    fn parse_vec_or_shape_type(&mut self) -> Result<Type<'arena>, ParseError> {
        if self
            .lookahead(1)?
            .is_some_and(|token| token.kind == TokenKind::LeftBracket)
        {
            let vec = self.expect_keyword(TokenKind::Vec)?;
            let left_bracket = self.expect_span(TokenKind::LeftBracket)?;
            let mut elements = Vec::new_in(self.arena);
            let mut commas = Vec::new_in(self.arena);
            let mut trailing_type = None;
            while !self.is_at(TokenKind::RightBracket)? {
                if self.is_at(TokenKind::DotDotDot)? {
                    let ellipsis = self.consume()?.compute_span();
                    let r#type =
                        if self.is_at(TokenKind::Comma)? || self.is_at(TokenKind::RightBracket)? {
                            None
                        } else {
                            Some(self.parse_type()?)
                        };
                    trailing_type = Some(TrailingType { ellipsis, r#type });
                    if self.is_at(TokenKind::Comma)? {
                        commas.push(self.consume()?);
                    }
                    break;
                }
                elements.push(self.parse_type()?.clone());
                if self.is_at(TokenKind::Comma)? {
                    commas.push(self.consume()?);
                } else {
                    break;
                }
            }
            let right_bracket = self.expect_span(TokenKind::RightBracket)?;
            return Ok(Type::VecShape(crate::cst::r#type::VecShapeType {
                vec,
                left_bracket,
                elements: TokenSeparatedSequence::new(elements, commas),
                trailing_type,
                right_bracket,
            }));
        }
        Ok(Type::Vec(self.parse_vec_type()?))
    }

    /// Parses a `dict` or `dict<K, V>` type. Bracketed forms are fixed-shape dictionaries.
    fn parse_dict_type(&mut self) -> Result<DictType<'arena>, ParseError> {
        let dict = self.expect_keyword(TokenKind::Dict)?;
        let type_arguments = self.parse_optional_type_argument_list()?;
        Ok(DictType {
            dict,
            type_arguments,
        })
    }

    fn parse_dict_or_shape_type(&mut self) -> Result<Type<'arena>, ParseError> {
        if self
            .lookahead(1)?
            .is_some_and(|token| token.kind == TokenKind::LeftBracket)
        {
            let dict = self.expect_keyword(TokenKind::Dict)?;
            let left_bracket = self.expect_span(TokenKind::LeftBracket)?;
            let mut entries = Vec::new_in(self.arena);
            let mut commas = Vec::new_in(self.arena);
            let mut rest = None;
            while !self.is_at(TokenKind::RightBracket)? {
                if self.is_at(TokenKind::DotDotDot)? {
                    let ellipsis = self.consume()?.compute_span();
                    let less_than = self.expect_span(TokenKind::LessThan)?;
                    let key = self.parse_type()?;
                    let comma = self.expect_span(TokenKind::Comma)?;
                    let value = self.parse_type()?;
                    let greater_than = self.expect_type_list_close()?;
                    let trailing_comma = self.eat_optional(TokenKind::Comma)?;
                    rest = Some(DictShapeRest {
                        ellipsis,
                        type_arguments: DictTypeArguments {
                            less_than,
                            key,
                            comma,
                            value,
                            greater_than,
                        },
                        trailing_comma,
                    });
                    break;
                }

                let token = self.consume()?;
                if !matches!(
                    token.kind,
                    TokenKind::LiteralString | TokenKind::LiteralInteger
                ) {
                    return Err(ParseError::UnexpectedToken(
                        Expected::Description("a string or integer dictionary shape key"),
                        token.kind,
                        token.compute_span(),
                    ));
                }
                let key = self.literal_of(token)?;
                let double_arrow = self.expect_span(TokenKind::EqualGreaterThan)?;
                let value = self.parse_type()?;
                entries.push(DictShapeTypeEntry {
                    key,
                    double_arrow,
                    value,
                });
                if self.is_at(TokenKind::Comma)? {
                    commas.push(self.consume()?);
                } else {
                    break;
                }
            }
            let right_bracket = self.expect_span(TokenKind::RightBracket)?;
            return Ok(Type::DictShape(DictShapeType {
                dict,
                left_bracket,
                entries: TokenSeparatedSequence::new(entries, commas),
                rest,
                right_bracket,
            }));
        }
        Ok(Type::Dict(self.parse_dict_type()?))
    }

    fn parse_classname_type(&mut self) -> Result<ClassnameType<'arena>, ParseError> {
        let classname = self.expect_keyword(TokenKind::Classname)?;
        let less_than = self.expect_span(TokenKind::LessThan)?;
        let inner = self.parse_type()?;
        let greater_than = self.expect_type_list_close()?;

        Ok(ClassnameType {
            classname,
            less_than,
            inner,
            greater_than,
        })
    }

    /// Parses a `(...)` type: a parenthesized type `(T)`, a fixed tuple
    /// `(T,)` / `(T1, T2)`, or a rest tuple `(...T)` / `(T1, ...T)`. A bare
    /// rest means `...mixed`. `()` is an error.
    fn parse_parenthesized_or_tuple_type(&mut self) -> Result<Type<'arena>, ParseError> {
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        if self.is_at(TokenKind::RightParenthesis)? {
            return Err(self.unexpected(Expected::Description(
                "a type ( `()` is not a valid type; use `void` or `null` )",
            )));
        }

        if self.is_at(TokenKind::DotDotDot)? {
            let ellipsis = self.consume()?.compute_span();
            let r#type = if self.is_at(TokenKind::RightParenthesis)? {
                None
            } else {
                Some(self.parse_type()?)
            };
            if self.is_at(TokenKind::Comma)? {
                return Err(self.unexpected(Expected::Description(
                    "a tuple rest type must be the final tuple element",
                )));
            }
            let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
            return Ok(Type::Tuple(TupleType {
                left_parenthesis,
                elements: TokenSeparatedSequence::new(
                    Vec::new_in(self.arena),
                    Vec::new_in(self.arena),
                ),
                trailing_type: Some(TrailingType { ellipsis, r#type }),
                right_parenthesis,
            }));
        }

        let first = self.parse_type()?;

        if self.is_at(TokenKind::RightParenthesis)? {
            let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

            return Ok(Type::Parenthesized(ParenthesizedType {
                left_parenthesis,
                r#type: first,
                right_parenthesis,
            }));
        }

        let mut elements = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);
        elements.push(first.clone());
        let mut trailing_type = None;
        while self.is_at(TokenKind::Comma)? {
            commas.push(self.consume()?);

            if self.is_at(TokenKind::RightParenthesis)? {
                break;
            }

            if self.is_at(TokenKind::DotDotDot)? {
                let ellipsis = self.consume()?.compute_span();
                let r#type =
                    if self.is_at(TokenKind::Comma)? || self.is_at(TokenKind::RightParenthesis)? {
                        None
                    } else {
                        Some(self.parse_type()?)
                    };
                trailing_type = Some(TrailingType { ellipsis, r#type });
                if self.is_at(TokenKind::Comma)? {
                    return Err(self.unexpected(Expected::Description(
                        "a tuple rest type must be the final tuple element",
                    )));
                }
                break;
            }
            elements.push(self.parse_type()?.clone());
        }

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(Type::Tuple(TupleType {
            left_parenthesis,
            elements: TokenSeparatedSequence::new(elements, commas),
            trailing_type,
            right_parenthesis,
        }))
    }
}
