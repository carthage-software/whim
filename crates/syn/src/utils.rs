use std::str;

use crate::arena::Arena;
use crate::arena::Vec;
use crate::error::StringLiteralError;
use crate::input::Input;
use crate::number_separator;

/// The number of source bytes occupied by one escape beginning at `bytes[0]`
/// in a double-quoted string. A Unicode escape owns its whole `{...}`
/// spelling, so those braces are not mistaken for string interpolation.
#[inline]
#[must_use]
pub(super) fn double_quoted_escape_length(bytes: &[u8]) -> usize {
    if matches!(bytes, [b'\\', b'u', b'{', ..]) {
        return bytes[3..]
            .iter()
            .position(|byte| *byte == b'}')
            .map_or(3, |close| close + 4);
    }

    bytes.len().min(2)
}

/// Parses a literal string and retains the reason an escape is invalid.
pub(super) fn parse_literal_string_detailed_in<'arena, A>(
    arena: &'arena A,
    source: &'arena [u8],
    quote_char: Option<u8>,
    has_quote: bool,
) -> Result<&'arena [u8], StringLiteralError>
where
    A: Arena,
{
    if source.is_empty() {
        return Ok(b"");
    }

    let (quote_char, content) = literal_content(source, quote_char, has_quote)?;

    let needs_processing =
        content.contains(&b'\\') || quote_char.is_some_and(|quote| content.contains(&quote));
    if !needs_processing {
        return Ok(content);
    }

    let mut result = Vec::with_capacity_in(content.len(), arena);
    let mut position = 0;

    while position < content.len() {
        let byte = content[position];
        if byte != b'\\' {
            result.push(byte);
            position += 1;
            continue;
        }

        let next_index = position + 1;
        let Some(&next) = content.get(next_index) else {
            result.push(b'\\');
            position += 1;
            continue;
        };

        let mut consumed = 2;
        match next {
            b'\\' => result.push(b'\\'),
            b'\'' if quote_char == Some(b'\'') => result.push(b'\''),
            b'"' if quote_char == Some(b'"') => result.push(b'"'),
            b'$' if quote_char == Some(b'"') => result.push(b'$'),
            b'{' if quote_char == Some(b'"') => result.push(b'{'),
            b'}' if quote_char == Some(b'"') => result.push(b'}'),
            b'n' if quote_char == Some(b'"') => result.push(b'\n'),
            b't' if quote_char == Some(b'"') => result.push(b'\t'),
            b'r' if quote_char == Some(b'"') => result.push(b'\r'),
            b'v' if quote_char == Some(b'"') => result.push(0x0B),
            b'e' if quote_char == Some(b'"') => result.push(0x1B),
            b'f' if quote_char == Some(b'"') => result.push(0x0C),
            b'x' if quote_char == Some(b'"') => {
                if let Some((value, length)) = hex_escape(content, position) {
                    result.push(value);
                    consumed = length;
                } else {
                    result.push(b'\\');
                    result.push(b'x');
                }
            }
            b'u' if quote_char == Some(b'"') && content.get(position + 2) == Some(&b'{') => {
                let (character, length) = unicode_escape(content, position)?;
                let mut encoded = [0; 4];
                result.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                consumed = length;
            }
            byte if quote_char == Some(b'"') && (b'0'..=b'7').contains(&byte) => {
                let (value, length) = octal_escape(content, position)?;
                result.push(value);
                consumed = length;
            }
            _ => {
                result.push(b'\\');
                result.push(next);
            }
        }

        position += consumed;
    }

    Ok(result.leak())
}

fn literal_content(
    source: &[u8],
    quote_char: Option<u8>,
    has_quote: bool,
) -> Result<(Option<u8>, &[u8]), StringLiteralError> {
    if let Some(quote_char) = quote_char {
        return Ok((Some(quote_char), source));
    }
    if !has_quote {
        return Ok((None, source));
    }
    if source.len() < 2 {
        return Err(StringLiteralError::MalformedLiteral);
    }

    match (source.first(), source.last()) {
        (Some(b'"'), Some(b'"')) => Ok((Some(b'"'), &source[1..source.len() - 1])),
        (Some(b'\''), Some(b'\'')) => Ok((Some(b'\''), &source[1..source.len() - 1])),
        _ => Err(StringLiteralError::MalformedLiteral),
    }
}

