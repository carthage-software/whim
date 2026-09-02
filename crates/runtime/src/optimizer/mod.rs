//! Semantics-preserving optimization of compiled bytecode.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledBuiltInFunction;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::optimizer::passes::inline_leaf_calls::InlineChanges;
use crate::optimizer::passes::specialize_comparison;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::IndexedUnit;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

mod analysis;
mod callable;
mod candidates;
mod cfg;
mod live;
mod liveness;
mod operands;
mod passes;
mod rewrite;
mod type_flow;

pub(crate) use callable::optimize_function as optimize_callable_function;
pub(crate) use callable::optimize_method as optimize_callable_method;
pub(crate) use cfg::relative_target;
pub(crate) use live::Refinement as LiveRefinement;
pub(crate) use live::refine as refine_live_chunk;
pub(crate) use type_flow::World;
pub(crate) use type_flow::WorldCache;
pub(crate) use type_flow::descriptor_proves;
pub(crate) use type_flow::descriptors_equal;

#[cfg(test)]
mod integration_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OptimizationConfiguration {
    /// Whether optimization runs at all.
    pub enabled: bool,
    /// Fold operations whose result is known at compile time.
    pub const_fold: bool,
    /// Reuse dominating pure expressions whose inputs remain unchanged.
    pub cse: bool,
    /// Coalesce a producer followed by a temporary-to-local move.
    pub move_coalescing: bool,
    /// Replace straight-line reads of a copied value with its stable source.
    pub copy_propagation: bool,
    /// Transfer ownership when a move's source is dead on every path.
    pub ownership_moves: bool,
    /// Remove side-effect-free register writes whose values are never read.
    pub dead_store: bool,
    /// Avoid materializing a specialized foreach key that the body never reads.
    pub elide_foreach_keys: bool,
    /// Elide runtime type checks whose input is proven to satisfy the type.
    pub elide_type_checks: bool,
    /// Elide zero-argument constructor dispatch for classes with no constructor.
    pub elide_empty_constructor: bool,
    /// Elide virtual dispatch and parameter checks at proven exact call sites.
    pub elide_parameter_checks: bool,
    /// Elide discarded-result checks for callables proven not to require them.
    pub elide_discarded_checks: bool,
    /// Elide property mutability and type checks proven by whole-unit flow.
    pub elide_property_checks: bool,
    /// Fuse a comparison whose only consumer is a conditional jump.
    pub fuse_comparison: bool,
    /// Fuse a one-literal exact function call with its adjacent literal load.
    pub fuse_call_constant: bool,
    /// Fuse literal loads into adjacent float and comparison consumers.
    pub fuse_float_constants: bool,
    /// Fuse integer literal loads into adjacent proven comparisons.
    pub fuse_int_constants: bool,
    /// Fuse two adjacent sequential float updates into one dispatch.
    pub fuse_float_pair_update: bool,
    /// Fuse zero-based integer loops that fill one indexed property.
    pub fuse_fill_loop: bool,
    /// Fuse exact sequential float multiply/add and subtract/add chains.
    pub fuse_muladd: bool,
    /// Fuse the increment, comparison, and back edge of counted loops.
    pub fuse_counter_loop: bool,
    /// Fuse an in-place immediate increment with its following jump.
    pub fuse_increment_jump: bool,
    /// Return two-element tuples without an intermediate tuple instruction.
    pub fuse_return_pair: bool,
    /// Fuse indexed compound addition into one container lookup.
    pub fuse_index_add_assign: bool,
    /// Execute closed numeric counted loops with unboxed scalar registers.
    pub fuse_numeric_loop: bool,
    /// Inline public literal class constants declared in the same unit.
    pub inline_class_constants: bool,
    /// Splice tiny straight-line leaf functions into proven call sites.
    pub inline_leaf_calls: bool,
    /// Fuse indexed property increments into direct property-container writes.
    pub fuse_property_index_increment: bool,
    /// Fuse scalar property increments and additions into checked writes.
    pub fuse_property_update: bool,
    /// Fuse adjacent writes that initialize one fresh object.
    pub fuse_property_initialization: bool,
    /// Fuse adjacent square operations with consecutive destinations.
    pub fuse_squares: bool,
    /// Fuse adjacent float squares with their immediate sum.
    pub fuse_square_sum: bool,
    /// Fuse a float square sum with its constant-consuming branch.
    pub fuse_square_sum_branch: bool,
    /// Hoist immutable loop bounds into dedicated registers.
    pub licm: bool,
    /// Retarget unconditional jumps through unconditional jump chains.
    pub jump_threading: bool,
    /// Remove moves whose source and destination are the same register.
    pub self_move: bool,
    /// Reuse non-overlapping compiler temporary registers.
    pub reuse_temporaries: bool,
    /// Replace fresh, non-escaping objects with property registers.
    pub scalar_replace_objects: bool,
    /// Sink pure scalar producers into later temporary-consuming moves.
    pub sink_move: bool,
    /// Replace proven integer operations with smaller immediate forms.
    pub strength_reduction: bool,
    /// Replace arithmetic over proven scalar types with unchecked opcodes.
    pub specialize_arithmetic: bool,
    /// Replace proven vec and dict operations with direct collection opcodes.
    pub specialize_arrays: bool,
    /// Replace comparisons over proven integers with direct integer opcodes.
    pub specialize_comparison: bool,
    /// Replace proven exact property reads with direct slot accesses.
    pub specialize_property_get: bool,
    /// Specialize counted loops whose counter and limit are proven integers.
    pub specialize_counter_loop: bool,
    /// Reserve fresh arrays populated by proven counted loops.
    pub reserve_counted_arrays: bool,
    /// Move straight-line branches that call cold symbols behind the hot exit.
    pub cold_block_layout: bool,
    /// The first function index the pipeline may rewrite.
    pub immutable_function_floor: usize,
    /// The first class index the pipeline may rewrite.
    pub immutable_class_floor: usize,
    /// The first constant index the pipeline may rewrite.
    pub immutable_constant_floor: usize,
}

