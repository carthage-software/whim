//! The compiled form of Whim programs.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "decoding and disassembly helpers stay private to the bytecode crate"
)]

pub mod aliases;
pub mod chunk;
pub mod decode;
pub mod disassemble;
pub mod instruction;
pub mod reference_registers;
pub mod render;
pub mod rewrite;
pub mod unit;
pub mod verify;

/// The number of registers represented by a reference ownership mask.
pub const REFERENCE_REGISTER_LIMIT: u16 = 64;
