//! The inline short string stored directly in a value.

use std::hint;

use crate::value::hash::HashState;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct ShortString {
    bytes: [u8; Self::CAPACITY],
    len: u8,
}

#[expect(
    clippy::inline_always,
    reason = "short strings are an inline value representation"
)]
impl ShortString {
    pub(crate) const CAPACITY: usize = 7;

    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the preceding capacity check bounds the length to seven"
    )]
    #[inline(always)]
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > Self::CAPACITY {
            return None;
        }
        let mut inline = [0; Self::CAPACITY];
        macro_rules! copy {
            ($($index:expr),* $(,)?) => {{
                // SAFETY: the surrounding invariant keeps this index in bounds.
                $(inline[$index] = unsafe { *bytes.get_unchecked($index) };)*
            }};
        }

        match bytes.len() {
            0 => {}
            1 => copy!(0),
            2 => copy!(0, 1),
            3 => copy!(0, 1, 2),
            4 => copy!(0, 1, 2, 3),
            5 => copy!(0, 1, 2, 3, 4),
            6 => copy!(0, 1, 2, 3, 4, 5),
            7 => copy!(0, 1, 2, 3, 4, 5, 6),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { hint::unreachable_unchecked() },
        }

        Some(Self {
            bytes: inline,
            len: bytes.len() as u8,
        })
    }

    /// Builds a short string from bytes packed in display order from the low byte.
    ///
    /// # Safety
    ///
    /// `len` must not exceed [`Self::CAPACITY`], and bytes above it must be zero.
    #[must_use]
    #[inline(always)]
    pub(crate) unsafe fn from_packed_unchecked(packed: u64, len: u8) -> Self {
        if usize::from(len) > Self::CAPACITY {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { hint::unreachable_unchecked() }
        }
        let bytes = packed.to_le_bytes();
        Self {
            bytes: [
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
            ],
            len,
        }
    }

    #[must_use]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { self.bytes.get_unchecked(..usize::from(self.len)) }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn hash64(self, state: &HashState) -> u64 {
        state.hash_short_string(u64::from_le_bytes([
            self.bytes[0],
            self.bytes[1],
            self.bytes[2],
            self.bytes[3],
            self.bytes[4],
            self.bytes[5],
            self.bytes[6],
            self.len,
        ]))
    }
}
