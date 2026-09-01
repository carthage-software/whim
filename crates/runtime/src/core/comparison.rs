//! Comparison protocols used by built-in and Whim-written values.

use whim_macros::whim_interface;

#[whim_interface("Whim\\Comparison\\Equal<in T>")]
pub(crate) trait Equal {
    #[whim_method("equals(T $other): bool")]
    fn equals();
}
