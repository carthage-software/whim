//! Arena storage used by the parser and syntax tree.

use std::fmt;
use std::str;

use allocator_api2::alloc::Allocator;
use allocator_api2::boxed::Box;
use allocator_api2::vec::Vec as AllocVec;

/// A vector whose storage lives in `arena`.
pub type Vec<'arena, T, A> = AllocVec<T, &'arena A>;

/// The process-local arena used by parser and formatter entry points.
pub type LocalArena = blink_alloc::BlinkAlloc;

/// Typed placement helpers over a blink allocator.
#[expect(
    clippy::mut_from_ref,
    reason = "arena allocations are unique even though allocation takes a shared handle"
)]
pub trait Arena: Allocator {
    #[inline]
    fn alloc<T>(&self, value: T) -> &mut T {
        Box::leak(Box::new_in(value, self))
    }

    #[inline]
    fn alloc_str(&self, source: &str) -> &mut str {
        let mut bytes = AllocVec::with_capacity_in(source.len(), self);
        bytes.extend_from_slice(source.as_bytes());

        // SAFETY: the bytes are copied from a valid string.
        unsafe { str::from_utf8_unchecked_mut(bytes.leak()) }
    }

    #[inline]
    fn alloc_fmt(&self, arguments: fmt::Arguments<'_>) -> &mut str {
        struct Writer<'buffer, 'arena, A: Allocator + ?Sized> {
            bytes: &'buffer mut AllocVec<u8, &'arena A>,
        }

        impl<A: Allocator + ?Sized> fmt::Write for Writer<'_, '_, A> {
            fn write_str(&mut self, fragment: &str) -> fmt::Result {
                self.bytes.extend_from_slice(fragment.as_bytes());
                Ok(())
            }
        }

        let mut bytes = AllocVec::new_in(self);
        {
            let _ = fmt::write(&mut Writer { bytes: &mut bytes }, arguments);
        }

        // SAFETY: `fmt::Write` receives valid UTF-8 string fragments.
        unsafe { str::from_utf8_unchecked_mut(bytes.leak()) }
    }
}

impl<A: Allocator + ?Sized> Arena for A {}
