//! Fusion of exact sequential float multiply/add and subtract/add chains.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_muladd || chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];

    for index in 0..chunk.code.len() - 1 {
        let Instruction::FloatSubtract {
            destination: difference,
            left,
            right,
        } = chunk.code[index]
        else {
            continue;
        };

        let Instruction::FloatAdd {
            destination,
            left: consumed,
            right: addend,
        } = chunk.code[index + 1]
        else {
            continue;
        };

        if consumed != difference
            || difference == addend
            || right.index().checked_sub(1) != Some(left.index())
            || targets.contains(&(index + 1))
            || !register_is_dead_after(chunk, difference, index + 2)
        {
            continue;
        }

        chunk.code[index] = Instruction::FloatDifferenceAdd {
            destination,
            first_operand: left,
            addend,
        };

        remove[index + 1] = true;
    }

    for index in 0..chunk.code.len().saturating_sub(2) {
        if remove[index] || remove[index + 1] || remove[index + 2] {
            continue;
        }
        let (scaled, left, constant) = match chunk.code[index] {
            Instruction::FloatMultiplyConstant {
                destination,
                source,
                constant,
            } => (destination, source, constant),
            Instruction::FloatAdd {
                destination,
                left,
                right,
            } if left == right => {
                let Ok(constant) = chunk.add_constant(Literal::Float(2.0)) else {
                    continue;
                };

                (destination, left, constant)
            }
            _ => continue,
        };

        let Instruction::FloatMultiply {
            destination: product,
            left: consumed,
            right,
        } = chunk.code[index + 1]
        else {
            continue;
        };

        let Instruction::FloatAdd {
            destination,
            left: consumed_product,
            right: addend,
        } = chunk.code[index + 2]
        else {
            continue;
        };

        if consumed != scaled
            || consumed_product != product
            || scaled.index() >= addend.index() && scaled.index() <= right.index()
            || product == addend
            || addend.index().checked_add(1) != Some(left.index())
            || left.index().checked_add(1) != Some(right.index())
            || targets.contains(&(index + 1))
            || targets.contains(&(index + 2))
            || !register_is_dead_after(chunk, scaled, index + 3)
            || !register_is_dead_after(chunk, product, index + 3)
        {
            continue;
        }

        chunk.code[index] = Instruction::FloatScaleProductAdd {
            destination,
            first_operand: addend,
            constant,
        };

        remove[index + 1] = true;
        remove[index + 2] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
