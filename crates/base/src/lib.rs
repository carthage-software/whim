//! Shared helpers and limits for Whim.

pub mod limits;

use std::hint;

/// Marks a path that the caller has proved unreachable.
///
/// # Safety
///
/// The caller must prove that this path cannot run.
#[expect(
    clippy::inline_always,
    reason = "release builds remove invariant failures from their callers"
)]
#[inline(always)]
pub unsafe fn unreachable_invariant(message: &'static str) -> ! {
    if cfg!(debug_assertions) {
        panic!("whim invariant violated: {message}");
    } else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { hint::unreachable_unchecked() }
    }
}

/// Unwraps a proven [`Some`] without a release panic branch.
///
/// # Safety
///
/// The caller must prove that `option` is [`Some`].
#[expect(
    clippy::inline_always,
    clippy::option_if_let_else,
    reason = "the explicit match exposes the cold invariant failure to every caller"
)]
#[inline(always)]
pub unsafe fn unwrap_option_invariant<T>(option: Option<T>, message: &'static str) -> T {
    match option {
        Some(value) => value,
        // SAFETY: the surrounding invariant makes this path unreachable.
        None => unsafe { unreachable_invariant(message) },
    }
}

/// Unwraps a proven [`Ok`] without a release panic branch.
///
/// # Safety
///
/// The caller must prove that `result` is [`Ok`].
#[expect(
    clippy::inline_always,
    clippy::option_if_let_else,
    reason = "the explicit match exposes the cold invariant failure to every caller"
)]
#[inline(always)]
pub unsafe fn unwrap_result_invariant<T, E>(result: Result<T, E>, message: &'static str) -> T {
    match result {
        Ok(value) => value,
        // SAFETY: the surrounding invariant makes this path unreachable.
        Err(_) => unsafe { unreachable_invariant(message) },
    }
}

/// Converts an index to `u32`.
///
/// # Panics
///
/// Panics if `value` exceeds [`u32::MAX`].
#[inline]
pub fn u32_index(value: usize) -> u32 {
    u32::try_from(value).expect("an index must fit in u32")
}

#[cfg(test)]
mod tests {
    use super::u32_index;

    #[test]
    fn u32_index_preserves_bounds() {
        assert_eq!(u32_index(0), 0);
        assert_eq!(u32_index(u32::MAX as usize), u32::MAX);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    #[should_panic(expected = "an index must fit in u32")]
    fn u32_index_rejects_overflow() {
        u32_index(u32::MAX as usize + 1);
    }
}
