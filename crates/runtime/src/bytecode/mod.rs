//! The compiled form of Whim programs.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "bytecode is shared across the compiler, optimizer, and VM"
)]

pub(crate) mod aliases;
pub(crate) mod chunk;
pub(crate) mod decode;
pub(crate) mod disassemble;
pub(crate) mod instruction;
pub(crate) mod reference_registers;
pub(crate) mod rewrite;
pub(crate) mod unit;
pub(crate) mod verify;

pub(crate) mod render;

/// The number of registers represented by a reference ownership mask.
pub(crate) const REFERENCE_REGISTER_LIMIT: u16 = 64;
