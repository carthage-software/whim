//! Files, sockets, and OS operations, independent of Whim values and execution.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod accounts;
pub mod dns;
mod error;
pub mod file;
pub mod filesystem;
pub mod message;
pub mod operation;
pub mod path;
pub mod process;
pub mod socket;
pub use platform::constants;
pub use platform::{system, terminal};

#[cfg(unix)]
#[path = "unix/mod.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod platform;

pub use platform::{Descriptor, initialize_console};
pub use whim_loop::{Interest, RawDescriptor};

use std::result;
use std::time::Duration;

#[derive(Clone, Copy)]
pub enum StandardStream {
    Input,
    Output,
    Error,
}

#[derive(Clone, Copy)]
pub enum Readiness {
    Descriptor(RawDescriptor),
    Poll(Duration),
}

pub use error::Error;

pub type Result<T> = result::Result<T, Error>;
