//! Conversion protocols used by built-in and Whim-written values.

use whim_macros::whim_interface;

#[whim_interface("Whim\\Convert\\ToString")]
pub(crate) trait ToString {
    #[whim_method("toString(): string")]
    fn to_string();
}
