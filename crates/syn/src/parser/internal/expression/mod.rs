//! The expression parser: a Pratt (precedence-climbing) parser.

mod array;
mod call;

use std::mem;

use crate::unreachable_invariant;

use whim_span::Span;

use crate::arena::Arena;
use crate::arena::Vec;
use crate::cst::access::Access;
use crate::cst::access::ClassReference;
use crate::cst::access::ConstantAccess;
use crate::cst::access::NullSafePropertyAccess;
use crate::cst::access::PropertyAccess;
use crate::cst::array::ArrayAccess;
use crate::cst::array::ArrayAppend;
use crate::cst::atom::Identifier;
use crate::cst::atom::LocalIdentifier;
use crate::cst::call::ArgumentList;
use crate::cst::call::Call;
use crate::cst::call::MethodCall;
use crate::cst::call::MethodPartialApplication;
use crate::cst::call::NullSafeMethodCall;
use crate::cst::call::PartialApplication;
use crate::cst::call::PartialArgumentList;
use crate::cst::expression::Break;
use crate::cst::expression::Continue;
use crate::cst::expression::Expression;
use crate::cst::expression::InterpolatedString;
use crate::cst::expression::InterpolatedStringExpression;
use crate::cst::expression::InterpolatedStringLiteral;
use crate::cst::expression::InterpolatedStringPart;
use crate::cst::expression::Return;
use crate::cst::expression::Throw;
use crate::cst::operation::Assignment;
use crate::cst::operation::AssignmentOperator;
use crate::cst::operation::AssignmentTarget;
use crate::cst::operation::Binary;
use crate::cst::operation::BinaryOperator;
use crate::cst::operation::TypeOperation;
use crate::cst::operation::TypeOperator;
use crate::cst::operation::UnaryPostfix;
use crate::cst::operation::UnaryPostfixOperator;
use crate::cst::operation::UnaryPrefix;
use crate::cst::operation::UnaryPrefixOperator;
use crate::error::Expected;
use crate::error::ParseError;
use crate::error::StringLiteralError;
use crate::parser::Parser;
use crate::token::Token;
use crate::token::kind::TokenKind;
use crate::token::precedence::Associativity;
use crate::token::precedence::GetPrecedence;
use crate::token::precedence::Precedence;
use crate::utils::double_quoted_escape_length;
use crate::utils::parse_literal_string_detailed_in;

