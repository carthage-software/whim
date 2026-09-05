//! Removal of side-effect-free writes whose produced value is never read.

use hashbrown::HashSet;

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::LivenessQueries;
use crate::optimizer::operands::for_each_write_register;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::TypeFlow;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

pub(in crate::optimizer) fn optimize_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> bool {
    if !configuration.dead_store {
        return false;
    }

    let mut changed = false;
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::DEAD_STORE) {
            continue;
        }

        let remove = removable_stores_with_flow(
            analyzed.chunk,
            Some(&analyzed.flow),
            analyzed.incoming_register_count,
        );
        if !remove.iter().any(|removed| *removed) {
            continue;
        }

        let removed = remove.iter().filter(|removed| **removed).count();
        for (index, removed) in remove.iter().copied().enumerate() {
            if removed {
                plan.remove(analyzed, index);
            }
        }
        statistics.instructions_removed += removed;
        changed = true;
    }

    changed
}

#[expect(
    clippy::too_many_arguments,
    reason = "the function context mirrors TypeFlow's entry facts"
)]
pub(in crate::optimizer::passes) fn optimize_chunk_with_context(
    chunk: &mut Chunk,
    parameters: &[CompiledParameter],
    has_receiver: bool,
    class_name: Option<&Atom>,
    class_type_parameters: &[CompiledTypeParameter],
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.dead_store || chunk.code.len() < 2 {
        return;
    }

    let remove = removable_stores(
        chunk,
        parameters,
        has_receiver,
        class_name,
        class_type_parameters,
        allocator,
    );
    remove_stores(chunk, &remove, statistics);
}

pub(in crate::optimizer::passes) fn optimize_chunk_without_flow(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.dead_store || chunk.code.len() < 2 {
        return;
    }
    let remove = removable_stores_with_flow(chunk, None, chunk.local_register_count);
    remove_stores(chunk, &remove, statistics);
}

fn removable_stores<'a>(
    chunk: &'a Chunk,
    parameters: &'a [CompiledParameter],
    has_receiver: bool,
    class_name: Option<&'a Atom>,
    class_type_parameters: &'a [CompiledTypeParameter],
    allocator: &'a Heap,
) -> Vec<bool> {
    if chunk.code.len() < 2 {
        return vec![false; chunk.code.len()];
    }

    let flow = (chunk.register_count != 0).then(|| {
        TypeFlow::analyze(
            chunk,
            parameters,
            has_receiver,
            class_name,
            class_type_parameters,
            allocator,
        )
    });
    removable_stores_with_flow(chunk, flow.as_ref(), chunk.local_register_count)
}

fn removable_stores_with_flow(
    chunk: &Chunk,
    flow: Option<&TypeFlow<'_>>,
    first_fresh_local: u16,
) -> Vec<bool> {
    let mut remove = vec![false; chunk.code.len()];
    let targets = control_flow_targets(chunk);
    let previous_values = PreviousValueSafety::analyze(chunk, &targets, first_fresh_local);
    let query_count = chunk
        .code
        .iter()
        .filter(|instruction| dead_store_candidate(**instruction))
        .count()
        * 2;
    let liveness = LivenessQueries::for_chunk(chunk, query_count);
    for (index, instruction) in chunk.code.iter().enumerate() {
        let destination = match *instruction {
            Instruction::Move {
                destination,
                source,
            } if scalar_write_is_unobservable(
                chunk,
                &targets,
                flow,
                &previous_values,
                index,
                destination,
            ) && (register_is_scalar_only(chunk, source)
                || flow
                    .as_ref()
                    .is_some_and(|flow| !flow.register_may_release_observably(index, source))) =>
            {
                destination
            }
            Instruction::LoadConstant { destination, .. }
            | Instruction::LoadNull { destination }
            | Instruction::LoadTrue { destination }
            | Instruction::LoadFalse { destination }
            | Instruction::LoadInt { destination, .. }
                if scalar_write_is_unobservable(
                    chunk,
                    &targets,
                    flow,
                    &previous_values,
                    index,
                    destination,
                ) =>
            {
                destination
            }
            Instruction::MoveOwned {
                destination,
                source,
            } if register_is_dead(chunk, &liveness, source, index + 1)
                && scalar_write_is_unobservable(
                    chunk,
                    &targets,
                    flow,
                    &previous_values,
                    index,
                    destination,
                )
                && (register_is_scalar_only(chunk, source)
                    || flow.is_some_and(|flow| {
                        !flow.register_may_release_observably(index, source)
                    })) =>
            {
                destination
            }
            Instruction::Clear { target }
                if flow
                    .is_some_and(|flow| !flow.register_may_release_observably(index, target))
                    || previous_values.cannot_own_reference(index, target) =>
            {
                target
            }
            _ => continue,
        };

        if register_is_dead(chunk, &liveness, destination, index + 1) {
            remove[index] = true;
        }
    }

    remove
}

fn dead_store_candidate(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Move { .. }
            | Instruction::MoveOwned { .. }
            | Instruction::LoadConstant { .. }
            | Instruction::LoadNull { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::LoadInt { .. }
            | Instruction::Clear { .. }
    )
}

fn register_is_dead(
    chunk: &Chunk,
    liveness: &LivenessQueries,
    register: Register,
    start: usize,
) -> bool {
    liveness.register_is_dead_after(chunk, register, start)
}

