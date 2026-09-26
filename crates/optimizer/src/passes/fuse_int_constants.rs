//! Fusion of integer literal loads into adjacent proven consumers.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ImmediateUint;
use whim_bytecode::instruction::operands::PropertyStepMode;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::rewrite::control_flow_targets;
use whim_bytecode::unit::CompiledUnit;

use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;
use crate::liveness::register_is_dead_after;
use crate::passes::compact_removed_instructions;
use crate::passes::for_each_mutable_chunk;

pub(crate) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, configuration, statistics);
    });
}

pub(crate) fn optimize_chunk(
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
        if targets.contains(&(index + 1)) {
            continue;
        }

        let (temporary, replacement) = match chunk.code[index] {
            Instruction::LoadUint {
                destination,
                immediate,
            } => (
                destination,
                unsigned_consumer(chunk.code[index + 1], destination, immediate),
            ),
            Instruction::LoadInt {
                destination: temporary,
                immediate,
            } => (
                temporary,
                match chunk.code[index + 1] {
                    Instruction::PropertyAdd {
                        object,
                        source,
                        cache,
                    } if source == temporary && object != temporary => {
                        Some(Instruction::PropertyStep {
                            object,
                            cache,
                            immediate,
                            mode: PropertyStepMode::Add,
                        })
                    }
                    Instruction::PropertyAddUnchecked {
                        object,
                        source,
                        slot,
                    } if source == temporary && object != temporary => {
                        Some(Instruction::PropertyStepUnchecked {
                            object,
                            slot,
                            immediate,
                            mode: PropertyStepMode::Add,
                        })
                    }
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
                    } if right == temporary && left != temporary => {
                        Some(Instruction::AddImmediate {
                            destination,
                            source: left,
                            immediate,
                        })
                    }
                    Instruction::IntAdd {
                        destination,
                        left,
                        right,
                    } if left == temporary && right != temporary => {
                        Some(Instruction::AddImmediate {
                            destination,
                            source: right,
                            immediate,
                        })
                    }
                    Instruction::IntSubtract {
                        destination,
                        left,
                        right,
                    } if right == temporary && left != temporary => {
                        Some(Instruction::SubtractImmediate {
                            destination,
                            source: left,
                            immediate,
                        })
                    }
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
                    } if right == temporary && left != temporary => {
                        Some(Instruction::IntModuloImmediate {
                            destination,
                            source: left,
                            immediate,
                        })
                    }
                    _ => None,
                },
            ),
            _ => continue,
        };

        let Some(replacement) = replacement else {
            continue;
        };
        let consumes_temporary = match replacement {
            Instruction::AddImmediate { destination, .. }
            | Instruction::SubtractImmediate { destination, .. }
            | Instruction::IntMultiplyImmediate { destination, .. }
            | Instruction::IntModuloImmediate { destination, .. }
            | Instruction::UintAddImmediate { destination, .. }
            | Instruction::UintSubtractImmediate { destination, .. }
            | Instruction::UintMultiplyImmediate { destination, .. }
            | Instruction::UintModuloImmediate { destination, .. } => destination == temporary,
            Instruction::ReturnIntUnchecked { .. } | Instruction::ReturnUintUnchecked { .. } => {
                true
            }
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

fn unsigned_consumer(
    instruction: Instruction,
    temporary: Register,
    immediate: ImmediateUint,
) -> Option<Instruction> {
    Some(match instruction {
        Instruction::UintJumpUnless {
            comparison,
            left,
            right,
            offset,
        } if right == temporary && left != temporary => Instruction::UintJumpUnlessImmediate {
            comparison,
            source: left,
            immediate,
            offset,
        },
        Instruction::UintJumpUnless {
            comparison,
            left,
            right,
            offset,
        } if left == temporary && right != temporary => Instruction::UintJumpUnlessImmediate {
            comparison: comparison.reversed(),
            source: right,
            immediate,
            offset,
        },
        Instruction::ReturnUnchecked { source } | Instruction::ReturnScalarUnchecked { source }
            if source == temporary =>
        {
            Instruction::ReturnUintUnchecked { immediate }
        }
        Instruction::UintAdd {
            destination,
            left,
            right,
        } if right == temporary && left != temporary => Instruction::UintAddImmediate {
            destination,
            source: left,
            immediate,
        },
        Instruction::UintAdd {
            destination,
            left,
            right,
        } if left == temporary && right != temporary => Instruction::UintAddImmediate {
            destination,
            source: right,
            immediate,
        },
        Instruction::UintSubtract {
            destination,
            left,
            right,
        } if right == temporary && left != temporary => Instruction::UintSubtractImmediate {
            destination,
            source: left,
            immediate,
        },
        Instruction::UintMultiply {
            destination,
            left,
            right,
        } if right == temporary && left != temporary => Instruction::UintMultiplyImmediate {
            destination,
            source: left,
            immediate,
        },
        Instruction::UintMultiply {
            destination,
            left,
            right,
        } if left == temporary && right != temporary => Instruction::UintMultiplyImmediate {
            destination,
            source: right,
            immediate,
        },
        Instruction::UintModulo {
            destination,
            left,
            right,
        } if right == temporary && left != temporary => Instruction::UintModuloImmediate {
            destination,
            source: left,
            immediate,
        },
        _ => return None,
    })
}
