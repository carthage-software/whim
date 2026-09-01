//! The heap string buffer, shared through ropes and slices.

use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::slice;

use crate::value::heap::allocate_bytes;
use crate::value::heap::deallocate_bytes;

pub(in crate::value) enum HeapBytes {
    Inline {
        len: u8,
        bytes: [u8; Self::INLINE_CAPACITY],
    },
    Allocated {
        pointer: NonNull<u8>,
        len: usize,
        /// The allocated buffer size, in bytes; always at least `len`.
        capacity: usize,
    },
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "inline lengths are bounded by the 23-byte capacity"
)]
impl HeapBytes {
    const INLINE_CAPACITY: usize = 23;
    const MINIMUM_GROWTH_CAPACITY: usize = 32;

    pub(in crate::value) const fn empty() -> Self {
        Self::Inline {
            len: 0,
            bytes: [0; Self::INLINE_CAPACITY],
        }
    }

    pub(in crate::value) fn from_slice(bytes: &[u8]) -> Self {
        if bytes.len() <= Self::INLINE_CAPACITY {
            let mut inline = [0; Self::INLINE_CAPACITY];
            inline[..bytes.len()].copy_from_slice(bytes);
            return Self::Inline {
                len: bytes.len() as u8,
                bytes: inline,
            };
        }
        let pointer = allocate_bytes(bytes.len());
        // SAFETY: `pointer` has room for `bytes`, and the ranges do not overlap.
        unsafe {
            pointer
                .as_ptr()
                .copy_from_nonoverlapping(bytes.as_ptr(), bytes.len());
        }
        Self::Allocated {
            pointer,
            len: bytes.len(),
            capacity: bytes.len(),
        }
    }

    pub(in crate::value) fn from_vec(bytes: Vec<u8>) -> Self {
        if bytes.len() <= Self::INLINE_CAPACITY {
            let mut inline = [0; Self::INLINE_CAPACITY];
            inline[..bytes.len()].copy_from_slice(&bytes);
            return Self::Inline {
                len: bytes.len() as u8,
                bytes: inline,
            };
        }

        let mut bytes = ManuallyDrop::new(bytes);
        let len = bytes.len();
        let capacity = bytes.capacity();
        // SAFETY: a non-empty `Vec` with retained capacity has a non-null pointer.
        let pointer = unsafe { NonNull::new_unchecked(bytes.as_mut_ptr()) };
        Self::Allocated {
            pointer,
            len,
            capacity,
        }
    }

    /// Copies fragments into one exact-sized buffer.
    ///
    /// # Safety
    ///
    /// The fragments must contain exactly `len` bytes.
    #[expect(
        clippy::option_if_let_else,
        reason = "the explicit match mirrors the allocated and inline representations"
    )]
    pub(in crate::value) unsafe fn from_fragments<'a>(
        len: usize,
        fragments: impl Iterator<Item = &'a [u8]>,
    ) -> Self {
        let mut inline = [0; Self::INLINE_CAPACITY];
        let pointer = (len > Self::INLINE_CAPACITY).then(|| allocate_bytes(len));
        let mut written = 0;
        for fragment in fragments {
            let end = written + fragment.len();
            debug_assert!(end <= len);
            match pointer {
                // SAFETY: the caller's length contract bounds this disjoint copy.
                Some(pointer) => unsafe {
                    pointer
                        .as_ptr()
                        .add(written)
                        .copy_from_nonoverlapping(fragment.as_ptr(), fragment.len());
                },
                None => {
                    inline[written..end].copy_from_slice(fragment);
                }
            }
            written = end;
        }
        debug_assert_eq!(written, len);
        match pointer {
            Some(pointer) => Self::Allocated {
                pointer,
                len,
                capacity: len,
            },
            None => Self::Inline {
                len: len as u8,
                bytes: inline,
            },
        }
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { len, bytes } => &bytes[..usize::from(*len)],
            // SAFETY: the pointer and length share one live allocation.
            Self::Allocated { pointer, len, .. } => unsafe {
                slice::from_raw_parts(pointer.as_ptr(), *len)
            },
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Inline { len, .. } => usize::from(*len),
            Self::Allocated { len, .. } => *len,
        }
    }

    /// Appends bytes, growing the allocation when needed.
    pub(in crate::value) fn append(&mut self, extra: &[u8]) {
        if extra.is_empty() {
            return;
        }
        match self {
            Self::Inline { len, bytes } => {
                let current = usize::from(*len);
                let needed = current + extra.len();
                if needed <= Self::INLINE_CAPACITY {
                    bytes[current..needed].copy_from_slice(extra);
                    *len = needed as u8;
                    return;
                }

                let capacity = needed.max(Self::MINIMUM_GROWTH_CAPACITY);
                let pointer = allocate_bytes(capacity);
                // SAFETY: the new allocation has room for both disjoint copies.
                unsafe {
                    pointer
                        .as_ptr()
                        .copy_from_nonoverlapping(bytes.as_ptr(), current);
                    pointer
                        .as_ptr()
                        .add(current)
                        .copy_from_nonoverlapping(extra.as_ptr(), extra.len());
                }
                *self = Self::Allocated {
                    pointer,
                    len: needed,
                    capacity,
                };
            }
            Self::Allocated {
                pointer,
                len,
                capacity,
            } => {
                let needed = *len + extra.len();
                if needed > *capacity {
                    let new_capacity = needed
                        .max(capacity.saturating_mul(2))
                        .max(Self::MINIMUM_GROWTH_CAPACITY);
                    let new_pointer = allocate_bytes(new_capacity);
                    // SAFETY: the new allocation holds `len` bytes and does not overlap the old one.
                    unsafe {
                        new_pointer
                            .as_ptr()
                            .copy_from_nonoverlapping(pointer.as_ptr(), *len);
                        deallocate_bytes(*pointer, *capacity);
                    }
                    *pointer = new_pointer;
                    *capacity = new_capacity;
                }
                if extra.len() <= 8 && *capacity - needed >= 8 {
                    // SAFETY: the padded eight-byte write stays within capacity.
                    unsafe {
                        let mut word = [0u8; 8];
                        word.as_mut_ptr()
                            .copy_from_nonoverlapping(extra.as_ptr(), extra.len());
                        pointer.as_ptr().add(*len).cast::<[u8; 8]>().write(word);
                    }
                } else {
                    // SAFETY: the capacity check reserved room for `extra`.
                    unsafe {
                        pointer
                            .as_ptr()
                            .add(*len)
                            .copy_from_nonoverlapping(extra.as_ptr(), extra.len());
                    }
                }
                *len = needed;
            }
        }
    }

    /// Frees an allocated buffer.
    pub(in crate::value) fn release(self) {
        if let Self::Allocated {
            pointer, capacity, ..
        } = self
        {
            // SAFETY: this value owns the allocation and its matching capacity.
            unsafe {
                deallocate_bytes(pointer, capacity);
            }
        }
    }
}