/// The result of parsing a `(...)` argument list, which may or may not turn
/// the surrounding call into a partial application.
enum ArgumentTail<'arena> {
    Full(ArgumentList<'arena>),
    Partial(PartialArgumentList<'arena>),
}

impl<'input, 'arena, A> Parser<'input, 'arena, A>
where
    'input: 'arena,
    A: Arena,
{
    pub(crate) fn parse_expression(&mut self) -> Result<Expression<'arena>, ParseError> {
        let saved = mem::replace(&mut self.no_as, false);
        let expression = self.parse_expression_bp(Precedence::Lowest);
        self.no_as = saved;

        expression
    }

    pub(crate) fn parse_expression_ref(
        &mut self,
    ) -> Result<&'arena Expression<'arena>, ParseError> {
        Ok(self.arena.alloc(self.parse_expression()?))
    }

    pub(crate) fn parse_expression_bp(
        &mut self,
        min: Precedence,
    ) -> Result<Expression<'arena>, ParseError> {
        self.enter()?;
        let result = self.parse_expression_bp_inner(min);
        self.leave();

        result
    }

    fn parse_expression_bp_ref(
        &mut self,
        min: Precedence,
    ) -> Result<&'arena Expression<'arena>, ParseError> {
        let expression = self.parse_expression_bp(min)?;

        Ok(self.arena.alloc(expression))
    }

    fn parse_expression_bp_inner(
        &mut self,
        min: Precedence,
    ) -> Result<Expression<'arena>, ParseError> {
        let mut left = self.parse_prefix()?;

        while let Some(kind) = self.peek_kind()? {
            if kind.is_postfix() {
                left = self.parse_postfix(left, kind)?;
                continue;
            }

            if kind == TokenKind::As && self.no_as {
                break;
            }

            let operator_precedence = Precedence::infix(&kind);
            if matches!(operator_precedence, Precedence::Lowest) {
                break;
            }

            let attaches = match operator_precedence.associativity() {
                Some(Associativity::Right) => operator_precedence >= min,
                _ => operator_precedence > min,
            };
            if !attaches {
                break;
            }

            if operator_precedence.is_non_associative()
                && top_precedence(&left) == operator_precedence
            {
                let Some(token) = self.peek()? else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("the peeked kind implies a token") }
                };
                let expected = if operator_precedence == Precedence::TypeOperation {
                    "parentheses around the chained type operation"
                } else {
                    "parentheses around the chained comparison"
                };

                return Err(ParseError::UnexpectedToken(
                    Expected::Description(expected),
                    token.kind,
                    token.compute_span(),
                ));
            }

            left = self.parse_infix(left, kind, operator_precedence)?;
        }

        Ok(left)
    }

    fn parse_prefix(&mut self) -> Result<Expression<'arena>, ParseError> {
        let Some(token) = self.peek()? else {
            return Err(self.unexpected(Expected::Description("an expression")));
        };

        if token.kind.is_unary_prefix() {
            return Ok(Expression::UnaryPrefix(UnaryPrefix {
                operator: unary_prefix_operator(self.consume()?),
                operand: self.parse_expression_bp_ref(Precedence::Unary)?,
            }));
        }

        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expression<'arena>, ParseError> {
        let Some(token) = self.peek()? else {
            return Err(self.unexpected(Expected::Description("an expression")));
        };

        if token.kind == TokenKind::Identifier
            && self
                .lookahead(1)?
                .is_some_and(|token| token.kind == TokenKind::Bang)
            && self
                .lookahead(2)?
                .is_some_and(|token| token.kind == TokenKind::LeftParenthesis)
        {
            return Ok(Expression::Construct(self.parse_construct()?));
        }

        let expression = match token.kind {
            TokenKind::StringPart => self.interpolated_string_expression()?,
            TokenKind::LiteralString
            | TokenKind::LiteralInteger
            | TokenKind::LiteralFloat
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null => {
                let token = self.consume()?;
                Expression::Literal(self.literal_of(token)?)
            }
            TokenKind::Variable => Expression::Variable(self.parse_variable()?),
            TokenKind::Vec
                if matches!(
                    self.lookahead(1)?.map(|token| token.kind),
                    Some(TokenKind::LeftBracket)
                ) =>
            {
                self.parse_vec_expression()?
            }
            TokenKind::Dict
                if matches!(
                    self.lookahead(1)?.map(|token| token.kind),
                    Some(TokenKind::LeftBracket)
                ) =>
            {
                Expression::Dict(self.parse_dict_expression()?)
            }
            TokenKind::LeftParenthesis => self.parse_parenthesized_or_tuple_expression()?,
            TokenKind::Match => return self.parse_match(),
            TokenKind::Function => Expression::Closure(self.parse_closure()?),
            TokenKind::Fn => Expression::ShortClosure(self.parse_short_closure()?),
            TokenKind::HashLeftBracket => return self.parse_attributed_expression(),
            TokenKind::New => Expression::Instantiation(self.parse_instantiation()?),
            TokenKind::Break => {
                let r#break = self.expect_keyword(TokenKind::Break)?;
                let level = if self.is_at(TokenKind::LiteralInteger)? {
                    Some(self.parse_integer_literal()?)
                } else {
                    None
                };

                Expression::Break(Break { r#break, level })
            }
            TokenKind::Continue => {
                let r#continue = self.expect_keyword(TokenKind::Continue)?;
                let level = if self.is_at(TokenKind::LiteralInteger)? {
                    Some(self.parse_integer_literal()?)
                } else {
                    None
                };

                Expression::Continue(Continue { r#continue, level })
            }
            TokenKind::Return => {
                let r#return = self.expect_keyword(TokenKind::Return)?;
                let value = if self.at_expression_start()? {
                    Some(self.parse_expression_ref()?)
                } else {
                    None
                };

                Expression::Return(Return { r#return, value })
            }
            TokenKind::Throw => {
                let throw = self.expect_keyword(TokenKind::Throw)?;
                let exception = self.parse_expression_ref()?;
                Expression::Throw(Throw { throw, exception })
            }
            TokenKind::Self_ => {
                let keyword = self.expect_keyword(TokenKind::Self_)?;
                return self.parse_static_access(ClassReference::Self_(keyword));
            }
            TokenKind::Parent => {
                let keyword = self.expect_keyword(TokenKind::Parent)?;
                return self.parse_static_access(ClassReference::Parent(keyword));
            }
            TokenKind::Static => {
                let keyword = self.expect_keyword(TokenKind::Static)?;
                return self.parse_static_access(ClassReference::Static(keyword));
            }
            TokenKind::Identifier
            | TokenKind::QualifiedIdentifier
            | TokenKind::FullyQualifiedIdentifier => {
                Expression::Access(Access::Constant(ConstantAccess {
                    name: self.parse_identifier()?,
                }))
            }
            kind if kind.is_constant_name() => self.parse_keyword_name()?,
            kind if kind.is_function_name()
                && matches!(
                    self.lookahead(1)?.map(|token| token.kind),
                    Some(TokenKind::LeftParenthesis)
                ) =>
            {
                self.parse_keyword_name()?
            }
            _ => {
                return Err(ParseError::UnexpectedToken(
                    Expected::Description("an expression"),
                    token.kind,
                    token.compute_span(),
                ));
            }
        };

        Ok(expression)
    }

    fn at_expression_start(&mut self) -> Result<bool, ParseError> {
        let Some(token) = self.peek()? else {
            return Ok(false);
        };

        let kind = token.kind;
        if kind == TokenKind::Semicolon {
            return Ok(false);
        }

        if kind.is_unary_prefix() || kind.is_constant_name() {
            return Ok(true);
        }

        if kind.is_function_name()
            && self
                .lookahead(1)?
                .is_some_and(|next| next.kind == TokenKind::LeftParenthesis)
        {
            return Ok(true);
        }

        Ok(matches!(
            kind,
            TokenKind::StringPart
                | TokenKind::LiteralString
                | TokenKind::LiteralInteger
                | TokenKind::LiteralFloat
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Variable
                | TokenKind::LeftParenthesis
                | TokenKind::Match
                | TokenKind::Function
                | TokenKind::Fn
                | TokenKind::HashLeftBracket
                | TokenKind::New
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Return
                | TokenKind::Throw
                | TokenKind::Self_
                | TokenKind::Parent
                | TokenKind::Static
                | TokenKind::Identifier
                | TokenKind::QualifiedIdentifier
                | TokenKind::FullyQualifiedIdentifier
        ))
    }

    fn interpolated_string_expression(&mut self) -> Result<Expression<'arena>, ParseError> {
        let first = self.expect(TokenKind::StringPart)?;
        if !first.value.starts_with('"') {
            return Err(ParseError::UnexpectedToken(
                Expected::Description("the start of an interpolated string"),
                first.kind,
                first.compute_span(),
            ));
        }

        let opening_quote = Span::new(first.start, first.start.forward(1));
        let mut parts = Vec::new_in(self.arena);
        let mut literal = first;
        let mut first_part = true;

        loop {
            let (literal_part, closing_quote) =
                self.interpolated_literal_part(literal, first_part)?;
            parts.push(InterpolatedStringPart::Literal(literal_part));
            if let Some(closing_quote) = closing_quote {
                return Ok(Expression::InterpolatedString(InterpolatedString {
                    opening_quote,
                    parts: parts.leak(),
                    closing_quote,
                }));
            }

            match self.peek_kind()? {
                Some(TokenKind::Variable) => {
                    parts.push(InterpolatedStringPart::Variable(self.parse_variable()?));
                }
                Some(TokenKind::LeftBrace) => {
                    let left_brace = self.expect_span(TokenKind::LeftBrace)?;
                    let expression = self.parse_expression_ref()?;
                    let right_brace = self.expect_span(TokenKind::RightBrace)?;
                    parts.push(InterpolatedStringPart::Expression(
                        InterpolatedStringExpression {
                            left_brace,
                            expression,
                            right_brace,
                        },
                    ));
                }
                Some(found) => {
                    let Some(token) = self.peek()? else {
                        // SAFETY: the surrounding invariant makes this path unreachable.
                        unsafe { unreachable_invariant("a token kind came from a token") }
                    };
                    return Err(ParseError::UnexpectedToken(
                        Expected::Description("an interpolation"),
                        found,
                        token.compute_span(),
                    ));
                }
                None => {
                    return Err(self.unexpected(Expected::Description(
                        "the remainder of an interpolated string",
                    )));
                }
            }

            literal = self.expect(TokenKind::StringPart)?;
            first_part = false;
        }
    }

    fn interpolated_literal_part(
        &self,
        token: Token<'input>,
        first: bool,
    ) -> Result<(InterpolatedStringLiteral<'arena>, Option<Span>), ParseError> {
        let starts_after_quote = usize::from(first);
        let closes =
            (!first || token.value.len() > 1) && ends_with_unescaped_quote(token.value.as_bytes());
        let ends_before_quote = usize::from(closes);
        let content_end = token.value.len().saturating_sub(ends_before_quote);
        let raw = &token.value[starts_after_quote..content_end];
        let start = token.start.forward(starts_after_quote as u32);

        if let Some(offset) = unescaped_closing_brace(raw.as_bytes()) {
            let brace = start.forward(offset as u32);
            return Err(ParseError::InvalidStringLiteral(
                StringLiteralError::UnescapedClosingBrace,
                Span::new(brace, brace.forward(1)),
            ));
        }

        let span = Span::new(start, start.forward(raw.len() as u32));
        let value = parse_literal_string_detailed_in(self.arena, raw.as_bytes(), Some(b'"'), false)
            .map_err(|error| ParseError::InvalidStringLiteral(error, span))?;
        let closing_quote = closes.then(|| {
            let quote = token.start.forward((token.value.len() - 1) as u32);
            Span::new(quote, quote.forward(1))
        });

        Ok((
            InterpolatedStringLiteral {
                span,
                raw: self.arena.alloc_str(raw),
                value,
            },
            closing_quote,
        ))
    }

    fn parse_keyword_name(&mut self) -> Result<Expression<'arena>, ParseError> {
        let token = self.consume()?;

        Ok(Expression::Access(Access::Constant(ConstantAccess {
            name: Identifier::Local(LocalIdentifier {
                span: token.compute_span(),
                value: self.arena.alloc_str(token.value),
            }),
        })))
    }

    fn parse_postfix(
        &mut self,
        left: Expression<'arena>,
        kind: TokenKind,
    ) -> Result<Expression<'arena>, ParseError> {
        let expression = match kind {
            TokenKind::LeftParenthesis => self.parse_function_call(left, None)?,
            TokenKind::ColonColonLessThan => {
                let type_arguments = self.parse_optional_turbofish()?;

                if self.is_at(TokenKind::LeftParenthesis)? {
                    self.parse_function_call(left, type_arguments)?
                } else if self.is_at(TokenKind::ColonColon)? {
                    let class = self.class_reference_of(left, type_arguments);
                    let double_colon = self.expect_span(TokenKind::ColonColon)?;

                    return self.parse_static_member(class, double_colon);
                } else {
                    return Err(self.unexpected(Expected::Description(
                        "`(` or `::` after a turbofish type-argument list",
                    )));
                }
            }
            TokenKind::LeftBracket => {
                let left_bracket = self.expect_span(TokenKind::LeftBracket)?;

                if self.is_at(TokenKind::RightBracket)? {
                    let right_bracket = self.expect_span(TokenKind::RightBracket)?;

                    if matches!(
                        self.peek_kind()?,
                        Some(
                            TokenKind::Comma
                                | TokenKind::RightParenthesis
                                | TokenKind::EqualGreaterThan
                        )
                    ) {
                        Expression::ArrayAppend(ArrayAppend {
                            array: self.arena.alloc(left),
                            left_bracket,
                            right_bracket,
                        })
                    } else {
                        return self.parse_append_assignment(left, left_bracket, right_bracket);
                    }
                } else {
                    let index = self.parse_expression_ref()?;
                    let right_bracket = self.expect_span(TokenKind::RightBracket)?;

                    Expression::ArrayAccess(ArrayAccess {
                        array: self.arena.alloc(left),
                        left_bracket,
                        index,
                        right_bracket,
                    })
                }
            }
            TokenKind::MinusGreaterThan => {
                let arrow = self.expect_span(TokenKind::MinusGreaterThan)?;
                let method = self.parse_member_name()?;
                let type_arguments = self.parse_optional_turbofish()?;

                if self.is_at(TokenKind::LeftParenthesis)? {
                    match self.parse_argument_tail()? {
                        ArgumentTail::Full(argument_list) => {
                            Expression::Call(Call::Method(MethodCall {
                                object: self.arena.alloc(left),
                                arrow,
                                method,
                                type_arguments,
                                argument_list,
                            }))
                        }
                        ArgumentTail::Partial(argument_list) => Expression::PartialApplication(
                            PartialApplication::Method(MethodPartialApplication {
                                object: self.arena.alloc(left),
                                arrow,
                                method,
                                type_arguments,
                                argument_list,
                            }),
                        ),
                    }
                } else if type_arguments.is_some() {
                    return Err(self
                        .unexpected(Expected::Description("an argument list after a turbofish")));
                } else {
                    Expression::Access(Access::Property(PropertyAccess {
                        object: self.arena.alloc(left),
                        arrow,
                        property: method,
                    }))
                }
            }
            TokenKind::QuestionMinusGreaterThan => {
                let question_mark_arrow = self.expect_span(TokenKind::QuestionMinusGreaterThan)?;
                let method = self.parse_member_name()?;
                let type_arguments = self.parse_optional_turbofish()?;

                if self.is_at(TokenKind::LeftParenthesis)? {
                    match self.parse_argument_tail()? {
                        ArgumentTail::Full(argument_list) => {
                            Expression::Call(Call::NullSafeMethod(NullSafeMethodCall {
                                object: self.arena.alloc(left),
                                question_mark_arrow,
                                method,
                                type_arguments,
                                argument_list,
                            }))
                        }
                        ArgumentTail::Partial(_) => {
                            return Err(ParseError::UnexpectedToken(
                                Expected::Description(
                                    "arguments (a null-safe call cannot be a partial application)",
                                ),
                                TokenKind::LeftParenthesis,
                                question_mark_arrow,
                            ));
                        }
                    }
                } else if type_arguments.is_some() {
                    return Err(self
                        .unexpected(Expected::Description("an argument list after a turbofish")));
                } else {
                    Expression::Access(Access::NullSafeProperty(NullSafePropertyAccess {
                        object: self.arena.alloc(left),
                        question_mark_arrow,
                        property: method,
                    }))
                }
            }
            TokenKind::ColonColon => {
                let double_colon = self.expect_span(TokenKind::ColonColon)?;
                let class = self.class_reference_of(left, None);

                return self.parse_static_member(class, double_colon);
            }
            TokenKind::PlusPlus | TokenKind::MinusMinus => Expression::UnaryPostfix(UnaryPostfix {
                operand: self.arena.alloc(left),
                operator: unary_postfix_operator(self.consume()?),
            }),
            _ => {
                let span = self.stream_span()?;

                return Err(ParseError::UnexpectedToken(
                    Expected::Description("a postfix operator"),
                    kind,
                    span,
                ));
            }
        };

        Ok(expression)
    }

    fn parse_infix(
        &mut self,
        left: Expression<'arena>,
        kind: TokenKind,
        precedence: Precedence,
    ) -> Result<Expression<'arena>, ParseError> {
        match kind {
            TokenKind::Is => {
                return Ok(Expression::TypeOperation(TypeOperation {
                    operand: self.arena.alloc(left),
                    operator: TypeOperator::Check(self.expect_keyword(TokenKind::Is)?),
                    r#type: self.parse_type()?,
                }));
            }
            TokenKind::As => {
                return Ok(Expression::TypeOperation(TypeOperation {
                    operand: self.arena.alloc(left),
                    operator: TypeOperator::Assert(self.expect_keyword(TokenKind::As)?),
                    r#type: self.parse_type()?,
                }));
            }
            TokenKind::Question => {
                return Ok(Expression::TypeOperation(TypeOperation {
                    operand: self.arena.alloc(left),
                    operator: TypeOperator::AssertOrNull(
                        self.expect_span(TokenKind::Question)?,
                        self.expect_keyword(TokenKind::As)?,
                    ),
                    r#type: self.parse_type()?,
                }));
            }
            _ => {}
        }

        if kind.is_assignment() {
            return Ok(Expression::Assignment(Assignment {
                target: self.expression_to_assignment_target(&left)?,
                operator: assignment_operator(self.consume()?),
                value: self.parse_expression_bp_ref(precedence)?,
            }));
        }

        Ok(Expression::Binary(Binary {
            lhs: self.arena.alloc(left),
            operator: binary_operator(self.consume()?),
            rhs: self.parse_expression_bp_ref(precedence)?,
        }))
    }

    fn parse_append_assignment(
        &mut self,
        array: Expression<'arena>,
        left_bracket: Span,
        right_bracket: Span,
    ) -> Result<Expression<'arena>, ParseError> {
        Ok(Expression::Assignment(Assignment {
            target: AssignmentTarget::ArrayAppend(ArrayAppend {
                array: self.arena.alloc(array),
                left_bracket,
                right_bracket,
            }),
            operator: assignment_operator(self.expect(TokenKind::Equal)?),
            value: self.parse_expression_bp_ref(Precedence::Assignment)?,
        }))
    }
}

