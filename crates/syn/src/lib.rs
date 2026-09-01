use std::hint;

pub mod arena;
pub mod cst;
pub mod diagnostic;
pub mod error;
pub mod fragment;
pub mod input;
pub mod lexer;
pub mod parser;
pub mod token;

mod consts;
mod macros;
mod utils;

/// # Safety
///
/// The path must be truly unreachable while the crate's invariants hold;
/// reaching it in a release build is undefined behavior.
#[inline(always)]
pub(crate) unsafe fn unreachable_invariant(message: &'static str) -> ! {
    if cfg!(debug_assertions) {
        panic!("whim-syn invariant violated: {message}");
    } else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { hint::unreachable_unchecked() }
    }
}
