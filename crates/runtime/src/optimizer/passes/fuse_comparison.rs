//! Fusion of comparison results consumed only by a conditional jump.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::liveness::register_is_dead_after_removals;
use crate::optimizer::passes::compact_removed_instructions;

pub(super) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_comparison || chunk.code.len() < 2 {
        return;
    }

    fuse_comparison_result(chunk, statistics);
    fuse_null_comparison(chunk, statistics);
    fuse_int_range_result(chunk, statistics);
}

fn fuse_int_range_result(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Instruction::Is {
            destination,
            source,
            descriptor,
        } = chunk.code[index]
        else {
            continue;
        };
        if !matches!(
            chunk.type_descriptors[usize::from(descriptor.index())],
            TypeDescriptor::IntRange { .. }
        ) {
            continue;
        }
        let (condition, offset, jumps_if_match) = match chunk.code[index + 1] {
            Instruction::JumpIfFalse { condition, offset } => (condition, offset, false),
            Instruction::JumpIfTrue { condition, offset } => (condition, offset, true),
            _ => continue,
        };
        let target = index as i64 + 1 + i64::from(offset.offset());
        let Ok(target_index) = usize::try_from(target) else {
            continue;
        };
        if condition != destination
            || targets.contains(&(index + 1))
            || !register_is_dead_after_removals(chunk, destination, index + 2, &remove)
            || !register_is_dead_after_removals(chunk, destination, target_index, &remove)
        {
            continue;
        }
        let Ok(relative) = i16::try_from(target - index as i64) else {
            continue;
        };
        let offset = ShortJumpOffset::new(relative);
        chunk.code[index] = if jumps_if_match {
            Instruction::IntRangeJumpIf {
                subject: source,
                descriptor,
                offset,
            }
        } else {
            Instruction::IntRangeJumpUnless {
                subject: source,
                descriptor,
                offset,
            }
        };
        remove[index + 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn fuse_comparison_result(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Some((comparison, destination, left, right)) = comparison(chunk.code[index]) else {
            continue;
        };

        let (condition, offset, comparison) = match chunk.code[index + 1] {
            Instruction::JumpIfFalse { condition, offset } => (condition, offset, comparison),
            Instruction::JumpIfTrue { condition, offset } => {
                let Some(comparison) = equality_complement(comparison) else {
                    continue;
                };
                (condition, offset, comparison)
            }
            _ => continue,
        };

        let target = index as i64 + 1 + i64::from(offset.offset());
        let Ok(target_index) = usize::try_from(target) else {
            continue;
        };

        if condition != destination
            || targets.contains(&(index + 1))
            || !register_is_dead_after_removals(chunk, destination, index + 2, &remove)
            || !register_is_dead_after_removals(chunk, destination, target_index, &remove)
        {
            continue;
        }

        let relative = target - index as i64;
        let Ok(relative) = i16::try_from(relative) else {
            continue;
        };

        chunk.code[index] = Instruction::JumpUnless {
            comparison,
            left,
            right,
            offset: ShortJumpOffset::new(relative),
        };

        remove[index + 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn equality_complement(comparison: Comparison) -> Option<Comparison> {
    match comparison {
        Comparison::Equal => Some(Comparison::NotEqual),
        Comparison::NotEqual => Some(Comparison::Equal),
        _ => None,
    }
}

fn fuse_null_comparison(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    #[expect(
        clippy::needless_range_loop,
        reason = "the pass mutates code and its parallel removal mask by index"
    )]
    for index in 0..chunk.code.len() - 1 {
        let Instruction::LoadNull {
            destination: temporary,
        } = chunk.code[index]
        else {
            continue;
        };

        let Instruction::JumpUnless {
            comparison,
            left,
            right,
            offset,
        } = chunk.code[index + 1]
        else {
            continue;
        };

        let subject = if left == temporary && right != temporary {
            right
        } else if right == temporary && left != temporary {
            left
        } else {
            continue;
        };

        if !matches!(comparison, Comparison::Equal | Comparison::NotEqual)
            || targets.contains(&index)
            || targets.contains(&(index + 1))
        {
            continue;
        }

        let target = relative_target(index + 1, i32::from(offset.offset()));
        if !register_is_dead_after(chunk, temporary, index + 2)
            || !register_is_dead_after(chunk, temporary, target)
        {
            continue;
        }

        let offset = JumpOffset::new(i32::from(offset.offset()));
        chunk.code[index + 1] = if comparison == Comparison::Equal {
            Instruction::JumpIfNotNull { subject, offset }
        } else {
            Instruction::JumpIfNull { subject, offset }
        };

        remove[index] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn comparison(instruction: Instruction) -> Option<(Comparison, Register, Register, Register)> {
    match instruction {
        Instruction::Equal {
            destination,
            left,
            right,
        } => Some((Comparison::Equal, destination, left, right)),
        Instruction::NotEqual {
            destination,
            left,
            right,
        } => Some((Comparison::NotEqual, destination, left, right)),
        Instruction::LessThan {
            destination,
            left,
            right,
        } => Some((Comparison::LessThan, destination, left, right)),
        Instruction::LessThanOrEqual {
            destination,
            left,
            right,
        } => Some((Comparison::LessThanOrEqual, destination, left, right)),
        Instruction::GreaterThan {
            destination,
            left,
            right,
        } => Some((Comparison::GreaterThan, destination, left, right)),
        Instruction::GreaterThanOrEqual {
            destination,
            left,
            right,
        } => Some((Comparison::GreaterThanOrEqual, destination, left, right)),
        _ => None,
    }
}
