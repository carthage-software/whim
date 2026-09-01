//! The language-construct parser.

use crate::unreachable_invariant;

use crate::arena::Arena;
use crate::arena::Vec;
use whim_span::Span;

use crate::cst::atom::Literal;
use crate::cst::atom::LocalIdentifier;
use crate::cst::construct::AssertConstruct;
use crate::cst::construct::AssertMessage;
use crate::cst::construct::CloneConstruct;
use crate::cst::construct::CloneField;
use crate::cst::construct::Construct;
use crate::cst::construct::ConstructArgument;
use crate::cst::construct::ContainsConstruct;
use crate::cst::construct::ContainsKeyConstruct;
use crate::cst::construct::DebugConstruct;
use crate::cst::construct::DirectoryConstruct;
use crate::cst::construct::DiscardConstruct;
use crate::cst::construct::DropConstruct;
use crate::cst::construct::EmbedConstruct;
use crate::cst::construct::ExitConstruct;
use crate::cst::construct::FileConstruct;
use crate::cst::construct::LengthConstruct;
use crate::cst::construct::PanicConstruct;
use crate::cst::construct::RemoveConstruct;
use crate::cst::construct::RemoveFirstConstruct;
use crate::cst::construct::RemoveLastConstruct;
use crate::cst::construct::RequireConstruct;
use crate::cst::construct::RequireOnceConstruct;
use crate::cst::construct::SwapRemoveConstruct;
use crate::cst::construct::WriteConstruct;
use crate::cst::construct::WriteErrorConstruct;
use crate::cst::construct::WriteErrorLineConstruct;
use crate::cst::construct::WriteLineConstruct;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::error::Expected;
use crate::error::ParseError;
use crate::parser::Parser;
use crate::token::kind::TokenKind;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_construct(&mut self) -> Result<Construct<'arena>, ParseError> {
        let Some(token) = self.peek()? else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the caller peeked a construct name token") }
        };

        Ok(match token.value {
            "require" => Construct::Require(self.parse_require_construct()?),
            "require_once" => Construct::RequireOnce(self.parse_require_once_construct()?),
            "length" => Construct::Length(self.parse_length_construct()?),
            "contains" => Construct::Contains(self.parse_contains_construct()?),
            "contains_key" => Construct::ContainsKey(self.parse_contains_key_construct()?),
            "clone" => Construct::Clone(self.parse_clone_construct()?),
            "remove" => Construct::Remove(self.parse_remove_construct()?),
            "swap_remove" => Construct::SwapRemove(self.parse_swap_remove_construct()?),
            "remove_first" => Construct::RemoveFirst(self.parse_remove_first_construct()?),
            "remove_last" => Construct::RemoveLast(self.parse_remove_last_construct()?),
            "assert" => Construct::Assert(self.parse_assert_construct()?),
            "exit" => Construct::Exit(self.parse_exit_construct()?),
            "panic" => Construct::Panic(self.parse_panic_construct()?),
            "write" => Construct::Write(self.parse_write_construct()?),
            "write_line" => Construct::WriteLine(self.parse_write_line_construct()?),
            "write_error" => Construct::WriteError(self.parse_write_error_construct()?),
            "write_error_line" => {
                Construct::WriteErrorLine(self.parse_write_error_line_construct()?)
            }
            "debug" => Construct::Debug(self.parse_debug_construct()?),
            "discard" => Construct::Discard(self.parse_discard_construct()?),
            "drop" => Construct::Drop(self.parse_drop_construct()?),
            "file" => Construct::File(self.parse_file_construct()?),
            "directory" => Construct::Directory(self.parse_directory_construct()?),
            "embed" => Construct::Embed(self.parse_embed_construct()?),
            _ => {
                return Err(ParseError::UnexpectedToken(
                    Expected::Description("a known language construct"),
                    token.kind,
                    token.compute_span(),
                ));
            }
        })
    }

    fn open_construct(&mut self) -> Result<(LocalIdentifier<'arena>, Span, Span), ParseError> {
        let name = self.parse_local_identifier()?;
        let bang = self.expect_span(TokenKind::Bang)?;
        let left_parenthesis = self.expect_span(TokenKind::LeftParenthesis)?;

        Ok((name, bang, left_parenthesis))
    }

    fn close_construct(&mut self) -> Result<(Option<Span>, Span), ParseError> {
        let trailing_comma = self.eat_optional(TokenKind::Comma)?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok((trailing_comma, right_parenthesis))
    }

    fn parse_require_construct(&mut self) -> Result<RequireConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let value = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(RequireConstruct {
            name,
            bang,
            left_parenthesis,
            value,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_embed_construct(&mut self) -> Result<EmbedConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let token = self.expect(TokenKind::LiteralString)?;
        let Literal::String(path) = self.literal_of(token)? else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a string token parses as a string literal") }
        };
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(EmbedConstruct {
            name,
            bang,
            left_parenthesis,
            path,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_require_once_construct(&mut self) -> Result<RequireOnceConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let value = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(RequireOnceConstruct {
            name,
            bang,
            left_parenthesis,
            value,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_length_construct(&mut self) -> Result<LengthConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let value = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(LengthConstruct {
            name,
            bang,
            left_parenthesis,
            value,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_contains_construct(&mut self) -> Result<ContainsConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let array = self.parse_expression_ref()?;
        let comma = self.expect_span(TokenKind::Comma)?;
        let value = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(ContainsConstruct {
            name,
            bang,
            left_parenthesis,
            array,
            comma,
            value,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_contains_key_construct(&mut self) -> Result<ContainsKeyConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let array = self.parse_expression_ref()?;
        let comma = self.expect_span(TokenKind::Comma)?;
        let key = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(ContainsKeyConstruct {
            name,
            bang,
            left_parenthesis,
            array,
            comma,
            key,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_clone_construct(&mut self) -> Result<CloneConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let object = self.parse_expression_ref()?;

        let mut fields = Vec::new_in(self.arena);
        let mut trailing_comma = None;
        while self.is_at(TokenKind::Comma)? {
            let comma = self.expect_span(TokenKind::Comma)?;
            if self.is_at(TokenKind::RightParenthesis)? {
                trailing_comma = Some(comma);
                break;
            }

            let field_name = self.parse_member_name()?;
            let colon = self.expect_span(TokenKind::Colon)?;
            let value = self.parse_expression_ref()?;
            fields.push(CloneField {
                comma,
                name: field_name,
                colon,
                value,
            });
        }

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(CloneConstruct {
            name,
            bang,
            left_parenthesis,
            object,
            fields: fields.leak(),
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_remove_construct(&mut self) -> Result<RemoveConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let array = self.parse_expression_ref()?;
        let comma = self.expect_span(TokenKind::Comma)?;
        let key = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(RemoveConstruct {
            name,
            bang,
            left_parenthesis,
            array,
            comma,
            key,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_swap_remove_construct(&mut self) -> Result<SwapRemoveConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let vector = self.parse_expression_ref()?;
        let comma = self.expect_span(TokenKind::Comma)?;
        let index = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(SwapRemoveConstruct {
            name,
            bang,
            left_parenthesis,
            vector,
            comma,
            index,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_remove_first_construct(&mut self) -> Result<RemoveFirstConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let array = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(RemoveFirstConstruct {
            name,
            bang,
            left_parenthesis,
            array,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_remove_last_construct(&mut self) -> Result<RemoveLastConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let array = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(RemoveLastConstruct {
            name,
            bang,
            left_parenthesis,
            array,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_assert_construct(&mut self) -> Result<AssertConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let condition = self.parse_expression_ref()?;

        let mut message = None;
        let mut trailing_comma = None;
        if self.is_at(TokenKind::Comma)? {
            let comma = self.expect_span(TokenKind::Comma)?;
            if self.is_at(TokenKind::RightParenthesis)? {
                trailing_comma = Some(comma);
            } else {
                let value = self.parse_expression_ref()?;
                message = Some(AssertMessage { comma, value });
                trailing_comma = self.eat_optional(TokenKind::Comma)?;
            }
        }

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(AssertConstruct {
            name,
            bang,
            left_parenthesis,
            condition,
            message,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_exit_construct(&mut self) -> Result<ExitConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;

        let code = if self.is_at(TokenKind::RightParenthesis)? {
            None
        } else {
            Some(self.parse_expression_ref()?)
        };

        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(ExitConstruct {
            name,
            bang,
            left_parenthesis,
            code,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_panic_construct(&mut self) -> Result<PanicConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let token = self.expect(TokenKind::LiteralString)?;
        let Literal::String(message) = self.literal_of(token)? else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a string token parses as a string literal") }
        };
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(PanicConstruct {
            name,
            bang,
            left_parenthesis,
            message,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_construct_arguments(
        &mut self,
    ) -> Result<TokenSeparatedSequence<'arena, ConstructArgument<'arena>>, ParseError> {
        self.parse_comma_separated_until(TokenKind::RightParenthesis, |parser| {
            Ok(ConstructArgument {
                value: parser.parse_expression_ref()?,
            })
        })
    }

    fn parse_write_construct(&mut self) -> Result<WriteConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let arguments = self.parse_construct_arguments()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(WriteConstruct {
            name,
            bang,
            left_parenthesis,
            arguments,
            right_parenthesis,
        })
    }

    fn parse_write_line_construct(&mut self) -> Result<WriteLineConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let arguments = self.parse_construct_arguments()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(WriteLineConstruct {
            name,
            bang,
            left_parenthesis,
            arguments,
            right_parenthesis,
        })
    }

    fn parse_write_error_construct(&mut self) -> Result<WriteErrorConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let arguments = self.parse_construct_arguments()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(WriteErrorConstruct {
            name,
            bang,
            left_parenthesis,
            arguments,
            right_parenthesis,
        })
    }

    fn parse_write_error_line_construct(
        &mut self,
    ) -> Result<WriteErrorLineConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let arguments = self.parse_construct_arguments()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(WriteErrorLineConstruct {
            name,
            bang,
            left_parenthesis,
            arguments,
            right_parenthesis,
        })
    }

    fn parse_debug_construct(&mut self) -> Result<DebugConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let arguments = self.parse_construct_arguments()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(DebugConstruct {
            name,
            bang,
            left_parenthesis,
            arguments,
            right_parenthesis,
        })
    }

    fn parse_drop_construct(&mut self) -> Result<DropConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let mut variables = Vec::new_in(self.arena);
        let mut commas = Vec::new_in(self.arena);

        variables.push(self.parse_variable()?);
        while self.is_at(TokenKind::Comma)? {
            commas.push(self.consume()?);
            if self.is_at(TokenKind::RightParenthesis)? {
                break;
            }
            variables.push(self.parse_variable()?);
        }

        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;
        Ok(DropConstruct {
            name,
            bang,
            left_parenthesis,
            variables: TokenSeparatedSequence::new(variables, commas),
            right_parenthesis,
        })
    }

    fn parse_discard_construct(&mut self) -> Result<DiscardConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let value = self.parse_expression_ref()?;
        let (trailing_comma, right_parenthesis) = self.close_construct()?;

        Ok(DiscardConstruct {
            name,
            bang,
            left_parenthesis,
            value,
            trailing_comma,
            right_parenthesis,
        })
    }

    fn parse_file_construct(&mut self) -> Result<FileConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(FileConstruct {
            name,
            bang,
            left_parenthesis,
            right_parenthesis,
        })
    }

    fn parse_directory_construct(&mut self) -> Result<DirectoryConstruct<'arena>, ParseError> {
        let (name, bang, left_parenthesis) = self.open_construct()?;
        let right_parenthesis = self.expect_span(TokenKind::RightParenthesis)?;

        Ok(DirectoryConstruct {
            name,
            bang,
            left_parenthesis,
            right_parenthesis,
        })
    }
}
