use memchr::memmem;

use crate::consts::IDENTIFIER_START_TABLE;
use crate::utils::double_quoted_escape_length;

#[inline]
#[must_use]
pub(in crate::lexer) fn scan(bytes: &[u8]) -> Option<usize> {
    let quote = bytes[0];
    let mut position = 1;

    while position < bytes.len() {
        match bytes[position] {
            b'\\' if quote == b'"' => {
                position += double_quoted_escape_length(&bytes[position..]);
            }
            b'\\' => position += 2,
            byte if byte == quote => return Some(position + 1),
            b'{' if quote == b'"' => position = scan_interpolation(bytes, position + 1)?,
            _ => position += 1,
        }
    }

    None
}

/// Whether a complete double-quoted string needs interpolation tokenization.
#[inline]
#[must_use]
pub(in crate::lexer) fn needs_interpolation(bytes: &[u8]) -> bool {
    let content = &bytes[1..bytes.len() - 1];
    let mut position = 0usize;
    while position < content.len() {
        match content[position] {
            b'\\' => position += double_quoted_escape_length(&content[position..]),
            b'{' | b'}' => return true,
            b'$' if content
                .get(position + 1)
                .is_some_and(|next| IDENTIFIER_START_TABLE[*next as usize]) =>
            {
                return true;
            }
            _ => position += 1,
        }
    }

    false
}

/// Returns the position immediately after the `}` closing an interpolation.
pub(in crate::lexer) fn scan_interpolation(bytes: &[u8], mut position: usize) -> Option<usize> {
    let mut braces = 1usize;
    while position < bytes.len() {
        match bytes[position] {
            b'\'' | b'"' => position += scan(&bytes[position..])?,
            b'/' if bytes.get(position + 1) == Some(&b'/') => {
                position += 2;
                while position < bytes.len() && !matches!(bytes[position], b'\n' | b'\r') {
                    position += 1;
                }
            }
            b'/' if bytes.get(position + 1) == Some(&b'*') => {
                let rest = &bytes[position + 2..];
                let close = memmem::find(rest, b"*/")?;
                position += close + 4;
            }
            b'{' => {
                braces += 1;
                position += 1;
            }
            b'}' => {
                braces -= 1;
                position += 1;
                if braces == 0 {
                    return Some(position);
                }
            }
            _ => position += 1,
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use crate::lexer::internal::string::scan;

    #[test]
    fn terminated_strings() {
        assert_eq!(scan(b"''"), Some(2));
        assert_eq!(scan(b"'abc'"), Some(5));
        assert_eq!(scan(b"\"abc\""), Some(5));
        assert_eq!(scan(b"'it\\'s'"), Some(7));
        assert_eq!(scan(b"\"say \\\"hi\\\"\""), Some(12));
        assert_eq!(scan(b"'abc' + 1"), Some(5));
        assert_eq!(scan(b"'a\\\\'"), Some(5));
        assert_eq!(scan(br#""\u{41}""#), Some(8));
        let nested = br#""outer {"inner $name"} tail""#;
        assert_eq!(scan(nested), Some(nested.len()));
        let matching = br#""value {match ($x) { 1 => "one", $_ => "other" }}""#;
        assert_eq!(scan(matching), Some(matching.len()));
    }

    #[test]
    fn unterminated_strings() {
        assert_eq!(scan(b"'"), None);
        assert_eq!(scan(b"'abc"), None);
        assert_eq!(scan(b"\"abc'"), None);
        assert_eq!(scan(b"'abc\\'"), None);
        assert_eq!(scan(b"'abc\\"), None);
    }
}