fn hex_escape(content: &[u8], start: usize) -> Option<(u8, usize)> {
    let mut value = 0;
    let mut length = 0;
    for &byte in content.get(start + 2..)?.iter().take(2) {
        let Some(digit) = hex_digit(byte) else {
            break;
        };
        value = value * 16 + digit;
        length += 1;
    }

    (length > 0).then_some((value, 2 + length))
}

fn unicode_escape(content: &[u8], start: usize) -> Result<(char, usize), StringLiteralError> {
    let mut code_point = 0_u32;
    let mut length = 0;
    let mut position = start + 3;
    while let Some(&byte) = content.get(position) {
        let Some(digit) = hex_digit(byte) else {
            break;
        };
        code_point = code_point
            .checked_mul(16)
            .and_then(|value| value.checked_add(u32::from(digit)))
            .ok_or(StringLiteralError::UnicodeOutOfRange)?;
        length += 1;
        position += 1;
    }

    if code_point > 0x10_FFFF {
        return Err(StringLiteralError::UnicodeOutOfRange);
    }
    if (0xD800..=0xDFFF).contains(&code_point) {
        return Err(StringLiteralError::UnicodeSurrogate);
    }
    if length == 0 || content.get(position) != Some(&b'}') {
        return Err(StringLiteralError::MalformedUnicodeEscape);
    }

    let Some(character) = char::from_u32(code_point) else {
        return Err(StringLiteralError::UnicodeOutOfRange);
    };
    Ok((character, position + 1 - start))
}

fn octal_escape(content: &[u8], start: usize) -> Result<(u8, usize), StringLiteralError> {
    let mut value = 0_u16;
    let mut length = 0;
    for &byte in content[start + 1..].iter().take(3) {
        if !(b'0'..=b'7').contains(&byte) {
            break;
        }
        value = value * 8 + u16::from(byte - b'0');
        length += 1;
    }

    let value = u8::try_from(value).map_err(|_| StringLiteralError::OctalEscapeOutOfRange)?;
    Ok((value, 1 + length))
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Parses a literal float, handling underscore separators.
#[inline]
#[must_use]
pub(super) fn parse_literal_float_in<A: Arena>(arena: &A, value: &[u8]) -> Option<f64> {
    if memchr::memchr(b'_', value).is_none() {
        return str::from_utf8(value).ok()?.parse::<f64>().ok();
    }

    let mut buffer = Vec::<u8, A>::with_capacity_in(64, arena);
    for &b in value {
        if b != b'_' {
            buffer.push(b);
        }
    }

    str::from_utf8(&buffer).ok()?.parse::<f64>().ok()
}

/// Parses a literal integer with support for binary, octal, decimal, and hex.
#[inline]
#[must_use]
pub(super) fn parse_literal_integer(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }

    let (radix, start) = match bytes {
        [b'0', b'x' | b'X', ..] => (16u64, 2),
        [b'0', b'o' | b'O', ..] => (8u64, 2),
        [b'0', b'b' | b'B', ..] => (2u64, 2),
        _ => (10u64, 0),
    };

    let mut result = 0_u64;
    let mut has_digits = false;

    for &b in &bytes[start..] {
        if b == b'_' {
            continue;
        }

        let digit = if b.is_ascii_digit() {
            u64::from(b - b'0')
        } else if (b'a'..=b'f').contains(&b) {
            u64::from(b - b'a' + 10)
        } else if (b'A'..=b'F').contains(&b) {
            u64::from(b - b'A' + 10)
        } else {
            return None;
        };

        if digit >= radix {
            return None;
        }

        has_digits = true;

        result = result.checked_mul(radix)?.checked_add(digit)?;
    }

    if !has_digits {
        return None;
    }

    Some(result)
}

