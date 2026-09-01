//! Fusion of a float square sum with its constant-consuming branch.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::JumpOffset;
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
    if !configuration.fuse_square_sum_branch || chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let (
            sum_destination,
            first_square_destination,
            second_square_destination,
            first_source,
            second_source,
            branch_index,
        ) = match chunk.code[index] {
            Instruction::FloatSquaresSum {
                first_destination,
                first_source,
                second_source,
            } => (
                first_destination,
                Register::new(first_destination.index() + 1),
                Register::new(first_destination.index() + 2),
                first_source,
                second_source,
                index + 1,
            ),
            Instruction::FloatSquares {
                first_destination,
                first_source,
                second_source,
            } if index + 2 < chunk.code.len() => {
                let Instruction::FloatAdd {
                    destination,
                    left,
                    right,
                } = chunk.code[index + 1]
                else {
                    continue;
                };

                if left != first_destination
                    || right.index().checked_sub(1) != Some(first_destination.index())
                {
                    continue;
                }

                (
                    destination,
                    first_destination,
                    right,
                    first_source,
                    second_source,
                    index + 2,
                )
            }
            _ => continue,
        };
        let Instruction::JumpUnlessConstant {
            comparison,
            source,
            constant,
            offset,
        } = chunk.code[branch_index]
        else {
            continue;
        };

        let target = branch_index as i64 + i64::from(offset.offset());
        if source != sum_destination || (index + 1..=branch_index).any(|at| targets.contains(&at)) {
            continue;
        }

        let Ok(relative) = i32::try_from(target - index as i64) else {
            continue;
        };

        let Ok(descriptor) =
            chunk.add_float_squares_sum_branch_descriptor(FloatSquaresSumBranchDescriptor {
                sum_destination,
                first_square_destination,
                second_square_destination,
                first_source,
                second_source,
                comparison,
                constant,
            })
        else {
            continue;
        };

        chunk.code[index] = Instruction::FloatSquaresSumBranch {
            descriptor,
            offset: JumpOffset::new(relative),
        };

        remove[index + 1..=branch_index].fill(true);
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
