//! Individually configurable bytecode optimization passes.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::rewrite::compact;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::TypeFlow;

use crate::value::atom::Atom;
use crate::value::heap::Heap;

/// Addresses one compiled chunk inside a unit.
#[derive(Clone, Copy)]
pub(in crate::optimizer) enum FunctionLocation {
    Main,
    Function(usize),
    Method { class: usize, method: usize },
}

/// Returns the chunk a location addresses.
pub(in crate::optimizer) fn chunk_mut(
    unit: &mut CompiledUnit,
    location: FunctionLocation,
) -> &mut Chunk {
    match location {
        FunctionLocation::Main => &mut unit.main,
        FunctionLocation::Function(index) => &mut unit.functions[index].chunk,
        FunctionLocation::Method { class, method } => {
            &mut unit.classes[class].methods[method].function.chunk
        }
    }
}

pub(in crate::optimizer::passes) fn for_each_mutable_chunk(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    mut operation: impl FnMut(&mut Chunk),
) {
    operation(&mut unit.main);

    let function_floor = configuration.function_floor(unit.functions.len());
    for function in &mut unit.functions[function_floor..] {
        operation(&mut function.chunk);
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for class in &mut unit.classes[class_floor..] {
        for method in &mut class.methods {
            operation(&mut method.function.chunk);
        }
    }
}

pub(in crate::optimizer::passes) fn compact_removed_instructions(
    chunk: &mut Chunk,
    remove: &[bool],
    statistics: &mut OptimizationStatistics,
) -> usize {
    let removed = remove.iter().filter(|removed| **removed).count();
    if removed != 0 {
        compact(chunk, remove);
        statistics.instructions_removed += removed;
    }
    removed
}

pub(in crate::optimizer::passes) fn plan_type_specializations(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    candidates: CandidateSet,
    specialize: for<'flow> fn(&TypeFlow<'flow>, usize, Instruction) -> Option<Instruction>,
) -> usize {
    let mut specialized = 0;
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(candidates) {
            continue;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) {
                continue;
            }

            let Some(replacement) = specialize(&analyzed.flow, index, instruction) else {
                continue;
            };

            specialized += usize::from(analyzed.write(plan, index, replacement));
        }
    }
    specialized
}

pub(in crate::optimizer::passes) fn specialize_chunk_instructions(
    chunk: &mut Chunk,
    heap: &Heap,
    specialize: for<'flow> fn(&TypeFlow<'flow>, usize, Instruction) -> Option<Instruction>,
) -> usize {
    let mut replacements = vec![None; chunk.code.len()];
    {
        let flow = TypeFlow::analyze(chunk, &[], false, None, &[], heap);
        for (index, instruction) in chunk.code.iter().copied().enumerate() {
            replacements[index] = specialize(&flow, index, instruction);
        }
    }

    let mut specialized = 0;
    for (instruction, replacement) in chunk.code.iter_mut().zip(replacements) {
        if let Some(replacement) = replacement {
            *instruction = replacement;
            specialized += 1;
        }
    }
    specialized
}

pub(super) mod const_fold;
mod copy_propagation;
mod fuse_comparison;
mod fuse_counter_loop;
mod fuse_fill_loop;
mod fuse_float_constants;
mod fuse_float_pair_update;
mod fuse_increment_jump;
pub(super) mod fuse_index_add_assign;
mod fuse_muladd;
mod fuse_numeric_loop;
mod fuse_property_index_update;
pub(super) mod fuse_property_initialization;
mod fuse_property_update;
mod fuse_square_sum;
mod fuse_square_sum_branch;
mod fuse_squares;
mod hoist_loop_constants;
mod hoist_loop_entry;
pub(super) mod hoist_string_property_reads;
mod jump_threading;
mod licm;
pub(super) mod move_coalescing;
mod sink_move;
pub(super) mod strength_reduction;

pub(super) mod cse;
pub(super) mod dead_store;
pub(super) mod duplicate_returns;
pub(super) mod elide_discarded_checks;
pub(super) mod elide_empty_constructor;
pub(super) mod elide_foreach_key;
pub(super) mod elide_parameter_checks;
pub(super) mod elide_property_checks;
pub(super) mod elide_type_checks;
pub(super) mod finalize_property_moves;
pub(super) mod finalizer_boundaries;
pub(super) mod fuse_call_constant;
pub(super) mod fuse_exact_call_window;
pub(super) mod fuse_int_constants;
pub(super) mod fuse_return_pair;
pub(super) mod inline_class_constants;
pub(super) mod inline_leaf_calls;
pub(super) mod layout_cold_blocks;
pub(super) mod ownership_moves;
pub(super) mod prune_clears;
pub(super) mod prune_unreachable;
pub(super) mod refine_reference_registers;
pub(super) mod reserve_counted_arrays;
pub(super) mod reuse_temporaries;
pub(super) mod scalar_replace_objects;
pub(super) mod specialize_arithmetic;
pub(super) mod specialize_arrays;
pub(super) mod specialize_comparison;
pub(super) mod specialize_counter_loop;
pub(super) mod specialize_lowered;
pub(super) mod specialize_property_get;

