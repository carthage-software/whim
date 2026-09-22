//! The Whim compiler: a parsed program to a compiled unit.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "compiler helpers stay private to the crate"
)]

use whim_bytecode::chunk::Chunk;
use whim_bytecode::unit::CompiledBuiltInFunction;
use whim_bytecode::unit::CompiledFile;
use whim_bytecode::unit::CompiledUnit;
use whim_optimizer::OptimizationConfiguration;
use whim_optimizer::optimize_unit;
use whim_span::HasSpan;
use whim_span::lines::line_starts_of;
use whim_syn::cst::Program;
use whim_value::heap::Heap;

mod declarations;
mod embed;
mod emit;
mod error;
mod limits;
mod names;
mod registers;
mod rules;
pub mod target;
mod types;

use crate::declarations::generics::extend_generics;
use crate::declarations::sealed::validate_sealed_permissions;
use crate::embed::EmbeddedFiles;
use crate::emit::BodyCompiler;
use crate::emit::BodyShape;
use crate::emit::ReturnKind;
use crate::emit::Scope;
use crate::emit::analysis::collect_scoped_bindings_in_statement;
use crate::emit::analysis::collect_variables_in_statement;
pub use crate::error::CompileError;
pub use crate::error::CompileErrorKind;
use crate::target::Target;
use crate::types::AliasGraph;
use crate::types::GenericTable;
use crate::types::bounds::validate_static_type_argument_bounds;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompileConfiguration {
    pub optimization: OptimizationConfiguration,
    /// Whether the source is trusted code whose written return types are
    /// guaranteed by review, so returns compile unchecked. Only the standard
    /// library build sets this.
    pub trusted_return_types: bool,
}

/// Compiles one parsed source file for the native platform.
///
/// # Errors
///
/// Returns an error when compilation or declaration checks fail.
pub fn compile_with_configuration(
    program: &Program<'_>,
    path: &str,
    heap: &Heap,
    configuration: CompileConfiguration,
) -> Result<CompiledUnit, CompileError> {
    compile_with_path_bytes_configuration_and_built_in_functions(
        program,
        path,
        path.as_bytes(),
        heap,
        configuration,
        &[],
    )
}

/// Compiles one parsed source file with its byte path and native function declarations.
///
/// # Errors
///
/// Returns an error when compilation or declaration checks fail.
pub fn compile_with_path_bytes_configuration_and_built_in_functions(
    program: &Program<'_>,
    diagnostic_path: &str,
    runtime_path: &[u8],
    heap: &Heap,
    configuration: CompileConfiguration,
    built_in_functions: &[CompiledBuiltInFunction],
) -> Result<CompiledUnit, CompileError> {
    let mut unit = new_unit(runtime_path, heap);

    let line_starts = line_starts_of(program.source_text);
    let mut compilation = Compilation::new(
        &[program],
        &Target::NATIVE,
        &line_starts,
        configuration.trusted_return_types,
    );
    unit.main = compilation.compile(
        heap,
        program,
        CompilePath {
            diagnostic: diagnostic_path,
            runtime: runtime_path,
        },
        &mut unit,
    )?;
    compilation.finish(unit, heap, built_in_functions, configuration.optimization)
}

#[derive(Clone, Copy)]
pub struct CompilePath<'path> {
    pub diagnostic: &'path str,
    pub runtime: &'path [u8],
}

/// Shared state for compiling several programs into one unit.
pub struct Compilation<'compilation, 'arena> {
    generics: GenericTable<'arena>,
    aliases: AliasGraph,
    embedded_files: EmbeddedFiles,
    target: &'compilation Target,
    line_starts: &'compilation [u32],
    trusted_return_types: bool,
}

