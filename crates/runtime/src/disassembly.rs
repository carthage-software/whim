//! Compilation and bytecode rendering for the official command-line tool.

#![deny(clippy::nursery, clippy::pedantic)]

use std::error::Error;
use std::fmt;
use std::io;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::rc::Rc;

use whim_span::Span;
use whim_syn::cst::Program;

use crate::bytecode::disassemble;
use crate::bytecode::unit::CompiledUnit;
use crate::compiler::CompileError;
use crate::engine::Engine;
use crate::value::heap::Heap;

/// One compiled program retained for deterministic bytecode rendering.
pub struct Disassembly {
    unit: CompiledUnit,
    /// Keeps every managed value in `unit` alive and drops after it.
    _heap: Rc<Heap>,
}

impl Disassembly {
    /// Compiles a parsed program without declaring or executing it.
    ///
    /// # Errors
    ///
    /// Returns a compiler error when the program cannot be compiled.
    pub fn compile(
        engine: &mut Engine,
        program: &Program<'_>,
        path: &Path,
    ) -> Result<Self, DisassemblyError> {
        let diagnostic_path = path.to_string_lossy();
        let unit = engine
            .compile_program(program, &diagnostic_path, path.as_os_str().as_bytes())
            .map_err(DisassemblyError::from)?;
        let heap = Rc::clone(&engine.heap);

        Ok(Self { unit, _heap: heap })
    }

    /// Writes the main chunk, every function, and every concrete method.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when writing fails.
    pub fn write_to<W>(&self, output: &mut W) -> io::Result<()>
    where
        W: Write,
    {
        output.write_all(disassemble::disassemble(&self.unit.main, "main").as_bytes())?;
        for function in &self.unit.functions {
            let name = function.name.to_string_lossy();
            output.write_all(b"\n")?;
            output.write_all(disassemble::disassemble(&function.chunk, &name).as_bytes())?;
        }

        for class in &self.unit.classes {
            let class_name = class.name.to_string_lossy();
            for method in &class.methods {
                if method.is_abstract {
                    continue;
                }

                let name = format!("{class_name}::{}", method.name.to_string_lossy());
                output.write_all(b"\n")?;
                output.write_all(
                    disassemble::disassemble(&method.function.chunk, &name).as_bytes(),
                )?;
            }
        }

        output.flush()
    }
}

/// A compiler rejection encountered while producing a disassembly.
#[derive(Debug)]
pub struct DisassemblyError(CompileError);

impl DisassemblyError {
    /// Every labelled source span, primary first, ready for diagnostic rendering.
    #[must_use]
    pub fn labels(&self) -> Vec<(Span, &str)> {
        self.0.labels()
    }
}

impl From<CompileError> for DisassemblyError {
    fn from(error: CompileError) -> Self {
        Self(error)
    }
}

impl fmt::Display for DisassemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.message)
    }
}

impl Error for DisassemblyError {}