fn top_precedence(expression: &Expression<'_>) -> Precedence {
    match expression {
        Expression::Binary(binary) => binary.operator.precedence(),
        Expression::TypeOperation(_) => Precedence::TypeOperation,
        _ => Precedence::Lowest,
    }
}

fn ends_with_unescaped_quote(bytes: &[u8]) -> bool {
    if !bytes.ends_with(b"\"") {
        return false;
    }

    bytes[..bytes.len() - 1]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        .is_multiple_of(2)
}

fn unescaped_closing_brace(bytes: &[u8]) -> Option<usize> {
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor += double_quoted_escape_length(&bytes[cursor..]),
            b'}' => return Some(cursor),
            _ => cursor += 1,
        }
    }

    None
}

fn binary_operator(token: Token<'_>) -> BinaryOperator {
    let span = token.compute_span();

    match token.kind {
        TokenKind::Plus => BinaryOperator::Addition(span),
        TokenKind::Minus => BinaryOperator::Subtraction(span),
        TokenKind::Asterisk => BinaryOperator::Multiplication(span),
        TokenKind::Slash => BinaryOperator::Division(span),
        TokenKind::Percent => BinaryOperator::Modulo(span),
        TokenKind::AsteriskAsterisk => BinaryOperator::Exponentiation(span),
        TokenKind::Ampersand => BinaryOperator::BitwiseAnd(span),
        TokenKind::Pipe => BinaryOperator::BitwiseOr(span),
        TokenKind::Caret => BinaryOperator::BitwiseXor(span),
        TokenKind::LeftShift => BinaryOperator::LeftShift(span),
        TokenKind::RightShift => BinaryOperator::RightShift(span),
        TokenKind::QuestionQuestion => BinaryOperator::NullCoalesce(span),
        TokenKind::EqualEqual => BinaryOperator::Equal(span),
        TokenKind::BangEqual => BinaryOperator::NotEqual(span),
        TokenKind::LessThan => BinaryOperator::LessThan(span),
        TokenKind::LessThanEqual => BinaryOperator::LessThanOrEqual(span),
        TokenKind::GreaterThan => BinaryOperator::GreaterThan(span),
        TokenKind::GreaterThanEqual => BinaryOperator::GreaterThanOrEqual(span),
        TokenKind::LessThanEqualGreaterThan => BinaryOperator::Spaceship(span),
        TokenKind::PipeGreaterThan => BinaryOperator::Pipe(span),
        TokenKind::Dot => BinaryOperator::StringConcat(span),
        TokenKind::AmpersandAmpersand => BinaryOperator::And(span),
        TokenKind::PipePipe => BinaryOperator::Or(span),
        kind => unreachable!("not a binary operator: {kind:?}"),
    }
}

