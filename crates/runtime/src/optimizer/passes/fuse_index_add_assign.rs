//! Fusion of indexed compound addition into one container lookup.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::liveness::register_is_untouched_between;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, configuration, statistics);
    });
}

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_index_add_assign || chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for start in 0..=chunk.code.len() - 3 {
        let Some((previous, container, index)) = indexed_read(chunk.code[start]) else {
            continue;
        };

        let Some((result, left, increment, integer)) = addition(chunk.code[start + 1]) else {
            continue;
        };

        let Some((written_container, written_index, value, specialized_mode)) =
            indexed_write(chunk.code[start + 2])
        else {
            continue;
        };

        if left != previous
            || value != result
            || written_container != container
            || written_index != index
            || previous == result
            || (!integer && specialized_mode.is_some())
            || targets.contains(&(start + 1))
            || targets.contains(&(start + 2))
            || !register_is_dead_after(chunk, previous, start + 3)
            || !register_is_dead_after(chunk, result, start + 3)
        {
            continue;
        }

        let mut fused_index = index;
        let mut fused_increment = increment;
        let mut candidate = start;
        for _ in 0..2 {
            let Some(move_index) = candidate.checked_sub(1) else {
                break;
            };

            let Instruction::Move {
                destination,
                source,
            } = chunk.code[move_index]
            else {
                break;
            };

            let replacement = if destination == fused_increment {
                &mut fused_increment
            } else if destination == fused_index {
                &mut fused_index
            } else {
                break;
            };

            if targets.contains(&move_index)
                || !register_is_dead_after(chunk, destination, start + 3)
                || !register_is_untouched_between(chunk, source, move_index + 1, start)
            {
                break;
            }

            *replacement = source;
            remove[move_index] = true;
            candidate = move_index;
        }

        let mode = if integer {
            specialized_mode.unwrap_or(IndexAddMode::Generic)
        } else {
            IndexAddMode::Generic
        };
        chunk.code[start] = Instruction::IndexAddAssign {
            container,
            index: fused_index,
            value: fused_increment,
            mode,
        };

        remove[start + 1] = true;
        remove[start + 2] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn indexed_read(instruction: Instruction) -> Option<(Register, Register, Register)> {
    match instruction {
        Instruction::IndexGet {
            destination,
            container,
            index,
        }
        | Instruction::VecIndexGet {
            destination,
            container,
            index,
            ..
        }
        | Instruction::DictIndexGetIntKey {
            destination,
            container,
            index,
            ..
        }
        | Instruction::DictIndexGetStringKey {
            destination,
            container,
            index,
            ..
        } => Some((destination, container, index)),
        _ => None,
    }
}

fn addition(instruction: Instruction) -> Option<(Register, Register, Register, bool)> {
    match instruction {
        Instruction::Add {
            destination,
            left,
            right,
        } => Some((destination, left, right, false)),
        Instruction::IntAdd {
            destination,
            left,
            right,
        } => Some((destination, left, right, true)),
        _ => None,
    }
}

fn indexed_write(
    instruction: Instruction,
) -> Option<(Register, Register, Register, Option<IndexAddMode>)> {
    match instruction {
        Instruction::IndexSet {
            container,
            index,
            value,
        }
        | Instruction::VecIndexSet {
            container,
            index,
            value,
        } => Some((container, index, value, None)),
        Instruction::DictIndexSet {
            container,
            index,
            value,
        }
        | Instruction::DictIndexSetIntKey {
            container,
            index,
            value,
        } => Some((
            container,
            index,
            value,
            Some(IndexAddMode::DictAnyKeyIntValue),
        )),
        Instruction::DictIndexSetStringKey {
            container,
            index,
            value,
        } => Some((
            container,
            index,
            value,
            Some(IndexAddMode::DictStringKeyIntValue),
        )),
        _ => None,
    }
}
