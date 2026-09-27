//! Fusion of integer literal loads into adjacent proven consumers.

use whim_bytecode::REFERENCE_REGISTER_LIMIT;
use whim_bytecode::chunk::Chunk;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ImmediateInt;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::ImmediateUint;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::PropertyStepMode;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::reference_registers;
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
    let mut reference_mask = None;
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
        if matches!(chunk.code[index + 1], Instruction::IntegerAddAssign { .. }) {
            let reference_mask = *reference_mask.get_or_insert_with(|| {
                chunk.reference_register_mask | reference_registers::mask(chunk)
            });
            if temporary.index() < chunk.local_register_count
                || chunk.trace_argument_registers.contains(&temporary)
                || temporary.index() >= REFERENCE_REGISTER_LIMIT
                || reference_mask & (1u64 << temporary.index()) != 0
                || chunk.catch_table.iter().any(|entry| {
                    index + 1 >= entry.start as usize && index + 1 < entry.end as usize
                })
            {
                continue;
            }
        }

        let Some(replacement) = consumer(chunk.code[index + 1], temporary, immediate, kind) else {
            continue;
        };
        let original = chunk.code[index + 1];
        chunk.code[index + 1] = replacement;
        if !register_is_dead_after(chunk, temporary, index + 1) {
            chunk.code[index + 1] = original;
            continue;
        }

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
        Instruction::Add {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == Some(kind) => Instruction::AddImmediate {
            destination,
            source: other_operand(left, right, temporary, true)?,
            immediate,
            kind: Some(kind),
        },
        Instruction::IntegerAddAssign {
            target,
            source,
            kind: operand_kind,
        } if operand_kind == kind && source == temporary && target != temporary => {
            Instruction::AddImmediate {
                destination: target,
                source: target,
                immediate,
                kind: Some(kind),
            }
        }
        Instruction::Subtract {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == Some(kind) => Instruction::SubtractImmediate {
            destination,
            source: other_operand(left, right, temporary, false)?,
            immediate,
            kind: Some(kind),
        },
        Instruction::Multiply {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == Some(kind) => Instruction::IntegerMultiplyImmediate {
            destination,
            source: other_operand(left, right, temporary, true)?,
            immediate,
            kind,
        },
        Instruction::Modulo {
            destination,
            left,
            right,
            kind: operand_kind,
        } if operand_kind == Some(kind) => Instruction::IntegerModuloImmediate {
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

#[cfg(test)]
mod tests;