impl OptimizationConfiguration {
    /// The first function index the pipeline may rewrite.
    #[must_use]
    pub(crate) fn function_floor(&self, total: usize) -> usize {
        self.immutable_function_floor.min(total)
    }

    /// The first class index the pipeline may rewrite.
    #[must_use]
    pub(crate) fn class_floor(&self, total: usize) -> usize {
        self.immutable_class_floor.min(total)
    }

    /// The first constant index the pipeline may rewrite.
    #[must_use]
    pub(crate) fn constant_floor(&self, total: usize) -> usize {
        self.immutable_constant_floor.min(total)
    }
}

impl Default for OptimizationConfiguration {
    fn default() -> Self {
        Self {
            enabled: true,
            const_fold: true,
            cse: true,
            move_coalescing: true,
            copy_propagation: true,
            ownership_moves: true,
            dead_store: true,
            elide_foreach_keys: true,
            elide_type_checks: true,
            elide_empty_constructor: true,
            elide_parameter_checks: true,
            elide_discarded_checks: true,
            elide_property_checks: true,
            fuse_comparison: true,
            fuse_call_constant: true,
            fuse_float_constants: true,
            fuse_int_constants: true,
            fuse_float_pair_update: true,
            fuse_fill_loop: true,
            fuse_muladd: true,
            fuse_counter_loop: true,
            fuse_increment_jump: true,
            fuse_return_pair: true,
            fuse_index_add_assign: true,
            fuse_numeric_loop: true,
            inline_class_constants: true,
            inline_leaf_calls: true,
            fuse_property_index_increment: true,
            fuse_property_update: true,
            fuse_property_initialization: true,
            fuse_squares: true,
            fuse_square_sum: true,
            fuse_square_sum_branch: true,
            licm: true,
            jump_threading: true,
            self_move: true,
            reuse_temporaries: true,
            scalar_replace_objects: true,
            sink_move: true,
            strength_reduction: true,
            specialize_arithmetic: true,
            specialize_arrays: true,
            specialize_comparison: true,
            specialize_property_get: true,
            specialize_counter_loop: true,
            reserve_counted_arrays: true,
            cold_block_layout: true,
            immutable_function_floor: 0,
            immutable_class_floor: 0,
            immutable_constant_floor: 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OptimizationStatistics {
    /// The number of operations replaced by a compile-time constant.
    pub constants_folded: usize,
    /// The number of repeated pure expressions replaced by an existing value.
    pub common_subexpressions_eliminated: usize,
    /// The number of dispatchable instructions removed from all chunks.
    pub instructions_removed: usize,
    /// The number of runtime type checks proven redundant.
    pub type_checks_elided: usize,
    /// The number of runtime discarded-result checks proven redundant.
    pub discarded_checks_elided: usize,
    /// The number of exact method calls specialized by whole-unit type flow.
    pub parameter_checks_elided: usize,
    /// The number of property write checks proven redundant.
    pub property_checks_elided: usize,
    /// The number of exact property reads specialized.
    pub property_gets_specialized: usize,
    /// The number of arithmetic operations specialized by proven operand type.
    pub operations_specialized: usize,
    /// The number of collection operations specialized by proven container type.
    pub array_operations_specialized: usize,
    /// The number of specialized collection loops whose unused key was elided.
    pub foreach_keys_elided: usize,
    /// The number of frame registers removed by temporary reuse.
    pub registers_removed: usize,
    /// The number of fresh object allocations replaced by scalar registers.
    pub objects_scalar_replaced: usize,
    /// The number of proven leaf calls spliced into their callers.
    pub calls_inlined: usize,
    /// The number of cold branch bodies moved behind their hot continuation.
    pub cold_blocks_relocated: usize,
}

impl OptimizationStatistics {
    fn specialized_total(&self) -> usize {
        self.constants_folded
            + self.operations_specialized
            + self.array_operations_specialized
            + self.property_gets_specialized
            + self.parameter_checks_elided
            + self.property_checks_elided
    }
}

/// Runs every pass that only replaces instructions one for one against a
/// single analysis of the unit, and repeats while the replacements it applied
/// let the next round prove more.
fn specialize_against_one_analysis(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for _ in 0..SPECIALIZATION_ROUNDS {
        let before = statistics.specialized_total();
        specialize_comparison::prepare_unit(unit, configuration, statistics);
        match plan_specialization_round(unit, world, allocator, configuration, statistics) {
            SpecializationRound::Repeat(plan) => {
                plan.apply(unit);
            }
            SpecializationRound::ConstantsRemoved(plan) => {
                plan.apply(unit);
                if statistics.specialized_total() == before {
                    break;
                }
            }
            SpecializationRound::Complete {
                plan,
                dead_stores_removed,
            } => {
                let result = plan.apply(unit);
                if !dead_stores_removed
                    && (result.replacements == 0 || statistics.specialized_total() == before)
                {
                    break;
                }
            }
        }
    }
}

enum SpecializationRound {
    Repeat(RewritePlan),
    ConstantsRemoved(RewritePlan),
    Complete {
        plan: RewritePlan,
        dead_stores_removed: bool,
    },
}

fn plan_specialization_round(
    unit: &CompiledUnit,
    world: &World<'_>,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> SpecializationRound {
    let indexed = IndexedUnit::with_world(unit, world);
    let analysis = analysis::Analysis::of(&indexed, configuration, allocator);
    let mut plan = RewritePlan::for_analysis(&analysis);

    let comparison_removed = specialize_comparison::remove_boolean_literal_branches_unit(
        &analysis,
        &mut plan,
        configuration,
        statistics,
    ) | specialize_comparison::specialize_reused_string_bytes_unit(
        &analysis,
        &mut plan,
        configuration,
        statistics,
    );

    if comparison_removed {
        passes::dead_store::optimize_unit(&analysis, &mut plan, configuration, statistics);
        return SpecializationRound::Repeat(plan);
    }

    passes::elide_parameter_checks::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::elide_property_checks::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::specialize_property_get::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::specialize_arrays::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::specialize_arithmetic::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::const_fold::optimize_unit(&analysis, &mut plan, configuration, statistics);
    passes::strength_reduction::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::specialize_counter_loop::optimize_unit(&mut plan, &analysis, configuration, statistics);
    specialize_comparison::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::elide_type_checks::optimize_unit(&mut plan, &analysis, configuration, statistics);
    passes::ownership_moves::optimize_unit(&mut plan, &analysis, configuration);
    let constants_removed =
        passes::const_fold::remove_unit(&analysis, &mut plan, configuration, statistics);
    if constants_removed {
        return SpecializationRound::ConstantsRemoved(plan);
    }

    let discarded_removed = passes::elide_discarded_checks::optimize_unit(
        &analysis,
        &mut plan,
        configuration,
        statistics,
    );
    if discarded_removed {
        return SpecializationRound::Repeat(plan);
    }

    let dead_stores_removed =
        passes::dead_store::optimize_unit(&analysis, &mut plan, configuration, statistics);
    SpecializationRound::Complete {
        plan,
        dead_stores_removed,
    }
}

fn specialize_operations_against_one_analysis(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    let plan = {
        let indexed = IndexedUnit::with_world(unit, world);
        let analysis = analysis::Analysis::of_early_operations(&indexed, configuration, allocator);
        let mut plan = RewritePlan::for_analysis(&analysis);
        passes::specialize_arrays::optimize_unit(&mut plan, &analysis, configuration, statistics);
        passes::specialize_arithmetic::optimize_unit(
            &mut plan,
            &analysis,
            configuration,
            statistics,
        );

        plan
    };

    plan.apply(unit);
}

/// How many times the specializing passes may prove more and run again.
const SPECIALIZATION_ROUNDS: usize = 8;

fn optimize_class(
    class: &mut CompiledClassLike,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_attributes(&mut class.attributes, allocator, configuration, statistics);
    for constant in &mut class.constants {
        optimize_initializer(
            &mut constant.initializer,
            allocator,
            configuration,
            statistics,
        );
        optimize_attributes(
            &mut constant.attributes,
            allocator,
            configuration,
            statistics,
        );
    }
    for property in &mut class.properties {
        if let Some(default) = &mut property.default {
            optimize_initializer(default, allocator, configuration, statistics);
        }
        optimize_attributes(
            &mut property.attributes,
            allocator,
            configuration,
            statistics,
        );
    }
    for method in &mut class.methods {
        optimize_function(
            &mut method.function,
            Some(&class.name),
            &class.type_parameters,
            !method.is_static,
            allocator,
            configuration,
            statistics,
        );
    }
    for case in &mut class.cases {
        if let Some(value) = &mut case.value {
            optimize_initializer(value, allocator, configuration, statistics);
        }
    }
}

fn optimize_declarations(
    unit: &mut CompiledUnit,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk(&mut unit.main, allocator, configuration, statistics);

    let function_floor = configuration.function_floor(unit.functions.len());
    for function in &mut unit.functions[function_floor..] {
        optimize_function(
            function,
            None,
            &[],
            false,
            allocator,
            configuration,
            statistics,
        );
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for class in &mut unit.classes[class_floor..] {
        optimize_class(class, allocator, configuration, statistics);
    }

    let constant_floor = configuration.constant_floor(unit.constants.len());
    for constant in &mut unit.constants[constant_floor..] {
        optimize_initializer(
            &mut constant.initializer,
            allocator,
            configuration,
            statistics,
        );
    }
}

fn reoptimize_inlined_callables(
    unit: &mut CompiledUnit,
    changes: &InlineChanges,
    world: &World<'_>,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if changes.main {
        optimize_chunk(&mut unit.main, allocator, configuration, statistics);
    }
    for position in &changes.functions {
        optimize_function(
            &mut unit.functions[*position],
            None,
            &[],
            false,
            allocator,
            configuration,
            statistics,
        );
    }
    for (class_position, method_position) in &changes.methods {
        let class = &mut unit.classes[*class_position];
        let method = &mut class.methods[*method_position];
        optimize_function(
            &mut method.function,
            Some(&class.name),
            &class.type_parameters,
            !method.is_static,
            allocator,
            configuration,
            statistics,
        );
    }
    specialize_against_one_analysis(unit, world, allocator, configuration, statistics);
}

/// Optimizes every executable chunk in a compiled unit, allocating all
/// analysis state through the same `heap` that owns the unit's atoms.
pub(crate) fn optimize_unit(
    unit: &mut CompiledUnit,
    units: &[&CompiledUnit],
    built_in_functions: &[CompiledBuiltInFunction],
    heap: &Heap,
    configuration: OptimizationConfiguration,
) -> OptimizationStatistics {
    if !configuration.enabled {
        return OptimizationStatistics::default();
    }

    let world = World::new(units, built_in_functions);
    optimize_unit_with_world(unit, &world, heap, configuration)
}

pub(crate) fn optimize_unit_with_world(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    heap: &Heap,
    configuration: OptimizationConfiguration,
) -> OptimizationStatistics {
    let mut statistics = OptimizationStatistics::default();
    let has_destructor = passes::finalizer_boundaries::has_destructor(unit);

    analysis::annotate_capture_types(unit, world, heap);
    passes::ownership_moves::normalize_unit(unit, configuration);
    passes::inline_class_constants::optimize_unit(unit, configuration, &mut statistics);
    passes::elide_empty_constructor::optimize_unit(unit, configuration, &mut statistics);
    passes::fuse_lowered_property_updates(unit, configuration, &mut statistics);
    passes::specialize_lowered::optimize_unit(unit, configuration, &mut statistics);
    specialize_operations_against_one_analysis(unit, world, heap, configuration, &mut statistics);
    optimize_declarations(unit, heap, configuration, &mut statistics);
    passes::layout_cold_blocks::optimize_unit(unit, configuration, &mut statistics);
    passes::scalar_replace_objects::optimize_unit(unit, configuration, &mut statistics);
    specialize_against_one_analysis(unit, world, heap, configuration, &mut statistics);
    passes::fuse_exact_call_window::optimize_unit(unit, configuration, &mut statistics);
    passes::cse::optimize_unit(unit, configuration, &mut statistics);
    passes::fuse_return_pair::optimize_unit(unit, configuration, &mut statistics);
    passes::cse::optimize_unit(unit, configuration, &mut statistics);
    passes::fuse_int_constants::optimize_unit(unit, configuration, &mut statistics);
    passes::scalar_replace_objects::optimize_unit(unit, configuration, &mut statistics);
    passes::fuse_call_constant::optimize_unit(unit, configuration, &mut statistics);
    passes::duplicate_returns::optimize_unit(unit, configuration);
    passes::fuse_int_constants::optimize_unit(unit, configuration, &mut statistics);
    passes::refine_reference_registers::optimize_unit(unit, configuration);
    let inline_changes =
        passes::inline_leaf_calls::optimize_unit(unit, world, heap, configuration, &mut statistics);
    if inline_changes.any() {
        reoptimize_inlined_callables(
            unit,
            &inline_changes,
            world,
            heap,
            configuration,
            &mut statistics,
        );
    }
    passes::fuse_exact_call_window::optimize_unit(unit, configuration, &mut statistics);
    passes::move_coalescing::optimize_unit(unit, configuration, &mut statistics);
    passes::fuse_index_add_assign::optimize_unit(unit, configuration, &mut statistics);
    if !has_destructor {
        passes::hoist_string_property_reads::optimize_unit(unit, configuration, &mut statistics);
        passes::cse::optimize_unit(unit, configuration, &mut statistics);
    }
    passes::optimize_unit_numeric_loops(unit, configuration);
    passes::reuse_temporaries::optimize_unit(unit, configuration, &mut statistics);
    passes::finalize_property_moves::optimize_unit(unit, configuration);
    passes::refine_reference_registers::optimize_unit(unit, configuration);
    passes::prune_clears::optimize_unit(unit, configuration, &mut statistics);
    passes::elide_foreach_key::optimize_unit(unit, configuration, &mut statistics);
    passes::refine_reference_registers::optimize_unit(unit, configuration);
    passes::fuse_return_pair::optimize_owned_sources_unit(unit, configuration, &mut statistics);
    passes::duplicate_returns::optimize_unit(unit, configuration);
    passes::fuse_int_constants::optimize_unit(unit, configuration, &mut statistics);
    passes::refine_reference_registers::optimize_unit(unit, configuration);
    passes::fuse_property_initialization::optimize_unit(unit, configuration, &mut statistics);
    passes::scalar_replace_objects::optimize_unit(unit, configuration, &mut statistics);
    statistics
}

pub(crate) fn optimize_unit_entry(
    unit: &mut CompiledUnit,
    units: &[&CompiledUnit],
    built_in_functions: &[CompiledBuiltInFunction],
    heap: &Heap,
    mut configuration: OptimizationConfiguration,
) -> OptimizationStatistics {
    configuration.immutable_function_floor = unit.functions.len();
    configuration.immutable_class_floor = unit.classes.len();
    optimize_unit(unit, units, built_in_functions, heap, configuration)
}

fn optimize_function(
    function: &mut CompiledFunction,
    class_name: Option<&Atom>,
    class_type_parameters: &[CompiledTypeParameter],
    has_receiver: bool,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_attributes(
        &mut function.attributes,
        allocator,
        configuration,
        statistics,
    );
    for parameter in &mut function.parameters {
        optimize_attributes(
            &mut parameter.attributes,
            allocator,
            configuration,
            statistics,
        );
    }
    passes::optimize_function(function, configuration, statistics);
    passes::optimize_function_before_numeric_loop(
        function,
        class_name,
        class_type_parameters,
        has_receiver,
        allocator,
        configuration,
        statistics,
    );
    passes::reserve_counted_arrays::optimize_chunk(&mut function.chunk, configuration);
    passes::optimize_numeric_loop(&mut function.chunk, configuration);
    passes::optimize_integer_step_loops(&mut function.chunk, configuration, statistics);
    passes::fuse_int_constants::optimize_chunk(&mut function.chunk, configuration, statistics);
    passes::cse::optimize_chunk(&mut function.chunk, configuration, statistics);
}

fn optimize_attributes(
    attributes: &mut [CompiledAttribute],
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for attribute in attributes {
        for argument in &mut attribute.arguments {
            optimize_initializer(argument, allocator, configuration, statistics);
        }
        for (_, argument) in &mut attribute.named_arguments {
            optimize_initializer(argument, allocator, configuration, statistics);
        }
    }
}

fn optimize_initializer(
    initializer: &mut ConstantInitializer,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if let ConstantInitializer::Thunk(chunk) = initializer {
        optimize_initializer_chunk(chunk, allocator, configuration, statistics);
    }
}

fn optimize_initializer_chunk(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    passes::fuse_index_add_assign::optimize_chunk(chunk, configuration, statistics);
    passes::specialize_arrays::optimize_chunk(chunk, allocator, configuration, statistics);
    passes::specialize_arithmetic::optimize_chunk(chunk, allocator, configuration, statistics);
    passes::optimize_initializer_chunk_before_numeric_loop(
        chunk,
        allocator,
        configuration,
        statistics,
    );
    passes::specialize_counter_loop::optimize_chunk(chunk, allocator, configuration, statistics);
    optimize_chunk_after_numeric_loop_preparation(
        chunk,
        allocator,
        configuration,
        statistics,
        true,
    );
}

fn optimize_chunk(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    passes::fuse_index_add_assign::optimize_chunk(chunk, configuration, statistics);
    passes::optimize_chunk_before_numeric_loop(chunk, allocator, configuration, statistics);
    optimize_chunk_after_numeric_loop_preparation(
        chunk,
        allocator,
        configuration,
        statistics,
        false,
    );
}

fn optimize_chunk_after_numeric_loop_preparation(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
    analyze_locally: bool,
) {
    passes::reserve_counted_arrays::optimize_chunk(chunk, configuration);
    passes::optimize_numeric_loop(chunk, configuration);
    if analyze_locally {
        specialize_comparison::optimize_chunk(chunk, allocator, configuration, statistics);
    }
    passes::optimize_integer_step_loops(chunk, configuration, statistics);
    passes::fuse_int_constants::optimize_chunk(chunk, configuration, statistics);
    passes::cse::optimize_chunk(chunk, configuration, statistics);
    if analyze_locally {
        passes::reuse_temporaries::optimize_chunk(chunk, configuration, statistics);
        passes::elide_foreach_key::optimize_chunk(chunk, configuration, statistics);
        chunk.refresh_runtime_metadata();
    }
}
