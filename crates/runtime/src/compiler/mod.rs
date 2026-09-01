//! The Whim compiler: a parsed program to a compiled unit.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "the private compiler module exposes one crate-wide boundary"
)]

use whim_span::HasSpan;
use whim_syn::cst::Program;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::unit::CompiledBuiltInFunction;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::optimize_unit;
use crate::symbols::line_starts_of;
use crate::value::heap::Heap;

mod declarations;
mod embed;
mod emit;
mod error;
mod limits;
mod names;
mod registers;
mod rules;
mod types;

use crate::compiler::declarations::generics::collect_generics;
pub(crate) use crate::compiler::declarations::generics::extend_generics;
use crate::compiler::declarations::sealed::validate_sealed_permissions;
pub(crate) use crate::compiler::embed::EmbeddedFiles;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::BodyShape;
use crate::compiler::emit::ReturnKind;
use crate::compiler::emit::Scope;
use crate::compiler::emit::analysis::collect_scoped_bindings_in_statement;
use crate::compiler::emit::analysis::collect_variables_in_statement;
pub(crate) use crate::compiler::error::CompileError;
use crate::compiler::types::bounds::validate_static_type_argument_bounds;

pub(crate) use crate::compiler::types::AliasGraph;
pub(crate) use crate::compiler::types::GenericTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct CompileConfiguration {
    pub optimization: OptimizationConfiguration,
    /// Whether the source is trusted code whose written return types are
    /// guaranteed by review, so returns compile unchecked. Only the standard
    /// library build sets this.
    pub trusted_return_types: bool,
}

#[cfg(test)]
pub(crate) fn compile_with_configuration(
    program: &Program<'_>,
    path: &str,
    heap: &Heap,
    configuration: CompileConfiguration,
) -> Result<CompiledUnit, CompileError> {
    compile_with_path_bytes_and_configuration(program, path, path.as_bytes(), heap, configuration)
}

#[cfg(test)]
pub(crate) fn compile_with_path_bytes_and_configuration(
    program: &Program<'_>,
    diagnostic_path: &str,
    runtime_path: &[u8],
    heap: &Heap,
    configuration: CompileConfiguration,
) -> Result<CompiledUnit, CompileError> {
    compile_with_path_bytes_configuration_and_built_in_functions(
        program,
        diagnostic_path,
        runtime_path,
        heap,
        configuration,
        &[],
    )
}

pub(crate) fn compile_with_path_bytes_configuration_and_built_in_functions(
    program: &Program<'_>,
    diagnostic_path: &str,
    runtime_path: &[u8],
    heap: &Heap,
    configuration: CompileConfiguration,
    built_in_functions: &[CompiledBuiltInFunction],
) -> Result<CompiledUnit, CompileError> {
    let mut unit = new_unit(runtime_path, heap);

    let generics = collect_generics(program);
    let mut aliases = AliasGraph::default();
    let embedded_files = EmbeddedFiles::default();
    let line_starts = line_starts_of(program.source_text);
    let mut compilation = Compilation::new(
        &generics,
        &mut aliases,
        &embedded_files,
        &line_starts,
        configuration.trusted_return_types,
    );
    unit.main = compile_program_into_unit(
        heap,
        program,
        CompilePath {
            diagnostic: diagnostic_path,
            runtime: runtime_path,
        },
        &mut unit,
        &mut compilation,
    )?;
    finish_unit(
        unit,
        &aliases,
        heap,
        built_in_functions,
        configuration.optimization,
    )
}

#[derive(Clone, Copy)]
pub(crate) struct CompilePath<'path> {
    pub(crate) diagnostic: &'path str,
    pub(crate) runtime: &'path [u8],
}

pub(crate) struct Compilation<'compilation, 'arena> {
    generics: &'compilation GenericTable<'arena>,
    aliases: &'compilation mut AliasGraph,
    embedded_files: &'compilation EmbeddedFiles,
    line_starts: &'compilation [u32],
    trusted_return_types: bool,
}

impl<'compilation, 'arena> Compilation<'compilation, 'arena> {
    pub(crate) const fn new(
        generics: &'compilation GenericTable<'arena>,
        aliases: &'compilation mut AliasGraph,
        embedded_files: &'compilation EmbeddedFiles,
        line_starts: &'compilation [u32],
        trusted_return_types: bool,
    ) -> Self {
        Self {
            generics,
            aliases,
            embedded_files,
            line_starts,
            trusted_return_types,
        }
    }
}

pub(crate) fn new_unit(runtime_path: &[u8], heap: &Heap) -> CompiledUnit {
    CompiledUnit {
        path: heap.intern(runtime_path),
        main: Chunk::new(),
        functions: Vec::new(),
        classes: Vec::new(),
        constants: Vec::new(),
        type_aliases: Vec::new(),
        newtypes: Vec::new(),
    }
}

pub(crate) fn finish_unit(
    mut unit: CompiledUnit,
    aliases: &AliasGraph,
    heap: &Heap,
    built_in_functions: &[CompiledBuiltInFunction],
    optimization: OptimizationConfiguration,
) -> Result<CompiledUnit, CompileError> {
    declarations::validate_alias_cycles(aliases)?;
    validate_sealed_permissions(&unit)?;
    validate_static_type_argument_bounds(&unit)?;
    optimize_unit(&mut unit, &[], built_in_functions, heap, optimization);
    Ok(unit)
}

pub(crate) fn compile_program_into_unit<'arena>(
    heap: &Heap,
    program: &Program<'arena>,
    path: CompilePath<'_>,
    unit: &mut CompiledUnit,
    compilation: &mut Compilation<'_, 'arena>,
) -> Result<Chunk, CompileError> {
    let regions = declarations::collect(heap, program, path, unit, compilation)?;

    let mut compiler = BodyCompiler::new(
        heap,
        path.diagnostic,
        path.runtime,
        program.source_text,
        &mut unit.functions,
        &unit.type_aliases,
        BodyShape {
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
                generics: compilation.generics,
                embedded_files: compilation.embedded_files,
                trusted_returns: compilation.trusted_return_types,
            };

            compiler.statement_public(&scope, statement)?;
        }
    }

    Ok(compiler.finish(program.span()))
}
