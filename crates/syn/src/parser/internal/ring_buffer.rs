//! Fixed-capacity ring buffer backing the parser's token lookahead.

use std::fmt;
use std::fmt::Debug;
use std::mem::MaybeUninit;

/// Fixed-capacity ring buffer for parser lookahead.
pub(in crate::parser) struct RingBuffer<T: Copy, const CAP: usize> {
    slots: [MaybeUninit<T>; CAP],
    head: usize,
    len: usize,
}

impl<T: Copy, const CAP: usize> Debug for RingBuffer<T, CAP> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RingBuffer")
            .field("cap", &CAP)
            .field("head", &self.head)
            .field("len", &self.len)
            .field("slots", &"<opaque>")
            .finish()
    }
}

impl<T: Copy, const CAP: usize> Default for RingBuffer<T, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const CAP: usize> RingBuffer<T, CAP> {
    #[inline(always)]
    #[must_use]
    pub(in crate::parser) fn new() -> Self {
        debug_assert!(
            CAP.is_power_of_two(),
            "the ring buffer capacity must be a power of two"
        );

        Self {
            slots: [const { MaybeUninit::uninit() }; CAP],
            head: 0,
            len: 0,
        }
    }

    #[inline(always)]
    pub(in crate::parser) const fn len(&self) -> usize {
        self.len
    }

    /// Push a token at the back.
    #[inline(always)]
    pub(in crate::parser) fn push_back(&mut self, value: T) {
        assert!(
            self.len < CAP,
            "ring buffer overflow: pushed {CAP} tokens without consuming"
        );

        let idx = (self.head + self.len) & (CAP - 1);
        self.slots[idx] = MaybeUninit::new(value);
        self.len += 1;
    }

    #[inline(always)]
    pub(in crate::parser) const fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }

        // SAFETY: `head` is within `len` occupied slots by construction;
        // those slots are always written before being read.
        let value = unsafe { self.slots[self.head].assume_init() };
        self.head = (self.head + 1) & (CAP - 1);
        self.len -= 1;
        Some(value)
    }

    /// Overwrite the front token in place, without changing occupancy.
    #[inline(always)]
    pub(in crate::parser) fn replace_front(&mut self, value: T) {
        assert!(self.len > 0, "replace_front on an empty RingBuffer");
        self.slots[self.head] = MaybeUninit::new(value);
    }

    /// Copy the `n`th-ahead token without consuming it (0 = next).
    #[inline(always)]
    pub(in crate::parser) const fn get(&self, n: usize) -> Option<T> {
        if n >= self.len {
            return None;
        }

        let idx = (self.head + n) & (CAP - 1);
        // SAFETY: `n < self.len` and all slots in [head, head+len) are
        // initialised by construction.
        Some(unsafe { self.slots[idx].assume_init() })
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::internal::ring_buffer::RingBuffer;

    #[test]
    fn roundtrip_smoke() {
        let mut buf: RingBuffer<u32, 8> = RingBuffer::new();
        assert_eq!(buf.len(), 0);
        for i in 0..6 {
            buf.push_back(i);
        }

        assert_eq!(buf.len(), 6);
        assert_eq!(buf.get(0), Some(0));
        assert_eq!(buf.get(5), Some(5));
        assert_eq!(buf.get(6), None);
        for i in 0..6 {
            assert_eq!(buf.pop_front(), Some(i));
        }

        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn wraps_around() {
        let mut buf: RingBuffer<u32, 4> = RingBuffer::new();
        for i in 0..4 {
            buf.push_back(i);
        }

        assert_eq!(buf.len(), 4);
        for _ in 0..3 {
            buf.pop_front();
        }

        assert_eq!(buf.len(), 1);
        buf.push_back(99);
        buf.push_back(100);
        assert_eq!(buf.get(0), Some(3));
        assert_eq!(buf.get(1), Some(99));
        assert_eq!(buf.get(2), Some(100));
    }
}
