//! Ownership transfer for moves whose source value is dead on every path.

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::PropertyValueMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::TypeFlow;

pub(in crate::optimizer) fn normalize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
) {
    for_each_mutable_chunk(unit, configuration, normalize_chunk);
}

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
) {
    if !configuration.ownership_moves {
        return;
    }

    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::OWNERSHIP) {
            continue;
        }

        for index in 0..analyzed.chunk.code.len() {
            if !plan.is_available(analyzed, index) {
                continue;
            }

            if !transfer_at(analyzed.chunk, Some(&analyzed.flow), index) {
                continue;
            }

            let replacement = match analyzed.chunk.code[index] {
                Instruction::Move {
                    destination,
                    source,
                } => Instruction::MoveOwned {
                    destination,
                    source,
                },
                Instruction::PropertySetUnchecked {
                    object,
                    value,
                    slot,
                    value_mode: PropertyValueMode::Clone,
                } => Instruction::PropertySetUnchecked {
                    object,
                    value,
                    slot,
                    value_mode: PropertyValueMode::MoveAndClear,
                },
                Instruction::PropertySetUnchecked {
                    object,
                    value,
                    slot,
                    value_mode: PropertyValueMode::FreshClone,
                } => Instruction::PropertySetUnchecked {
                    object,
                    value,
                    slot,
                    value_mode: PropertyValueMode::FreshMoveAndClear,
                },
                _ => continue,
            };
            analyzed.write(plan, index, replacement);
        }
    }
}

pub(in crate::optimizer) fn normalize_chunk(chunk: &mut Chunk) {
    for instruction in &mut chunk.code {
        let Instruction::MoveOwned {
            destination,
            source,
        } = *instruction
        else {
            continue;
        };

        *instruction = Instruction::Move {
            destination,
            source,
        };
    }
}

fn transfer_at(chunk: &Chunk, flow: Option<&TypeFlow<'_>>, index: usize) -> bool {
    match chunk.code[index] {
        Instruction::Move {
            destination,
            source,
        } => {
            !feeds_receiver_only_method_call(chunk, index, destination)
                && can_transfer(chunk, flow, index, source, destination, index + 1)
        }
        Instruction::PropertySetUnchecked {
            object,
            value,
            value_mode: PropertyValueMode::Clone | PropertyValueMode::FreshClone,
            ..
        } => can_transfer(chunk, flow, index, value, object, index + 1),
        _ => false,
    }
}

fn feeds_receiver_only_method_call(chunk: &Chunk, index: usize, destination: Register) -> bool {
    matches!(
        chunk.code.get(index + 1),
        Some(
            Instruction::CallMethod {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethodUnchecked {
                argument_count,
                first_argument,
                ..
            }
        ) if argument_count.value() == 1 && *first_argument == destination
    )
}

fn can_transfer(
    chunk: &Chunk,
    flow: Option<&TypeFlow<'_>>,
    index: usize,
    source: Register,
    destination: Register,
    start: usize,
) -> bool {
    source.index() != 0
        && source != destination
        && !is_parameter_register(chunk, source)
        && (source.index() >= chunk.local_register_count
            || chunk.register_count <= REFERENCE_REGISTER_LIMIT
                && chunk.reference_register_mask & (1u64 << source.index()) == 0
            || flow.is_some_and(|flow| !flow.register_may_release_observably(index, source)))
        && register_is_dead_after(chunk, source, start)
}

fn is_parameter_register(chunk: &Chunk, register: Register) -> bool {
    let position = register
        .index()
        .wrapping_sub(chunk.parameter_register_start);
    position < chunk.parameter_register_count
}
