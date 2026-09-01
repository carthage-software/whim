//! Whim's compiler and runtime.

pub mod artifact;
pub mod disassembly;
pub mod engine;
pub mod path;

pub(crate) mod blocking;
pub(crate) mod builtin;
pub(crate) mod bytecode;
pub(crate) mod classes;
pub(crate) mod compiler;
pub(crate) mod core;
pub(crate) mod limits;
pub(crate) mod linker;
pub(crate) mod optimizer;
pub(crate) mod symbols;
pub(crate) mod value;
mod variance;
pub(crate) mod vm;

use std::hint;

/// Marks an engine-proved path unreachable.
///
/// # Safety
///
/// The engine must prove that this path cannot run.
#[expect(
    clippy::inline_always,
    reason = "release builds remove invariant failures from their callers"
)]
#[inline(always)]
pub(crate) unsafe fn unreachable_invariant(message: &'static str) -> ! {
    if cfg!(debug_assertions) {
        panic!("whim-runtime invariant violated: {message}");
    } else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { hint::unreachable_unchecked() }
    }
}

/// Unwraps an engine-proved [`Some`] without a release panic branch.
///
/// # Safety
///
/// The engine must prove that `option` is [`Some`].
#[expect(
    clippy::inline_always,
    clippy::option_if_let_else,
    reason = "the explicit match exposes the cold invariant failure to every caller"
)]
#[inline(always)]
pub(crate) unsafe fn unwrap_option_invariant<T>(option: Option<T>, message: &'static str) -> T {
    match option {
        Some(value) => value,
        // SAFETY: the surrounding invariant makes this path unreachable.
        None => unsafe { unreachable_invariant(message) },
    }
}

/// Unwraps an engine-proved [`Ok`] without a release panic branch.
///
/// # Safety
///
/// The engine must prove that `result` is [`Ok`].
#[expect(
    clippy::inline_always,
    clippy::option_if_let_else,
    reason = "the explicit match exposes the cold invariant failure to every caller"
)]
#[inline(always)]
pub(crate) unsafe fn unwrap_result_invariant<T, E>(
    result: Result<T, E>,
    message: &'static str,
) -> T {
    match result {
        Ok(value) => value,
        // SAFETY: the surrounding invariant makes this path unreachable.
        Err(_) => unsafe { unreachable_invariant(message) },
    }
}

/// Narrows a runtime table index to its bytecode representation.
#[inline]
pub(crate) fn u32_index(value: usize) -> u32 {
    // SAFETY: runtime tables cannot contain more than u32::MAX entries.
    unsafe {
        unwrap_result_invariant(
            u32::try_from(value),
            "a runtime table cannot contain more than u32::MAX entries",
        )
    }
}
