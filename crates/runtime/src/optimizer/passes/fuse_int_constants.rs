//! Fusion of integer literal loads into adjacent proven consumers.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
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
    if !configuration.fuse_int_constants || chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for (index, should_remove) in remove.iter_mut().enumerate().take(chunk.code.len() - 1) {
        let Instruction::LoadInt {
            destination: temporary,
            immediate,
        } = chunk.code[index]
        else {
            continue;
        };

        if targets.contains(&(index + 1)) {
            continue;
        }

        let replacement = match chunk.code[index + 1] {
            Instruction::IntJumpUnless {
                comparison,
                left,
                right,
                offset,
            } if right == temporary && left != temporary => {
                Some(Instruction::IntJumpUnlessImmediate {
                    comparison,
                    source: left,
                    immediate,
                    offset,
                })
            }
            Instruction::IntJumpUnless {
                comparison,
                left,
                right,
                offset,
            } if left == temporary && right != temporary => {
                Some(Instruction::IntJumpUnlessImmediate {
                    comparison: comparison.reversed(),
                    source: right,
                    immediate,
                    offset,
                })
            }
            Instruction::ReturnUnchecked { source }
            | Instruction::ReturnScalarUnchecked { source }
                if source == temporary =>
            {
                Some(Instruction::ReturnIntUnchecked { immediate })
            }
            Instruction::IntAdd {
                destination,
                left,
                right,
            } if right == temporary && left != temporary => Some(Instruction::AddImmediate {
                destination,
                source: left,
                immediate,
            }),
            Instruction::IntAdd {
                destination,
                left,
                right,
            } if left == temporary && right != temporary => Some(Instruction::AddImmediate {
                destination,
                source: right,
                immediate,
            }),
            Instruction::IntSubtract {
                destination,
                left,
                right,
            } if right == temporary && left != temporary => Some(Instruction::SubtractImmediate {
                destination,
                source: left,
                immediate,
            }),
            Instruction::IntMultiply {
                destination,
                left,
                right,
            } if right == temporary && left != temporary => {
                Some(Instruction::IntMultiplyImmediate {
                    destination,
                    source: left,
                    immediate,
                })
            }
            Instruction::IntMultiply {
                destination,
                left,
                right,
            } if left == temporary && right != temporary => {
                Some(Instruction::IntMultiplyImmediate {
                    destination,
                    source: right,
                    immediate,
                })
            }
            Instruction::IntModulo {
                destination,
                left,
                right,
            } if right == temporary && left != temporary => Some(Instruction::IntModuloImmediate {
                destination,
                source: left,
                immediate,
            }),
            _ => None,
        };

        let Some(replacement) = replacement else {
            continue;
        };
        let consumes_temporary = match replacement {
            Instruction::AddImmediate { destination, .. }
            | Instruction::SubtractImmediate { destination, .. }
            | Instruction::IntMultiplyImmediate { destination, .. }
            | Instruction::IntModuloImmediate { destination, .. } => destination == temporary,
            Instruction::ReturnIntUnchecked { .. } => true,
            _ => false,
        };
        if !consumes_temporary && !register_is_dead_after(chunk, temporary, index + 2) {
            continue;
        }

        chunk.code[index + 1] = replacement;
        *should_remove = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