/// Counts digit bytes valid for `base`, starting `offset` past the cursor,
/// treating a `_` between two digits as part of the run.
#[inline]
pub(super) fn read_digits_of_base(input: &Input<'_>, offset: usize, base: u8) -> usize {
    #[inline]
    fn read_digits_with<F>(input: &Input<'_>, offset: usize, is_digit: F) -> usize
    where
        F: Fn(u8) -> bool,
    {
        let bytes = input.read_remaining();
        let total = bytes.len();
        let mut pos = offset;

        while pos < total {
            let current = bytes[pos];
            if is_digit(current) {
                pos += 1;
            } else if pos + 1 < total
                && bytes[pos] == number_separator!()
                && is_digit(bytes[pos + 1])
            {
                pos += 2;
            } else {
                break;
            }
        }

        pos
    }

    if base == 16 {
        read_digits_with(input, offset, |byte| byte.is_ascii_hexdigit())
    } else {
        let max = b'0' + base;

        read_digits_with(input, offset, |byte| (b'0'..max).contains(&byte))
    }
}

#[cfg(test)]
mod tests {
    use crate::arena::LocalArena;

    use crate::utils::*;

    fn parse_literal_string_in<'arena>(
        arena: &'arena LocalArena,
        source: &'arena [u8],
        quote: Option<u8>,
        has_quote: bool,
    ) -> Option<&'arena [u8]> {
        parse_literal_string_detailed_in(arena, source, quote, has_quote).ok()
    }

    macro_rules! parse_int {
        ($input:expr, $expected:expr) => {
            assert_eq!(parse_literal_integer($input), $expected);
        };
    }

    #[test]
    fn test_unicode_escape_in_double_quoted() {
        let arena = LocalArena::new();

        assert_eq!(
            parse_literal_string_in(&arena, b"A\\u{1F600}", Some(b'"'), false),
            Some(&b"A\xF0\x9F\x98\x80"[..]),
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{41}", Some(b'"'), false),
            Some(&b"A"[..])
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{E9}", Some(b'"'), false),
            Some(&b"\xC3\xA9"[..])
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{D800}", Some(b'"'), false),
            None,
            "a surrogate is not a Unicode scalar value and is rejected"
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{DFFF}", Some(b'"'), false),
            None
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{}", Some(b'"'), false),
            None
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{12", Some(b'"'), false),
            None
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{ZZ}", Some(b'"'), false),
            None
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{110000}", Some(b'"'), false),
            None
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\uABC", Some(b'"'), false),
            Some(&b"\\uABC"[..])
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\u{41}", Some(b'\''), false),
            Some(&b"\\u{41}"[..])
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\x", Some(b'"'), false),
            Some(&b"\\x"[..])
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\q", Some(b'"'), false),
            Some(&b"\\q"[..])
        );
    }

    #[test]
    fn test_octal_escape_in_double_quoted() {
        let arena = LocalArena::new();

        assert_eq!(
            parse_literal_string_in(&arena, b"\\7\\77\\377", Some(b'"'), false),
            Some(&b"\x07\x3F\xFF"[..]),
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\400", Some(b'"'), false),
            None,
            "an octal escape must fit in one byte",
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\777", Some(b'"'), false),
            None,
        );
        assert_eq!(
            parse_literal_string_in(&arena, b"\\8\\9", Some(b'"'), false),
            Some(&b"\\8\\9"[..]),
        );
    }

    #[test]
    fn test_parse_literal_integer() {
        parse_int!(b"123", Some(123));
        parse_int!(b"0", Some(0));
        parse_int!(b"0b1010", Some(10));
        parse_int!(b"0o17", Some(15));
        parse_int!(b"0x1A3F", Some(6719));
        parse_int!(b"0XFF", Some(255));
        parse_int!(b"0_1_2_3", Some(123));
        parse_int!(b"0b1_0_1_0", Some(10));
        parse_int!(b"0o1_7", Some(15));
        parse_int!(b"0x1_A_3_F", Some(6719));
        parse_int!(b"", None);
        parse_int!(b"0xGHI", None);
        parse_int!(b"0b102", None);
        parse_int!(b"0o89", None);
    }
}
