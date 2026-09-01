//! The `Whim\Env` namespace: process arguments, environment variables, and the
//! well-known directories and binaries of the running process.

#![expect(
    clippy::option_if_let_else,
    reason = "explicit environment fallbacks are clearer than closure adapters"
)]
pub(crate) mod directories;
pub(crate) mod process;
pub(crate) mod variables;
