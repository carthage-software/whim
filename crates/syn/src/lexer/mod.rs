//! The Whim lexer.

mod internal;

use memchr::memmem;
use whim_span::Position;

use crate::arena::Arena;
use crate::arena::Vec as ArenaVec;
use crate::consts::IDENTIFIER_START_TABLE;
use crate::error::SyntaxError;
use crate::input::Input;
use crate::token::Token;
use crate::token::kind::TokenKind;
use crate::utils::double_quoted_escape_length;

/// Single-byte tokens that never combine into a multi-byte token.
const SIMPLE_TOKEN_TABLE: [Option<TokenKind>; 256] = {
    let mut table: [Option<TokenKind>; 256] = [None; 256];
    table[b'(' as usize] = Some(TokenKind::LeftParenthesis);
    table[b')' as usize] = Some(TokenKind::RightParenthesis);
    table[b'[' as usize] = Some(TokenKind::LeftBracket);
    table[b']' as usize] = Some(TokenKind::RightBracket);
    table[b'{' as usize] = Some(TokenKind::LeftBrace);
    table[b'}' as usize] = Some(TokenKind::RightBrace);
    table[b',' as usize] = Some(TokenKind::Comma);
    table[b';' as usize] = Some(TokenKind::Semicolon);
    table[b'~' as usize] = Some(TokenKind::Tilde);
    table[b'@' as usize] = Some(TokenKind::At);
    table
};

/// The Whim lexer.
#[derive(Debug)]
pub struct Lexer<'input, 'arena, A>
where
    A: Arena,
{
    input: Input<'input>,
    mode: LexerMode,
    modes: ArenaVec<'arena, LexerMode, A>,
}

#[derive(Debug, Clone, Copy)]
enum LexerMode {
    Code,
    String { opening: bool },
    VariableInterpolation,
    BracedInterpolationStart { end: usize },
    BracedInterpolation { end: usize },
}

