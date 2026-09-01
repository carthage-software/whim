#![deny(clippy::nursery, clippy::pedantic)]
#![forbid(unsafe_code)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "crate visibility records module boundaries in this binary"
)]

//! The Whim command-line interface.

mod color;
mod commands;
mod config;
mod engine;
mod error;
mod filesystem;
mod git;
mod logger;
mod output;
mod package;
mod server;
mod source;
mod style;

use std::process::ExitCode;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> ExitCode {
    match commands::execute() {
        Ok(code) => code,
        Err(error) => {
            tracing::debug!(error = ?error, "command failed");
            tracing::error!("{error}");
            error.exit_code()
        }
    }
}