fn assignment_operator(token: Token<'_>) -> AssignmentOperator {
    let span = token.compute_span();

    match token.kind {
        TokenKind::Equal => AssignmentOperator::Assign(span),
        TokenKind::PlusEqual => AssignmentOperator::Addition(span),
        TokenKind::MinusEqual => AssignmentOperator::Subtraction(span),
        TokenKind::AsteriskEqual => AssignmentOperator::Multiplication(span),
        TokenKind::SlashEqual => AssignmentOperator::Division(span),
        TokenKind::PercentEqual => AssignmentOperator::Modulo(span),
        TokenKind::AsteriskAsteriskEqual => AssignmentOperator::Exponentiation(span),
        TokenKind::DotEqual => AssignmentOperator::Concat(span),
        TokenKind::AmpersandEqual => AssignmentOperator::BitwiseAnd(span),
        TokenKind::PipeEqual => AssignmentOperator::BitwiseOr(span),
        TokenKind::CaretEqual => AssignmentOperator::BitwiseXor(span),
        TokenKind::LeftShiftEqual => AssignmentOperator::LeftShift(span),
        TokenKind::RightShiftEqual => AssignmentOperator::RightShift(span),
        TokenKind::QuestionQuestionEqual => AssignmentOperator::Coalesce(span),
        TokenKind::AmpersandAmpersandEqual => AssignmentOperator::LogicalAnd(span),
        TokenKind::PipePipeEqual => AssignmentOperator::LogicalOr(span),
        kind => unreachable!("not an assignment operator: {kind:?}"),
    }
}

fn unary_prefix_operator(token: Token<'_>) -> UnaryPrefixOperator {
    let span = token.compute_span();

    match token.kind {
        TokenKind::Bang => UnaryPrefixOperator::Not(span),
        TokenKind::Tilde => UnaryPrefixOperator::BitwiseNot(span),
        TokenKind::Plus => UnaryPrefixOperator::Plus(span),
        TokenKind::Minus => UnaryPrefixOperator::Negation(span),
        TokenKind::PlusPlus => UnaryPrefixOperator::PreIncrement(span),
        TokenKind::MinusMinus => UnaryPrefixOperator::PreDecrement(span),
        kind => unreachable!("not a unary prefix operator: {kind:?}"),
    }
}

fn unary_postfix_operator(token: Token<'_>) -> UnaryPostfixOperator {
    let span = token.compute_span();

    match token.kind {
        TokenKind::PlusPlus => UnaryPostfixOperator::PostIncrement(span),
        TokenKind::MinusMinus => UnaryPostfixOperator::PostDecrement(span),
        kind => unreachable!("not a unary postfix operator: {kind:?}"),
    }
}
