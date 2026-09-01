//! The top-level `Whim\VERSION` constant.

use whim_macros::whim_constant;

#[whim_constant("Whim\\VERSION", "string")]
const VERSION: &str = env!("CARGO_PKG_VERSION");
