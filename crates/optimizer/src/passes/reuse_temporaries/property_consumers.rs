use whim_bytecode::chunk::Chunk;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::PropertyReadMode;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::rewrite::control_flow_targets;
use whim_bytecode::unit::CompiledUnit;
use whim_value::heap::Heap;

use crate::OptimizationConfiguration;
use crate::analysis::Analysis;
use crate::analysis::AnalyzedChunk;
use crate::candidates::CandidateSet;
use crate::cfg::branches_or_terminates;
use crate::liveness::effect::effect_on;
use crate::liveness::register_is_dead_after;
use crate::passes::FunctionLocation;
use crate::rewrite::plan::RewritePlan;
use crate::type_flow::IndexedUnit;
use crate::type_flow::World;

pub(crate) fn optimize_unit(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    heap: &Heap,
    configuration: OptimizationConfiguration,
) {
    if !configuration.reuse_temporaries {
        return;
    }

    let plan = {
        let indexed = IndexedUnit::with_world(unit, world);
        let analysis = Analysis::of_property_consumers(&indexed, configuration, heap);
        let mut plan = RewritePlan::for_analysis(&analysis);
        for analyzed in analysis.chunks() {
            let chunk = analyzed.chunk;
            if !analyzed
                .candidates
                .contains(CandidateSet::PROPERTY_CONSUMER)
                || !chunk.catch_table.is_empty()
            {
                continue;
            }

            let targets = control_flow_targets(chunk);
            for index in 0..chunk.code.len().saturating_sub(1) {
                let Instruction::PropertyGetUnchecked {
                    destination: temporary,
                    object,
                    slot,
                    value_mode: PropertyReadMode::Clone,
                } = chunk.code[index]
                else {
                    continue;
                };
                let (consumer_index, literal) = match chunk.code[index + 1] {
                    Instruction::LoadInteger { destination, .. } => (index + 2, Some(destination)),
                    _ => (index + 1, None),
                };
                let Some(instruction) = chunk.code.get(consumer_index).copied() else {
                    continue;
                };
                let (destination, consumer) = match instruction {
                    Instruction::Length {
                        destination,
                        source,
                    } if source == temporary => (
                        destination,
                        Instruction::Length {
                            destination,
                            source: destination,
                        },
                    ),
                    Instruction::VecIndexGet {
                        destination,
                        container,
                        index,
                        value_mode,
                    } if container == temporary && index != destination && index != temporary => (
                        destination,
                        Instruction::VecIndexGet {
                            destination,
                            container: destination,
                            index,
                            value_mode,
                        },
                    ),
                    _ => continue,
                };
                let local_length = destination.index() < chunk.local_register_count
                    && literal.is_none()
                    && matches!(instruction, Instruction::Length { .. })
                    && analyzed.flow.proves_collection(consumer_index, temporary);
                if destination == temporary
                    || destination == object
                    || !temporary_register(chunk, analyzed.incoming_register_count, temporary)
                    || !unprotected_register(chunk, analyzed.incoming_register_count, destination)
                    || (destination.index() < chunk.local_register_count && !local_length)
                    || (index + 1..=consumer_index).any(|position| targets.contains(&position))
                    || !plan.is_available(analyzed, index)
                    || (index + 1..=consumer_index)
                        .any(|position| !plan.is_available(analyzed, position))
                    || literal.is_some_and(|register| {
                        register == temporary
                            || register == destination
                            || register == object
                            || analyzed
                                .flow
                                .register_may_release_observably(index + 1, register)
                    })
                    || !register_is_dead_after(chunk, temporary, consumer_index + 1)
                    || analyzed
                        .flow
                        .register_may_release_observably(index, temporary)
                    || (analyzed
                        .flow
                        .register_may_release_observably(consumer_index, destination)
                        && !(local_length
                            && !targets.iter().any(|target| *target <= consumer_index)
                            && first_local_assignment(analyzed, consumer_index, destination)))
                    || analyzed
                        .flow
                        .register_may_release_observably(consumer_index, temporary)
                {
                    continue;
                }

                analyzed.write(
                    &mut plan,
                    index,
                    Instruction::PropertyGetUnchecked {
                        destination,
                        object,
                        slot,
                        value_mode: PropertyReadMode::Clone,
                    },
                );
                analyzed.write(&mut plan, consumer_index, consumer);
            }
        }
        plan
    };
    plan.apply(unit);
}

fn first_local_assignment(analyzed: &AnalyzedChunk<'_>, before: usize, register: Register) -> bool {
    // Frame teardown leaves only scalar or uninitialized values in reused storage.
    matches!(
        analyzed.location,
        FunctionLocation::Function(_) | FunctionLocation::Method { .. }
    ) && register.index() < analyzed.chunk.local_register_count
        && unprotected_register(analyzed.chunk, analyzed.incoming_register_count, register)
        && analyzed.chunk.code[..before].iter().all(|instruction| {
            !branches_or_terminates(*instruction)
                && effect_on(analyzed.chunk, *instruction, register).is_none()
        })
}

fn temporary_register(chunk: &Chunk, incoming: u16, register: Register) -> bool {
    register.index() >= chunk.local_register_count
        && unprotected_register(chunk, incoming, register)
}

fn unprotected_register(chunk: &Chunk, incoming: u16, register: Register) -> bool {
    register.index() >= incoming
        && register.index() < chunk.register_count
        && !chunk.trace_argument_registers.contains(&register)
}

#[cfg(test)]
mod tests;
