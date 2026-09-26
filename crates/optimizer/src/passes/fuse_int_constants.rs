//! Fusion of integer literal loads into adjacent proven consumers.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ImmediateInt;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::ImmediateUint;
use whim_bytecode::instruction::operands::IntegerKind;
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

        let Instruction::LoadInteger {
            destination: temporary,
            immediate,
            kind,
        } = chunk.code[index]
        else {
            continue;
        };
        let Some(replacement) = consumer(chunk.code[index + 1], temporary, immediate, kind) else {
            continue;
        };
        let consumes_temporary = match replacement {
            Instruction::IntegerAddImmediate { destination, .. }
            | Instruction::IntegerSubtractImmediate { destination, .. }
            | Instruction::IntegerMultiplyImmediate { destination, .. }
            | Instruction::IntegerModuloImmediate { destination, .. } => destination == temporary,
            Instruction::ReturnIntegerUnchecked { .. } => true,
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

fn consumer(
    instruction: Instruction,
    temporary: Register,
    immediate: ImmediateInteger,
    kind: IntegerKind,
) -> Option<Instruction> {
    Some(match instruction {
        Instruction::PropertyAdd {
            object,
            source,
            cache,
        } if kind == IntegerKind::I64 && source == temporary && object != temporary => {
            Instruction::PropertyStep {
                object,
                cache,
                immediate: ImmediateInt::new(immediate.as_int()),
                mode: PropertyStepMode::Add,
            }
        }
        Instruction::PropertyAddUnchecked {
            object,
            source,
            slot,
        } if kind == IntegerKind::I64 && source == temporary && object != temporary => {
            Instruction::PropertyStepUnchecked {
                object,
                slot,
                immediate: ImmediateInt::new(immediate.as_int()),
                mode: PropertyStepMode::Add,
            }
        }
        Instruction::IntJumpUnless {
            comparison,
            left,
            right,
            offset,
        } if kind == IntegerKind::I64 => Instruction::IntJumpUnlessImmediate {
            comparison: if right == temporary {
                comparison
            } else {
                comparison.reversed()
            },
            source: other_operand(left, right, temporary, true)?,
            immediate: ImmediateInt::new(immediate.as_int()),
            offset,
        },
        Instruction::UintJumpUnless {
            comparison,
            left,
            right,
            offset,
        } if kind == IntegerKind::U64 => Instruction::UintJumpUnlessImmediate {
            comparison: if right == temporary {
                comparison
            } else {
                comparison.reversed()
            },
            source: other_operand(left, right, temporary, true)?,
            immediate: ImmediateUint::new(immediate.as_uint()),
            offset,
        },
        Instruction::ReturnUnchecked { source } | Instruction::ReturnScalarUnchecked { source }
            if source == temporary =>
        {
            Instruction::ReturnIntegerUnchecked { immediate, kind }
        }
        Instruction::IntegerAdd {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == kind => Instruction::IntegerAddImmediate {
            destination,
            source: other_operand(left, right, temporary, true)?,
            immediate,
            kind,
        },
        Instruction::IntegerSubtract {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == kind => Instruction::IntegerSubtractImmediate {
            destination,
            source: other_operand(left, right, temporary, false)?,
            immediate,
            kind,
        },
        Instruction::IntegerMultiply {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == kind => Instruction::IntegerMultiplyImmediate {
            destination,
            source: other_operand(left, right, temporary, true)?,
            immediate,
            kind,
        },
        Instruction::IntegerModulo {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == kind => Instruction::IntegerModuloImmediate {
            destination,
            source: other_operand(left, right, temporary, false)?,
            immediate,
            kind,
        },
        _ => return None,
    })
}

fn other_operand(
    left: Register,
    right: Register,
    temporary: Register,
    commutative: bool,
) -> Option<Register> {
    if right == temporary && left != temporary {
        Some(left)
    } else if commutative && left == temporary && right != temporary {
        Some(right)
    } else {
        None
    }
}
