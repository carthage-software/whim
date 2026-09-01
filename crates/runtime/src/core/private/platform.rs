//! Platform constants exposed to the Whim standard library.

use std::env::consts;

use whim_macros::whim_constant;

#[whim_constant("Whim\\_Private\\OS", "string")]
pub(crate) const OS: &str = consts::OS;