impl<'compilation, 'arena> Compilation<'compilation, 'arena> {
    #[must_use]
    pub fn new(
        programs: &[&Program<'arena>],
        target: &'compilation Target,
        line_starts: &'compilation [u32],
        trusted_return_types: bool,
    ) -> Self {
        let mut generics = GenericTable::new();
        for program in programs {
            extend_generics(program, &mut generics);
        }

        Self {
            generics,
            aliases: AliasGraph::default(),
            embedded_files: EmbeddedFiles::default(),
            target,
            line_starts,
            trusted_return_types,
        }
    }

    /// Adds a program's declarations to the unit and returns its main chunk.
    ///
    /// # Errors
    ///
    /// Returns an error when the program fails a compiler check.
    pub fn compile(
        &mut self,
        heap: &Heap,
        program: &Program<'arena>,
        path: CompilePath<'_>,
        unit: &mut CompiledUnit,
    ) -> Result<Chunk, CompileError> {
        compile_program_into_unit(heap, program, path, unit, self)
    }

    /// Checks the complete unit and applies the requested optimizations.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid alias cycles, sealed permissions, or type arguments.
    pub fn finish(
        self,
        mut unit: CompiledUnit,
        heap: &Heap,
        built_in_functions: &[CompiledBuiltInFunction],
        optimization: OptimizationConfiguration,
    ) -> Result<CompiledUnit, CompileError> {
        declarations::validate_alias_cycles(&self.aliases)?;
        validate_sealed_permissions(&unit)?;
        validate_static_type_argument_bounds(&unit)?;
        optimize_unit(&mut unit, &[], built_in_functions, heap, optimization);
        Ok(unit)
    }
}

#[must_use]
pub fn new_unit(runtime_path: &[u8], heap: &Heap) -> CompiledUnit {
    CompiledUnit {
        path: heap.intern(runtime_path),
        files: Vec::new(),
        main: Chunk::new(),
        functions: Vec::new(),
        classes: Vec::new(),
        constants: Vec::new(),
        type_aliases: Vec::new(),
        newtypes: Vec::new(),
    }
}

fn compile_program_into_unit<'arena>(
    heap: &Heap,
    program: &Program<'arena>,
    path: CompilePath<'_>,
    unit: &mut CompiledUnit,
    compilation: &mut Compilation<'_, 'arena>,
) -> Result<Chunk, CompileError> {
    let mut regions = declarations::collect(heap, program, path, unit, compilation)?;
    let mut attributes = Vec::new();
    for region in &mut regions {
        attributes.append(&mut region.file_attributes);
    }

    limits::check_count(
        CompileErrorKind::TooManyAttributes,
        "one file may carry",
        "attributes",
        attributes.len(),
        attributes
            .last()
            .map_or_else(|| program.span(), |attribute| attribute.span),
    )?;

    unit.files.push(CompiledFile {
        path: (!path.runtime.is_empty() && path.runtime != b"-").then(|| heap.intern(path.runtime)),
        span: program.span(),
        attributes,
        has_top_level_code: regions
            .iter()
            .any(|region| !region.main_statements.is_empty()),
    });

    let mut compiler = BodyCompiler::new(
        heap,
        path.diagnostic,
        path.runtime,
        program.source_text,
        &mut unit.functions,
        &unit.type_aliases,
        BodyShape {
            where_clause: None,
            is_instance_method: false,
            return_kind: ReturnKind::Forbidden,
            promote_parameters: false,
            trusted_returns: compilation.trusted_return_types,
        },
    );

    for region in &regions {
        for (statement, _) in &region.main_statements {
            let names = collect_variables_in_statement(statement);
            for name in names {
                if name != "$this" {
                    compiler.declare_local(&name, false, statement.span())?;
                }
            }
            let mut bindings = Vec::new();
            collect_scoped_bindings_in_statement(statement, &mut bindings);
            compiler.declare_scoped_bindings(bindings)?;
        }
    }

    for region in &regions {
        for (statement, resolver) in &region.main_statements {
            let scope = Scope {
                heap,
                runtime_path: path.runtime,
                line_starts: compilation.line_starts,
                resolver,
                class: None,
                binders: Vec::new(),
                forbidden_binders: Vec::new(),
                generics: &compilation.generics,
                embedded_files: &compilation.embedded_files,
                target: compilation.target,
                trusted_returns: compilation.trusted_return_types,
            };

            compiler.statement_public(&scope, statement)?;
        }
    }

    Ok(compiler.finish(program.span()))
}
