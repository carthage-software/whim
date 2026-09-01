//! Fusion of adjacent square operations into one dispatch.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_squares || chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Some((first_destination, first_source, first_is_float)) = square(chunk.code[index])
        else {
            continue;
        };

        let Some((second_destination, second_source, second_is_float)) =
            square(chunk.code[index + 1])
        else {
            continue;
        };

        if first_is_float != second_is_float
            || first_destination == second_source
            || first_destination.index().checked_add(1) != Some(second_destination.index())
            || targets.contains(&(index + 1))
        {
            continue;
        }

        chunk.code[index] = if first_is_float {
            Instruction::FloatSquares {
                first_destination,
                first_source,
                second_source,
            }
        } else {
            Instruction::Squares {
                first_destination,
                first_source,
                second_source,
            }
        };

        remove[index + 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn square(instruction: Instruction) -> Option<(Register, Register, bool)> {
    match instruction {
        Instruction::Multiply {
            destination,
            left,
            right,
        } if left == right => Some((destination, left, false)),
        Instruction::FloatMultiply {
            destination,
            left,
            right,
        } if left == right => Some((destination, left, true)),
        _ => None,
    }
}