impl<'input, 'arena, A> Lexer<'input, 'arena, A>
where
    A: Arena,
{
    #[must_use]
    pub const fn new(arena: &'arena A, input: Input<'input>) -> Self {
        Lexer {
            input,
            mode: LexerMode::Code,
            modes: ArenaVec::new_in(arena),
        }
    }

    #[inline]
    #[must_use]
    pub const fn has_reached_eof(&self) -> bool {
        self.input.has_reached_eof()
    }

    #[inline]
    #[must_use]
    pub const fn current_position(&self) -> Position {
        self.input.current_position()
    }

    /// `Some(Err(_))` is fatal: the caller must stop advancing. `None` means
    /// end of input.
    #[inline]
    pub fn advance(&mut self) -> Option<Result<Token<'input>, SyntaxError>> {
        match self.mode {
            LexerMode::String { opening } => return Some(self.advance_string(opening)),
            LexerMode::VariableInterpolation => {
                return Some(Ok(self.advance_interpolated_variable()));
            }
            LexerMode::BracedInterpolationStart { end } => {
                return Some(Ok(self.advance_interpolation_start(end)));
            }
            LexerMode::BracedInterpolation { end } if self.input.current_offset() == end => {
                return Some(Ok(self.advance_interpolation_end()));
            }
            LexerMode::Code | LexerMode::BracedInterpolation { .. } => {}
        }

        if self.input.has_reached_eof() {
            return None;
        }

        let start = self.input.current_position();

        if start.is_zero() && self.input.is_at(b"#!/", false) {
            let buffer = self.input.consume_through(b'\n');

            return Some(Ok(Token::new(TokenKind::Shebang, buffer, start)));
        }

        let whitespaces = self.input.consume_whitespaces();
        if !whitespaces.is_empty() {
            return Some(Ok(Token::new(TokenKind::Whitespace, whitespaces, start)));
        }

        let &first_byte = self.input.read(1).first()?;

        if let Some(kind) = SIMPLE_TOKEN_TABLE[first_byte as usize] {
            let buffer = self.input.consume(1);

            return Some(Ok(Token::new(kind, buffer, start)));
        }

        if IDENTIFIER_START_TABLE[first_byte as usize] {
            let (kind, length) = self.scan_identifier_or_keyword();
            let buffer = self.input.consume(length);

            return Some(Ok(Token::new(kind, buffer, start)));
        }

        if first_byte == b'$'
            && let Some(&next) = self.input.read(2).get(1)
            && IDENTIFIER_START_TABLE[next as usize]
        {
            let (identifier_length, _) = self.input.scan_identifier(1);
            let buffer = self.input.consume(1 + identifier_length);

            return Some(Ok(Token::new(TokenKind::Variable, buffer, start)));
        }

        let (kind, length) = match self.input.read(3) {
            [b'*', b'*', b'='] => (TokenKind::AsteriskAsteriskEqual, 3),
            [b'<', b'<', b'='] => (TokenKind::LeftShiftEqual, 3),
            [b'>', b'>', b'='] => (TokenKind::RightShiftEqual, 3),
            [b'<', b'=', b'>'] => (TokenKind::LessThanEqualGreaterThan, 3),
            [b'&', b'&', b'='] => (TokenKind::AmpersandAmpersandEqual, 3),
            [b'|', b'|', b'='] => (TokenKind::PipePipeEqual, 3),
            [b'?', b'?', b'='] => (TokenKind::QuestionQuestionEqual, 3),
            [b'?', b'-', b'>'] => (TokenKind::QuestionMinusGreaterThan, 3),
            [b':', b':', b'<'] => (TokenKind::ColonColonLessThan, 3),
            [b'.', b'.', b'.'] => (TokenKind::DotDotDot, 3),
            [b'.', b'.', b'='] => (TokenKind::DotDotEqual, 3),
            [b'/', b'/', ..] => {
                let remaining = self.input.read_remaining();
                let length = memchr::memchr2(b'\n', b'\r', remaining).unwrap_or(remaining.len());

                (TokenKind::SingleLineComment, length)
            }
            [b'/', b'*', ..] => {
                let remaining = self.input.read_remaining();
                if let Some(found) = memmem::find(&remaining[2..], b"*/") {
                    let length = 2 + found + 2;
                    let kind = if length > 4 && remaining[2] == b'*' {
                        TokenKind::DocBlockComment
                    } else {
                        TokenKind::MultiLineComment
                    };

                    (kind, length)
                } else {
                    self.input.consume_remaining();

                    return Some(Err(SyntaxError::UnclosedBlockComment(start)));
                }
            }
            [b'\'' | b'"', ..] => {
                let remaining = self.input.read_remaining();
                if let Some(length) = internal::string::scan(remaining) {
                    if remaining[0] == b'"'
                        && internal::string::needs_interpolation(&remaining[..length])
                    {
                        self.modes.push(self.mode);
                        self.mode = LexerMode::String { opening: true };

                        return Some(self.advance_string(true));
                    }

                    let raw = self.input.consume(length);
                    return Some(Ok(Token::new(TokenKind::LiteralString, raw, start)));
                }
                self.input.consume_remaining();

                return Some(Err(SyntaxError::UnclosedStringLiteral(start)));
            }
            [b'0'..=b'9', ..] => {
                if matches!(
                    self.input.read(2),
                    [b'0', b'X' | b'x' | b'O' | b'o' | b'B' | b'b']
                ) && !matches!(
                    self.input.read(3),
                    crate::start_of_hexadecimal_number!()
                        | crate::start_of_octal_number!()
                        | crate::start_of_binary_number!()
                ) {
                    self.input.skip(2);

                    return Some(Err(SyntaxError::MalformedNumericLiteral(start)));
                }

                let (kind, length) = internal::number::scan(&self.input);

                if matches!(kind, TokenKind::LiteralInteger)
                    && length > 1
                    && matches!(self.input.read(2), [b'0', b'0'..=b'9' | b'_'])
                {
                    self.input.skip(length);

                    return Some(Err(SyntaxError::LeadingZeroInIntegerLiteral(start)));
                }

                (kind, length)
            }
            [b'.', b'0'..=b'9', ..] => (
                TokenKind::LiteralFloat,
                internal::number::scan_leading_dot_float(&self.input),
            ),
            [b'\\', second, ..] if IDENTIFIER_START_TABLE[*second as usize] => {
                let (segment_length, _) = self.input.scan_identifier(1);
                let length = self.scan_qualified_tail(1 + segment_length);

                (TokenKind::FullyQualifiedIdentifier, length)
            }
            [b'\\', ..] => (TokenKind::NamespaceSeparator, 1),
            [b'=', b'=', ..] => (TokenKind::EqualEqual, 2),
            [b'=', b'>', ..] => (TokenKind::EqualGreaterThan, 2),
            [b'!', b'=', ..] => (TokenKind::BangEqual, 2),
            [b'&', b'&', ..] => (TokenKind::AmpersandAmpersand, 2),
            [b'&', b'=', ..] => (TokenKind::AmpersandEqual, 2),
            [b'|', b'|', ..] => (TokenKind::PipePipe, 2),
            [b'|', b'=', ..] => (TokenKind::PipeEqual, 2),
            [b'|', b'>', ..] => (TokenKind::PipeGreaterThan, 2),
            [b'^', b'=', ..] => (TokenKind::CaretEqual, 2),
            [b'+', b'+', ..] => (TokenKind::PlusPlus, 2),
            [b'+', b'=', ..] => (TokenKind::PlusEqual, 2),
            [b'-', b'-', ..] => (TokenKind::MinusMinus, 2),
            [b'-', b'=', ..] => (TokenKind::MinusEqual, 2),
            [b'-', b'>', ..] => (TokenKind::MinusGreaterThan, 2),
            [b'*', b'*', ..] => (TokenKind::AsteriskAsterisk, 2),
            [b'*', b'=', ..] => (TokenKind::AsteriskEqual, 2),
            [b'/', b'=', ..] => (TokenKind::SlashEqual, 2),
            [b'%', b'=', ..] => (TokenKind::PercentEqual, 2),
            [b'.', b'=', ..] => (TokenKind::DotEqual, 2),
            [b'?', b'?', ..] => (TokenKind::QuestionQuestion, 2),
            [b'<', b'<', ..] => (TokenKind::LeftShift, 2),
            [b'<', b'=', ..] => (TokenKind::LessThanEqual, 2),
            [b'>', b'>', ..] => (TokenKind::RightShift, 2),
            [b'>', b'=', ..] => (TokenKind::GreaterThanEqual, 2),
            [b':', b':', ..] => (TokenKind::ColonColon, 2),
            [b'#', b'[', ..] => (TokenKind::HashLeftBracket, 2),
            [b'.', b'.', ..] => (TokenKind::DotDot, 2),
            [b'=', ..] => (TokenKind::Equal, 1),
            [b'!', ..] => (TokenKind::Bang, 1),
            [b'&', ..] => (TokenKind::Ampersand, 1),
            [b'|', ..] => (TokenKind::Pipe, 1),
            [b'^', ..] => (TokenKind::Caret, 1),
            [b'+', ..] => (TokenKind::Plus, 1),
            [b'-', ..] => (TokenKind::Minus, 1),
            [b'*', ..] => (TokenKind::Asterisk, 1),
            [b'/', ..] => (TokenKind::Slash, 1),
            [b'%', ..] => (TokenKind::Percent, 1),
            [b'.', ..] => (TokenKind::Dot, 1),
            [b'<', ..] => (TokenKind::LessThan, 1),
            [b'>', ..] => (TokenKind::GreaterThan, 1),
            [b':', ..] => (TokenKind::Colon, 1),
            [b'?', ..] => (TokenKind::Question, 1),
            [b'$', ..] => (TokenKind::Dollar, 1),
            [unknown_byte, ..] => {
                let unknown_byte = *unknown_byte;
                self.input.skip(1);

                return Some(Err(SyntaxError::UnrecognizedByte(unknown_byte, start)));
            }
            [] => {
                return None;
            }
        };

        let buffer = self.input.consume(length);

        Some(Ok(Token::new(kind, buffer, start)))
    }

    fn advance_string(&mut self, opening: bool) -> Result<Token<'input>, SyntaxError> {
        let start = self.input.current_position();
        let remaining = self.input.read_remaining();
        let mut length = usize::from(opening);

        while length < remaining.len() {
            match remaining[length] {
                b'\\' => length += double_quoted_escape_length(&remaining[length..]),
                b'$' if remaining
                    .get(length + 1)
                    .is_some_and(|next| IDENTIFIER_START_TABLE[*next as usize]) =>
                {
                    self.mode = LexerMode::VariableInterpolation;
                    break;
                }
                b'{' => {
                    let Some(after_interpolation) =
                        internal::string::scan_interpolation(remaining, length + 1)
                    else {
                        self.input.consume_remaining();

                        return Err(SyntaxError::UnclosedStringLiteral(start));
                    };
                    self.mode = LexerMode::BracedInterpolationStart {
                        end: self.input.current_offset() + after_interpolation - 1,
                    };
                    break;
                }
                b'"' => {
                    length += 1;
                    self.mode = self.modes.pop().unwrap_or(LexerMode::Code);
                    break;
                }
                _ => length += 1,
            }
        }

        let buffer = self.input.consume(length);

        Ok(Token::new(TokenKind::StringPart, buffer, start))
    }

    fn advance_interpolated_variable(&mut self) -> Token<'input> {
        let start = self.input.current_position();
        let (identifier_length, _) = self.input.scan_identifier(1);
        let buffer = self.input.consume(1 + identifier_length);
        self.mode = LexerMode::String { opening: false };

        Token::new(TokenKind::Variable, buffer, start)
    }

    fn advance_interpolation_start(&mut self, end: usize) -> Token<'input> {
        let start = self.input.current_position();
        let buffer = self.input.consume(1);
        self.mode = LexerMode::BracedInterpolation { end };

        Token::new(TokenKind::LeftBrace, buffer, start)
    }

    fn advance_interpolation_end(&mut self) -> Token<'input> {
        let start = self.input.current_position();
        let buffer = self.input.consume(1);
        self.mode = LexerMode::String { opening: false };

        Token::new(TokenKind::RightBrace, buffer, start)
    }

    #[inline]
    fn scan_identifier_or_keyword(&self) -> (TokenKind, usize) {
        let (segment_length, _) = self.input.scan_identifier(0);
        let length = self.scan_qualified_tail(segment_length);

        if length == segment_length {
            return internal::keyword::lookup(self.input.read(segment_length))
                .map_or((TokenKind::Identifier, segment_length), |kind| {
                    (kind, segment_length)
                });
        }

        (TokenKind::QualifiedIdentifier, length)
    }

    /// Extends `length` past any `\segment` continuations of a qualified
    /// identifier. Each continuation requires a `\` followed by a valid
    /// identifier **start** byte; a trailing `\` is never consumed.
    #[inline]
    fn scan_qualified_tail(&self, mut length: usize) -> usize {
        loop {
            match self.input.peek(length, 2) {
                [b'\\', next] if IDENTIFIER_START_TABLE[*next as usize] => {
                    let (segment_length, _) = self.input.scan_identifier(length + 1);
                    length += 1 + segment_length;
                }
                _ => return length,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::arena::LocalArena;
    use crate::lexer::*;

    fn lex(source: &str) -> Vec<Token<'_>> {
        let arena = LocalArena::new();
        let mut lexer = Lexer::new(&arena, Input::new(source));
        let mut tokens = Vec::new();
        while let Some(result) = lexer.advance() {
            tokens.push(result.expect("expected lexing to succeed"));
        }

        tokens
    }

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source)
            .into_iter()
            .map(|token| token.kind)
            .filter(|kind| !kind.is_trivia())
            .collect()
    }

    fn first_error(source: &str) -> SyntaxError {
        let arena = LocalArena::new();
        let mut lexer = Lexer::new(&arena, Input::new(source));
        while let Some(result) = lexer.advance() {
            if let Err(error) = result {
                return error;
            }
        }

        panic!("expected a syntax error");
    }

    #[test]
    fn shebang_at_file_start() {
        let tokens = lex("#!/usr/bin/env whim\n$x = 1;");
        assert_eq!(tokens[0].kind, TokenKind::Shebang);
        assert_eq!(tokens[0].value, "#!/usr/bin/env whim\n");
        assert_eq!(tokens[1].kind, TokenKind::Variable);
    }

    #[test]
    fn shebang_not_allowed_after_start() {
        let error = first_error("$x = 1;\n#!/usr/bin/env whim\n");
        assert!(matches!(error, SyntaxError::UnrecognizedByte(b'#', _)));
    }

    #[test]
    fn shebang_requires_slash() {
        let error = first_error("#!whim\n");
        assert!(matches!(error, SyntaxError::UnrecognizedByte(b'#', _)));
    }

    #[test]
    fn keywords_are_case_sensitive() {
        assert_eq!(
            kinds("class Class CLASS"),
            vec![
                TokenKind::Class,
                TokenKind::Identifier,
                TokenKind::Identifier
            ],
        );
        assert_eq!(
            kinds("true True TRUE"),
            vec![
                TokenKind::True,
                TokenKind::Identifier,
                TokenKind::Identifier
            ],
        );
    }

    #[test]
    fn all_keywords() {
        let cases: &[(&str, TokenKind)] = &[
            ("abstract", TokenKind::Abstract),
            ("array", TokenKind::Array),
            ("as", TokenKind::As),
            ("bool", TokenKind::Bool),
            ("break", TokenKind::Break),
            ("case", TokenKind::Case),
            ("catch", TokenKind::Catch),
            ("class", TokenKind::Class),
            ("classname", TokenKind::Classname),
            ("const", TokenKind::Const),
            ("continue", TokenKind::Continue),
            ("default", TokenKind::Default),
            ("dict", TokenKind::Dict),
            ("do", TokenKind::Do),
            ("else", TokenKind::Else),
            ("enum", TokenKind::Enum),
            ("extends", TokenKind::Extends),
            ("false", TokenKind::False),
            ("final", TokenKind::Final),
            ("finally", TokenKind::Finally),
            ("float", TokenKind::Float),
            ("fn", TokenKind::Fn),
            ("for", TokenKind::For),
            ("foreach", TokenKind::Foreach),
            ("function", TokenKind::Function),
            ("if", TokenKind::If),
            ("implements", TokenKind::Implements),
            ("in", TokenKind::In),
            ("int", TokenKind::Int),
            ("interface", TokenKind::Interface),
            ("is", TokenKind::Is),
            ("match", TokenKind::Match),
            ("using", TokenKind::Using),
            ("mixed", TokenKind::Mixed),
            ("namespace", TokenKind::Namespace),
            ("never", TokenKind::Never),
            ("new", TokenKind::New),
            ("null", TokenKind::Null),
            ("object", TokenKind::Object),
            ("out", TokenKind::Out),
            ("parent", TokenKind::Parent),
            ("private", TokenKind::Private),
            ("protected", TokenKind::Protected),
            ("public", TokenKind::Public),
            ("readonly", TokenKind::Readonly),
            ("return", TokenKind::Return),
            ("self", TokenKind::Self_),
            ("static", TokenKind::Static),
            ("string", TokenKind::String),
            ("throw", TokenKind::Throw),
            ("true", TokenKind::True),
            ("try", TokenKind::Try),
            ("type", TokenKind::Type),
            ("newtype", TokenKind::Newtype),
            ("use", TokenKind::Use),
            ("vec", TokenKind::Vec),
            ("void", TokenKind::Void),
            ("while", TokenKind::While),
        ];

        assert_eq!(cases.len(), 58);
        for (source, expected) in cases {
            let tokens = lex(source);
            assert_eq!(tokens.len(), 1, "{source}");
            assert_eq!(tokens[0].kind, *expected, "{source}");
            assert!(expected.is_keyword());
        }
    }

    #[test]
    fn operators() {
        let cases: &[(&str, TokenKind)] = &[
            ("=", TokenKind::Equal),
            ("+=", TokenKind::PlusEqual),
            ("-=", TokenKind::MinusEqual),
            ("*=", TokenKind::AsteriskEqual),
            ("/=", TokenKind::SlashEqual),
            ("%=", TokenKind::PercentEqual),
            ("**=", TokenKind::AsteriskAsteriskEqual),
            (".=", TokenKind::DotEqual),
            ("&=", TokenKind::AmpersandEqual),
            ("|=", TokenKind::PipeEqual),
            ("^=", TokenKind::CaretEqual),
            ("<<=", TokenKind::LeftShiftEqual),
            (">>=", TokenKind::RightShiftEqual),
            ("??=", TokenKind::QuestionQuestionEqual),
            ("&&=", TokenKind::AmpersandAmpersandEqual),
            ("||=", TokenKind::PipePipeEqual),
            ("==", TokenKind::EqualEqual),
            ("!=", TokenKind::BangEqual),
            ("<", TokenKind::LessThan),
            ("<=", TokenKind::LessThanEqual),
            (">", TokenKind::GreaterThan),
            (">=", TokenKind::GreaterThanEqual),
            ("<=>", TokenKind::LessThanEqualGreaterThan),
            ("&&", TokenKind::AmpersandAmpersand),
            ("||", TokenKind::PipePipe),
            ("|>", TokenKind::PipeGreaterThan),
            ("!", TokenKind::Bang),
            ("??", TokenKind::QuestionQuestion),
            ("&", TokenKind::Ampersand),
            ("|", TokenKind::Pipe),
            ("^", TokenKind::Caret),
            ("~", TokenKind::Tilde),
            ("<<", TokenKind::LeftShift),
            (">>", TokenKind::RightShift),
            ("+", TokenKind::Plus),
            ("-", TokenKind::Minus),
            ("*", TokenKind::Asterisk),
            ("/", TokenKind::Slash),
            ("%", TokenKind::Percent),
            ("**", TokenKind::AsteriskAsterisk),
            (".", TokenKind::Dot),
            ("++", TokenKind::PlusPlus),
            ("--", TokenKind::MinusMinus),
            ("->", TokenKind::MinusGreaterThan),
            ("?->", TokenKind::QuestionMinusGreaterThan),
            ("::", TokenKind::ColonColon),
            ("=>", TokenKind::EqualGreaterThan),
            ("..", TokenKind::DotDot),
            ("..=", TokenKind::DotDotEqual),
            ("...", TokenKind::DotDotDot),
            ("#[", TokenKind::HashLeftBracket),
            ("?", TokenKind::Question),
            (":", TokenKind::Colon),
            (";", TokenKind::Semicolon),
            (",", TokenKind::Comma),
            ("\\", TokenKind::NamespaceSeparator),
            ("$", TokenKind::Dollar),
        ];

        for (source, expected) in cases {
            let tokens = lex(source);
            assert_eq!(tokens.len(), 1, "{source}");
            assert_eq!(tokens[0].kind, *expected, "{source}");
        }
    }

    #[test]
    fn assert_or_null_is_two_tokens() {
        assert_eq!(
            kinds("$value ?as int"),
            vec![
                TokenKind::Variable,
                TokenKind::Question,
                TokenKind::As,
                TokenKind::Int
            ],
        );
        assert_eq!(
            kinds("$value ? as int"),
            vec![
                TokenKind::Variable,
                TokenKind::Question,
                TokenKind::As,
                TokenKind::Int
            ],
        );
    }

    #[test]
    fn integer_ranges_do_not_lex_as_floats() {
        assert_eq!(
            kinds("1..10 1..=10 ..10 ..=10"),
            vec![
                TokenKind::LiteralInteger,
                TokenKind::DotDot,
                TokenKind::LiteralInteger,
                TokenKind::LiteralInteger,
                TokenKind::DotDotEqual,
                TokenKind::LiteralInteger,
                TokenKind::DotDot,
                TokenKind::LiteralInteger,
                TokenKind::DotDotEqual,
                TokenKind::LiteralInteger,
            ],
        );
    }

    #[test]
    fn integer_literals() {
        let cases: &[&str] = &[
            "0",
            "123",
            "1_000_000",
            "0x1A3F",
            "0xff",
            "0b1010",
            "0b1_0_1_0",
            "0o17",
            "0o1_7",
        ];

        for source in cases {
            let tokens = lex(source);
            assert_eq!(tokens.len(), 1, "{source}");
            assert_eq!(tokens[0].kind, TokenKind::LiteralInteger, "{source}");
            assert_eq!(tokens[0].value, *source);
        }
    }

    #[test]
    fn float_literals() {
        let cases: &[&str] = &[
            "1.5", "0.5", ".5", "5.", "1e10", "1E10", "1e+5", "1.5e-3", ".5e3", "1_0.5", "1.0_1",
        ];

        for source in cases {
            let tokens = lex(source);
            assert_eq!(tokens.len(), 1, "{source}");
            assert_eq!(tokens[0].kind, TokenKind::LiteralFloat, "{source}");
            assert_eq!(tokens[0].value, *source);
        }
    }

    #[test]
    fn no_legacy_octal() {
        assert!(matches!(
            first_error("0755;"),
            SyntaxError::LeadingZeroInIntegerLiteral(_)
        ));
        assert!(matches!(
            first_error("0_1;"),
            SyntaxError::LeadingZeroInIntegerLiteral(_)
        ));
        assert_eq!(
            kinds("0 0o755"),
            vec![TokenKind::LiteralInteger, TokenKind::LiteralInteger]
        );
    }

    #[test]
    fn exponent_without_digits_is_not_a_float() {
        assert_eq!(
            kinds("1e"),
            vec![TokenKind::LiteralInteger, TokenKind::Identifier]
        );
    }

    #[test]
    fn separator_cannot_begin_a_float_component() {
        let fractional = lex("1._0");
        assert_eq!(fractional.len(), 2);
        assert_eq!(fractional[0].kind, TokenKind::LiteralFloat);
        assert_eq!(fractional[0].value, "1.");
        assert_eq!(fractional[1].kind, TokenKind::Identifier);
        assert_eq!(fractional[1].value, "_0");

        let exponent = lex("1.0e_1");
        assert_eq!(exponent.len(), 2);
        assert_eq!(exponent[0].kind, TokenKind::LiteralFloat);
        assert_eq!(exponent[0].value, "1.0");
        assert_eq!(exponent[1].kind, TokenKind::Identifier);
        assert_eq!(exponent[1].value, "e_1");
    }

    #[test]
    fn string_literals() {
        let cases: &[&str] = &["'abc'", "\"abc\"", "''", "'it\\'s'", "\"say \\\"hi\\\"\""];

        for source in cases {
            let tokens = lex(source);
            assert_eq!(tokens.len(), 1, "{source}");
            assert_eq!(tokens[0].kind, TokenKind::LiteralString, "{source}");
            assert_eq!(tokens[0].value, *source);
        }
    }

    #[test]
    fn interpolated_strings_are_lexed_as_their_source_parts() {
        let tokens = lex(r#""hello, $name! {$foo}!""#);
        let parts: Vec<_> = tokens
            .iter()
            .map(|token| (token.kind, token.value))
            .collect();

        assert_eq!(
            parts,
            vec![
                (TokenKind::StringPart, "\"hello, "),
                (TokenKind::Variable, "$name"),
                (TokenKind::StringPart, "! "),
                (TokenKind::LeftBrace, "{"),
                (TokenKind::Variable, "$foo"),
                (TokenKind::RightBrace, "}"),
                (TokenKind::StringPart, "!\""),
            ]
        );
    }

    #[test]
    fn double_quoted_strings_without_interpolation_stay_literal_tokens() {
        let tokens = lex(r#""hello, name!""#);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::LiteralString);
    }

    #[test]
    fn unclosed_string_is_an_error() {
        assert!(matches!(
            first_error("'abc"),
            SyntaxError::UnclosedStringLiteral(_)
        ));
        assert!(matches!(
            first_error("\"abc\\\""),
            SyntaxError::UnclosedStringLiteral(_)
        ));
    }

    #[test]
    fn comments() {
        let tokens = lex("// line\n/* block */ /** doc */ /**/");
        let comments: Vec<_> = tokens.iter().filter(|t| t.kind.is_comment()).collect();

        assert_eq!(comments.len(), 4);
        assert_eq!(comments[0].kind, TokenKind::SingleLineComment);
        assert_eq!(comments[0].value, "// line");
        assert_eq!(comments[1].kind, TokenKind::MultiLineComment);
        assert_eq!(comments[1].value, "/* block */");
        assert_eq!(comments[2].kind, TokenKind::DocBlockComment);
        assert_eq!(comments[2].value, "/** doc */");
        assert_eq!(comments[3].kind, TokenKind::MultiLineComment);
        assert_eq!(comments[3].value, "/**/");
    }

    #[test]
    fn no_hash_comments() {
        assert!(matches!(
            first_error("# comment"),
            SyntaxError::UnrecognizedByte(b'#', _)
        ));
    }

    #[test]
    fn unclosed_block_comment_is_an_error() {
        assert!(matches!(
            first_error("/* abc"),
            SyntaxError::UnclosedBlockComment(_)
        ));
    }

    #[test]
    fn identifiers_and_variables() {
        assert_eq!(
            kinds("foo Foo\\Bar \\Foo\\Bar $foo"),
            vec![
                TokenKind::Identifier,
                TokenKind::QualifiedIdentifier,
                TokenKind::FullyQualifiedIdentifier,
                TokenKind::Variable,
            ],
        );
    }

    #[test]
    fn qualified_identifier_never_includes_trailing_backslash() {
        assert_eq!(
            kinds("Foo\\Bar\\"),
            vec![
                TokenKind::QualifiedIdentifier,
                TokenKind::NamespaceSeparator
            ],
        );
    }

    #[test]
    fn attribute_open() {
        assert_eq!(
            kinds("#[Attribute]"),
            vec![
                TokenKind::HashLeftBracket,
                TokenKind::Identifier,
                TokenKind::RightBracket
            ],
        );
    }

    #[test]
    fn token_positions_and_spans() {
        let tokens = lex("$a + 1");

        assert_eq!(tokens[0].start.offset, 0);
        assert_eq!(tokens[0].compute_span().end.offset, 2);
        assert_eq!(tokens[2].start.offset, 3);
        assert_eq!(tokens[4].start.offset, 5);
        assert_eq!(tokens[4].compute_span().end.offset, 6);
    }

    #[test]
    fn a_small_program() {
        let source = r"#!/usr/bin/env whim
namespace App;

function main(): int {
    $value = $input ?as int ?? 0;
    if ($value is int && $value > 0) {
        return $value ** 2;
    }

    return 0;
}
";

        let expected = vec![
            TokenKind::Namespace,
            TokenKind::Identifier,
            TokenKind::Semicolon,
            TokenKind::Function,
            TokenKind::Identifier,
            TokenKind::LeftParenthesis,
            TokenKind::RightParenthesis,
            TokenKind::Colon,
            TokenKind::Int,
            TokenKind::LeftBrace,
            TokenKind::Variable,
            TokenKind::Equal,
            TokenKind::Variable,
            TokenKind::Question,
            TokenKind::As,
            TokenKind::Int,
            TokenKind::QuestionQuestion,
            TokenKind::LiteralInteger,
            TokenKind::Semicolon,
            TokenKind::If,
            TokenKind::LeftParenthesis,
            TokenKind::Variable,
            TokenKind::Is,
            TokenKind::Int,
            TokenKind::AmpersandAmpersand,
            TokenKind::Variable,
            TokenKind::GreaterThan,
            TokenKind::LiteralInteger,
            TokenKind::RightParenthesis,
            TokenKind::LeftBrace,
            TokenKind::Return,
            TokenKind::Variable,
            TokenKind::AsteriskAsterisk,
            TokenKind::LiteralInteger,
            TokenKind::Semicolon,
            TokenKind::RightBrace,
            TokenKind::Return,
            TokenKind::LiteralInteger,
            TokenKind::Semicolon,
            TokenKind::RightBrace,
        ];

        assert_eq!(kinds(source), expected);
    }
}
