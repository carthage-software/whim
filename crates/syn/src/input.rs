use std::str;

use whim_span::Position;

use crate::consts::IDENTIFIER_PART_TABLE;
use crate::consts::WHITESPACE_TABLE;

/// The cursor only ever advances by whole tokens, which begin and end on
/// UTF-8 character boundaries. `consume*` methods rely on this to hand back
/// `&str` without re-validating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Input<'src> {
    bytes: &'src [u8],
    offset: usize,
    starting_position: Position,
}

/// # Safety
///
/// `bytes` must be valid UTF-8. Every caller passes a sub-slice of an
/// [`Input`]'s underlying `&str`, taken at token boundaries (which are always
/// UTF-8 character boundaries), so the invariant holds.
#[inline(always)]
const unsafe fn as_str(bytes: &[u8]) -> &str {
    // SAFETY: guaranteed by the caller; see the method documentation.
    unsafe { str::from_utf8_unchecked(bytes) }
}

impl<'src> Input<'src> {
    #[must_use]
    pub const fn new(source: &'src str) -> Self {
        Self {
            bytes: source.as_bytes(),
            offset: 0,
            starting_position: Position::new(0),
        }
    }

    /// `source` is a slice of a larger file. The cursor starts at offset 0
    /// within `source`, but `current_position` is computed relative to
    /// `anchor_position`, so tokens keep their true position in the file.
    #[must_use]
    pub const fn anchored_at(source: &'src str, anchor_position: Position) -> Self {
        Self {
            bytes: source.as_bytes(),
            offset: 0,
            starting_position: anchor_position,
        }
    }

    #[inline]
    #[must_use]
    pub const fn current_position(&self) -> Position {
        self.starting_position.forward(self.offset as u32)
    }

    /// The byte offset within the current slice; not the same as
    /// `current_position`, which is absolute within the source file.
    #[inline]
    #[must_use]
    pub const fn current_offset(&self) -> usize {
        self.offset
    }

    #[inline(always)]
    #[must_use]
    pub const fn has_reached_eof(&self) -> bool {
        self.offset >= self.bytes.len()
    }

    #[inline(always)]
    pub const fn next(&mut self) {
        if self.offset < self.bytes.len() {
            self.offset += 1;
        }
    }

    #[inline]
    pub(crate) fn skip(&mut self, count: usize) {
        self.offset = self.offset.saturating_add(count).min(self.bytes.len());
    }

    #[inline(always)]
    pub(crate) fn consume(&mut self, count: usize) -> &'src str {
        let from = self.offset;
        let until = from.saturating_add(count).min(self.bytes.len());
        self.offset = until;
        // SAFETY: `from <= until <= self.bytes.len()`, and both bounds land on UTF-8
        // character boundaries because the cursor only advances by whole tokens.
        unsafe { as_str(self.bytes.get_unchecked(from..until)) }
    }

    #[inline]
    pub(crate) fn consume_remaining(&mut self) -> &'src str {
        if self.has_reached_eof() {
            return "";
        }

        let from = self.offset;
        self.offset = self.bytes.len();

        // SAFETY: `from` is at a UTF-8 character boundary and the range runs to
        // the end of a valid-UTF-8 buffer, so the slice is valid UTF-8.
        unsafe { as_str(self.bytes.get_unchecked(from..)) }
    }

    #[inline]
    pub(crate) fn consume_through(&mut self, search: u8) -> &'src str {
        let start = self.offset;
        let end = if let Some(pos) = memchr::memchr(search, &self.bytes[self.offset..]) {
            self.offset += pos + 1;
            self.offset
        } else {
            self.offset = self.bytes.len();
            self.bytes.len()
        };

        // SAFETY: `start` is at a character boundary; `search` is a single-byte
        // (ASCII) delimiter, so `end` is one too, keeping the slice valid UTF-8.
        unsafe { as_str(self.bytes.get_unchecked(start..end)) }
    }

    #[inline(always)]
    pub(crate) fn consume_whitespaces(&mut self) -> &'src str {
        let start = self.offset;
        let bytes = self.bytes;
        let len = bytes.len();

        while self.offset < len {
            // SAFETY: `self.offset < len` was just checked, so the index is in bounds.
            let b = unsafe { *bytes.get_unchecked(self.offset) };
            if !WHITESPACE_TABLE[b as usize] {
                break;
            }

            self.offset += 1;
        }

        // SAFETY: `start <= self.offset <= len` (the loop only advances `self.offset`),
        // and every consumed byte is ASCII whitespace, so the slice is valid UTF-8.
        unsafe { as_str(bytes.get_unchecked(start..self.offset)) }
    }

    /// Does not validate the byte at `offset_from_current` itself; the caller
    /// must already know it starts an identifier. Returns the run length,
    /// excluding a trailing `\`, and whether that `\` is followed by another
    /// identifier-start byte.
    #[inline(always)]
    #[must_use]
    pub fn scan_identifier(&self, offset_from_current: usize) -> (usize, bool) {
        let start = self.offset.saturating_add(offset_from_current);
        if start >= self.bytes.len() {
            return (0, false);
        }

        let bytes = self.bytes;
        let len = bytes.len();
        let mut pos = start + 1;

        while pos < len {
            // SAFETY: `pos < len` was just checked.
            let b = unsafe { *bytes.get_unchecked(pos) };
            if IDENTIFIER_PART_TABLE[b as usize] {
                pos += 1;
            } else if b == b'\\' && pos + 1 < len {
                // SAFETY: `pos + 1 < len` was just checked in the `else if` guard.
                let next = unsafe { *bytes.get_unchecked(pos + 1) };
                if IDENTIFIER_PART_TABLE[next as usize] {
                    return (pos - start, true);
                }

                break;
            } else {
                break;
            }
        }

        (pos - start, false)
    }

    #[inline(always)]
    #[must_use]
    pub fn read(&self, n: usize) -> &'src [u8] {
        let from = self.offset;
        let until = from.saturating_add(n).min(self.bytes.len());
        // SAFETY: `from <= until <= self.bytes.len()`.
        unsafe { self.bytes.get_unchecked(from..until) }
    }

    #[inline(always)]
    #[must_use]
    pub fn read_remaining(&self) -> &'src [u8] {
        let from = self.offset;

        // SAFETY: `from` is always within the byte slice.
        unsafe { self.bytes.get_unchecked(from..) }
    }

    #[inline(always)]
    #[must_use]
    pub fn is_at(&self, search: &[u8], ignore_ascii_case: bool) -> bool {
        let len = search.len();
        let from = self.offset;
        let until = from.saturating_add(len).min(self.bytes.len());

        if until - from != len {
            return false;
        }

        let slice = &self.bytes[from..until];
        if ignore_ascii_case {
            slice.eq_ignore_ascii_case(search)
        } else {
            slice == search
        }
    }

    #[inline(always)]
    #[must_use]
    pub fn peek(&self, offset: usize, n: usize) -> &'src [u8] {
        let from = self.offset.saturating_add(offset);
        let len = self.bytes.len();
        if from >= len {
            return &[];
        }

        let until = from.saturating_add(n).min(len);
        // SAFETY: the checks above prove `from < len` and `until <= len`.
        unsafe { self.bytes.get_unchecked(from..until) }
    }
}

