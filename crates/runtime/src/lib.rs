//! Whim's compiler and runtime.

pub mod artifact;
pub mod compiler;
pub mod disassembly;
pub mod engine;

mod variance;

pub(crate) mod blocking;
pub(crate) mod builtin;
pub(crate) mod bytecode;
pub(crate) mod classes;
pub(crate) mod core;
pub(crate) mod linker;
pub(crate) mod optimizer;
pub(crate) mod symbols;
pub(crate) mod vm;