fn remove_stores(
    chunk: &mut Chunk,
    remove: &[bool],
    statistics: &mut OptimizationStatistics,
) -> bool {
    compact_removed_instructions(chunk, remove, statistics) != 0
}

pub(in crate::optimizer::passes) fn scalar_write_is_unobservable(
    chunk: &Chunk,
    targets: &HashSet<usize>,
    flow: Option<&TypeFlow<'_>>,
    previous_values: &PreviousValueSafety,
    index: usize,
    register: Register,
) -> bool {
    register_is_scalar_only(chunk, register)
        || flow.is_some_and(|flow| !flow.register_may_release_observably(index, register))
        || previous_values.cannot_own_reference(index, register)
        || continuation_proves_boolean(chunk, targets, index, register)
}

fn continuation_proves_boolean(
    chunk: &Chunk,
    targets: &HashSet<usize>,
    index: usize,
    register: Register,
) -> bool {
    if index == 0 || targets.contains(&index) {
        return false;
    }
    matches!(
        chunk.code[index - 1],
        Instruction::JumpIfFalse { condition, .. } | Instruction::JumpIfTrue { condition, .. }
            if condition == register
    )
}

fn register_is_scalar_only(chunk: &Chunk, register: Register) -> bool {
    chunk.register_count <= REFERENCE_REGISTER_LIMIT
        && chunk.reference_register_mask & (1u64 << u32::from(register.index())) == 0
}

pub(in crate::optimizer::passes) struct PreviousValueSafety {
    safe_before: Vec<Option<Register>>,
}

impl PreviousValueSafety {
    pub(in crate::optimizer::passes) fn analyze(
        chunk: &Chunk,
        targets: &HashSet<usize>,
        first_fresh_local: u16,
    ) -> Self {
        let mut safe_before = vec![None; chunk.code.len()];
        let mut states = vec![None; usize::from(chunk.register_count)];
        let mut segment = 0usize;
        let mut edges = Vec::new();

        for (index, instruction) in chunk.code.iter().copied().enumerate() {
            if targets.contains(&index) {
                segment += 1;
            }

            let mut first_write = None;
            let mut multiple_writes = false;
            let classified = for_each_write_register(instruction, |register| {
                if first_write.replace(register).is_some() {
                    multiple_writes = true;
                }
            });
            if !multiple_writes
                && let Some(register) = first_write
                && value_is_safe_before(chunk, &states, segment, register, first_fresh_local)
            {
                safe_before[index] = Some(register);
            }

            if classified {
                let safe = writes_non_owning_value(instruction);
                for_each_write_register(instruction, |register| {
                    states[usize::from(register.index())] = Some((segment, safe));
                });
            } else {
                segment += 1;
            }

            record_implicit_writes(chunk, instruction, &mut states, segment);
            edges.clear();
            successors(chunk, index, &mut edges);
            if index + 1 < chunk.code.len() && !edges.contains(&(index + 1)) {
                segment += 1;
            }
        }

        Self { safe_before }
    }

    pub(in crate::optimizer::passes) fn cannot_own_reference(
        &self,
        index: usize,
        register: Register,
    ) -> bool {
        self.safe_before.get(index) == Some(&Some(register))
    }
}

fn value_is_safe_before(
    chunk: &Chunk,
    states: &[Option<(usize, bool)>],
    segment: usize,
    register: Register,
    first_fresh_local: u16,
) -> bool {
    if chunk.trace_argument_registers.contains(&register) {
        return false;
    }

    states[usize::from(register.index())].map_or_else(
        || register.index() >= first_fresh_local,
        |(known_segment, safe)| known_segment == segment && safe,
    )
}

fn writes_non_owning_value(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::LoadConstant { .. }
            | Instruction::LoadNull { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::LoadInt { .. }
            | Instruction::VecIndexGet {
                value_mode: ArrayValueMode::Int,
                ..
            }
            | Instruction::DictIndexGetIntKey {
                value_mode: ArrayValueMode::Int,
                ..
            }
    )
}

fn record_implicit_writes(
    chunk: &Chunk,
    instruction: Instruction,
    states: &mut [Option<(usize, bool)>],
    segment: usize,
) {
    let argument_window = match instruction {
        Instruction::CallValue {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallValueUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallValueDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamed {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamedDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamedUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethod {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethodDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethodUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallStatic {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallStaticDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallSelfUnchecked {
            argument_count,
            first_argument,
            ..
        } => Some((first_argument, usize::from(argument_count.value()))),
        Instruction::CallWithNames {
            callee, descriptor, ..
        }
        | Instruction::CallWithNamesDiscarded {
            callee, descriptor, ..
        } => {
            let descriptor = &chunk.call_descriptors[usize::from(descriptor.index())];
            Some((
                Register::new(callee.index() + 1),
                usize::from(descriptor.positional) + descriptor.named.len(),
            ))
        }
        _ => None,
    };
    if let Some((first, count)) = argument_window {
        for offset in 0..count {
            states[usize::from(first.index()) + offset] = Some((segment, false));
        }
    }

    if let Instruction::MoveOwned { source, .. } = instruction {
        states[usize::from(source.index())] = Some((segment, false));
    }
    if let Instruction::Clear { target } = instruction {
        states[usize::from(target.index())] = Some((segment, true));
    }
    if let Instruction::PropertySetUnchecked {
        value, value_mode, ..
    } = instruction
        && value_mode.moves()
    {
        states[usize::from(value.index())] = Some((segment, true));
    }
}
