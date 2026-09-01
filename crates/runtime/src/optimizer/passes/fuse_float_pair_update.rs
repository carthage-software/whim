//! Fusion of adjacent sequential float updates into one dispatch.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_float_pair_update || chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Instruction::FloatScaleProductAdd {
            destination: first_destination,
            first_operand,
            constant,
        } = chunk.code[index]
        else {
            continue;
        };

        let Instruction::FloatDifferenceAdd {
            destination: second_destination,
            first_operand: second_operand,
            addend: second_addend,
        } = chunk.code[index + 1]
        else {
            continue;
        };

        if targets.contains(&(index + 1)) {
            continue;
        }

        let Ok(descriptor) = chunk.add_float_pair_update_descriptor(FloatPairUpdateDescriptor {
            first_destination,
            first_operand,
            constant,
            second_destination,
            second_operand,
            second_addend,
        }) else {
            continue;
        };

        chunk.code[index] = Instruction::FloatPairUpdate { descriptor };
        remove[index + 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
