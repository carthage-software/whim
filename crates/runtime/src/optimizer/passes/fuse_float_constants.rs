//! Fusion of float literal loads into their sole adjacent consumer.

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
    if !configuration.fuse_float_constants || chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for (index, removed) in remove.iter_mut().enumerate().take(chunk.code.len() - 1) {
        let Instruction::LoadConstant {
            destination: temporary,
            constant,
        } = chunk.code[index]
        else {
            continue;
        };

        let Literal::Float(value) = chunk.constants[usize::from(constant.index())] else {
            continue;
        };
        if targets.contains(&index) || !register_is_dead_after(chunk, temporary, index + 2) {
            continue;
        }

        let replacement = match chunk.code[index + 1] {
            Instruction::FloatMultiply {
                destination,
                left,
                right,
            } if value == 2.0 && left == temporary && right != temporary => {
                Some(Instruction::FloatAdd {
                    destination,
                    left: right,
                    right,
                })
            }
            Instruction::FloatMultiply {
                destination,
                left,
                right,
            } if value == 2.0 && right == temporary && left != temporary => {
                Some(Instruction::FloatAdd {
                    destination,
                    left,
                    right: left,
                })
            }
            Instruction::FloatMultiply {
                destination,
                left,
                right,
            } if left == temporary && right != temporary => {
                Some(Instruction::FloatMultiplyConstant {
                    destination,
                    source: right,
                    constant,
                })
            }
            Instruction::FloatMultiply {
                destination,
                left,
                right,
            } if right == temporary && left != temporary => {
                Some(Instruction::FloatMultiplyConstant {
                    destination,
                    source: left,
                    constant,
                })
            }
            Instruction::JumpUnless {
                comparison,
                left,
                right,
                offset,
            } if right == temporary && left != temporary => Some(Instruction::JumpUnlessConstant {
                comparison,
                source: left,
                constant,
                offset,
            }),
            Instruction::JumpUnless {
                comparison,
                left,
                right,
                offset,
            } if left == temporary && right != temporary => Some(Instruction::JumpUnlessConstant {
                comparison: comparison.reversed(),
                source: right,
                constant,
                offset,
            }),
            _ => None,
        };

        let Some(replacement) = replacement else {
            continue;
        };

        chunk.code[index + 1] = replacement;
        *removed = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