#[cfg(test)]
mod tests {
    use whim_span::Position;

    use crate::input::*;

    #[test]
    fn test_new() {
        let source = "Hello, world!";
        let input = Input::new(source);

        assert_eq!(input.current_position(), Position::new(0));
        assert_eq!(input.bytes, source.as_bytes());
    }

    #[test]
    fn test_is_eof() {
        let input = Input::new("");

        assert!(input.has_reached_eof());

        let mut input = Input::new("data");

        assert!(!input.has_reached_eof());

        input.skip(4);

        assert!(input.has_reached_eof());
    }

    #[test]
    fn test_next() {
        let mut input = Input::new("a\nb\r\nc\rd");

        input.next();
        assert_eq!(input.current_position(), Position::new(1));

        input.next();
        assert_eq!(input.current_position(), Position::new(2));

        input.next();
        assert_eq!(input.current_position(), Position::new(3));

        input.next();
        assert_eq!(input.current_position(), Position::new(4));

        input.next();
        assert_eq!(input.current_position(), Position::new(5));

        input.next();
        assert_eq!(input.current_position(), Position::new(6));

        input.next();
        assert_eq!(input.current_position(), Position::new(7));
    }

    #[test]
    fn test_consume() {
        let mut input = Input::new("abcdef");

        let consumed = input.consume(3);
        assert_eq!(consumed, "abc");
        assert_eq!(input.current_position(), Position::new(3));

        let consumed = input.consume(3);
        assert_eq!(consumed, "def");
        assert_eq!(input.current_position(), Position::new(6));

        let consumed = input.consume(1);
        assert_eq!(consumed, "");
        assert!(input.has_reached_eof());
    }

    #[test]
    fn test_consume_remaining() {
        let mut input = Input::new("abcdef");

        input.skip(2);
        let remaining = input.consume_remaining();
        assert_eq!(remaining, "cdef");
        assert!(input.has_reached_eof());
    }

    #[test]
    fn test_consume_preserves_multibyte_characters() {
        let mut input = Input::new("café");

        let consumed = input.consume(5);
        assert_eq!(consumed, "café");
        assert!(input.has_reached_eof());
    }

    #[test]
    fn test_read() {
        let input = Input::new("abcdef");

        let read = input.read(3);
        assert_eq!(read, b"abc");
        assert_eq!(input.current_position(), Position::new(0));
    }

    #[test]
    fn test_is_at() {
        let mut input = Input::new("abcdef");

        assert!(input.is_at(b"abc", false));
        input.skip(2);
        assert!(input.is_at(b"cde", false));
        assert!(!input.is_at(b"xyz", false));
    }

    #[test]
    fn test_is_at_ignore_ascii_case() {
        let mut input = Input::new("AbCdEf");

        assert!(input.is_at(b"abc", true));
        input.skip(2);
        assert!(input.is_at(b"cde", true));
        assert!(!input.is_at(b"xyz", true));
    }

    #[test]
    fn test_peek() {
        let input = Input::new("abcdef");

        let peeked = input.peek(2, 3);
        assert_eq!(peeked, b"cde");
        assert_eq!(input.current_position(), Position::new(0));
    }

    #[test]
    fn test_peek_saturates_large_lengths() {
        let input = Input::new("abcdef");

        assert_eq!(input.peek(2, usize::MAX), b"cdef");
    }

    #[test]
    fn test_scan_identifier_trusts_caller_for_first_byte() {
        let input = Input::new("!abc");

        let (length, escaped) = input.scan_identifier(0);

        assert_eq!(length, 4);
        assert!(!escaped);
    }
}