pub(in crate::optimizer) fn optimize_function(
    function: &mut CompiledFunction,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    fuse_index_add_assign::optimize_chunk(&mut function.chunk, configuration, statistics);
}

pub(in crate::optimizer) fn fuse_lowered_property_updates(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        fuse_property_index_update::optimize_chunk(chunk, configuration, statistics);
        fuse_property_update::optimize_chunk(chunk, configuration, statistics);
    });
}

pub(in crate::optimizer) fn optimize_chunk_before_numeric_loop(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk_before_numeric_loop_with_context(
        chunk,
        &[],
        false,
        None,
        &[],
        allocator,
        configuration,
        statistics,
        false,
    );
}

pub(in crate::optimizer) fn optimize_initializer_chunk_before_numeric_loop(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk_before_numeric_loop_with_context(
        chunk,
        &[],
        false,
        None,
        &[],
        allocator,
        configuration,
        statistics,
        true,
    );
}

pub(in crate::optimizer) fn optimize_isolated_live_tail(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk_before_numeric_loop_with_context(
        chunk,
        &[],
        false,
        None,
        &[],
        allocator,
        configuration,
        statistics,
        true,
    );
    reserve_counted_arrays::optimize_chunk(chunk, configuration);
    optimize_numeric_loop(chunk, configuration);
    optimize_integer_step_loops(chunk, configuration, statistics);
    fuse_int_constants::optimize_chunk(chunk, configuration, statistics);
    cse::optimize_chunk(chunk, configuration, statistics);
    reuse_temporaries::optimize_chunk(chunk, configuration, statistics);
    elide_foreach_key::optimize_chunk(chunk, configuration, statistics);
    chunk.refresh_runtime_metadata();
}

pub(in crate::optimizer) fn optimize_function_before_numeric_loop(
    function: &mut CompiledFunction,
    class_name: Option<&Atom>,
    class_type_parameters: &[CompiledTypeParameter],
    has_receiver: bool,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk_before_numeric_loop_with_context(
        &mut function.chunk,
        &function.parameters,
        function.captures_this || has_receiver,
        class_name,
        class_type_parameters,
        allocator,
        configuration,
        statistics,
        false,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "the function context mirrors TypeFlow's entry facts"
)]
fn optimize_chunk_before_numeric_loop_with_context(
    chunk: &mut Chunk,
    parameters: &[CompiledParameter],
    has_receiver: bool,
    class_name: Option<&Atom>,
    class_type_parameters: &[CompiledTypeParameter],
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
    analyze_locally: bool,
) {
    if analyze_locally {
        const_fold::optimize_chunk(chunk, allocator, configuration, statistics);
    } else {
        const_fold::prepare_chunk(chunk, configuration, statistics);
    }
    if analyze_locally {
        strength_reduction::optimize_chunk(chunk, allocator, configuration, statistics);
    }
    move_coalescing::optimize_chunk(chunk, configuration, statistics);
    copy_propagation::optimize_chunk(chunk, configuration, statistics);
    fuse_index_add_assign::optimize_chunk(chunk, configuration, statistics);
    fuse_property_index_update::optimize_chunk(chunk, configuration, statistics);
    fuse_property_update::optimize_chunk(chunk, configuration, statistics);
    fuse_squares::optimize_chunk(chunk, configuration, statistics);
    fuse_square_sum::optimize_chunk(chunk, configuration, statistics);
    fuse_comparison::optimize_chunk(chunk, configuration, statistics);
    jump_threading::optimize_chunk(chunk, configuration);
    if analyze_locally {
        dead_store::optimize_chunk_with_context(
            chunk,
            parameters,
            has_receiver,
            class_name,
            class_type_parameters,
            allocator,
            configuration,
            statistics,
        );
        const_fold::prepare_chunk(chunk, configuration, statistics);
    }
    fuse_int_constants::optimize_chunk(chunk, configuration, statistics);
    fuse_float_constants::optimize_chunk(chunk, configuration, statistics);
    fuse_square_sum_branch::optimize_chunk(chunk, configuration, statistics);
    fuse_muladd::optimize_chunk(chunk, configuration, statistics);
    sink_move::optimize_chunk(chunk, configuration, statistics);
    fuse_float_pair_update::optimize_chunk(chunk, configuration, statistics);
    fuse_increment_jump::optimize_chunk(chunk, configuration, statistics);
    licm::optimize_chunk(chunk, configuration);
    hoist_loop_constants::optimize_chunk(chunk, configuration);
    fuse_counter_loop::optimize_chunk(chunk, configuration, statistics);
    fuse_fill_loop::optimize_chunk(chunk, configuration, statistics);
}

pub(in crate::optimizer) fn optimize_numeric_loop(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
) {
    fuse_numeric_loop::optimize_chunk(chunk, configuration);
    hoist_loop_entry::optimize_chunk(chunk, configuration);
}

pub(in crate::optimizer) fn optimize_integer_step_loops(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    fuse_counter_loop::optimize_chunk(chunk, configuration, statistics);
}

pub(in crate::optimizer) fn optimize_unit_numeric_loops(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_numeric_loop(chunk, configuration);
    });
}
