use std::borrow::Cow;
use std::mem;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::NearJumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::rewrite::control_flow_targets;
use crate::bytecode::rewrite::rebase_targets;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if configuration.fuse_coalescing {
        for_each_mutable_chunk(unit, configuration, |chunk| {
            optimize_chunk(chunk, statistics)
        });
    }
}

pub(in crate::optimizer) fn normalize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
) {
    for_each_mutable_chunk(unit, configuration, normalize_chunk);
}

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    statistics: &mut OptimizationStatistics,
) {
    if !chunk
        .code
        .iter()
        .any(|instruction| matches!(instruction, Instruction::JumpIfNotNull { .. }))
    {
        return;
    }
    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len().saturating_sub(1) {
        if remove[index] || targets.contains(&(index + 1)) {
            continue;
        }
        let Instruction::JumpIfNotNull { subject, offset } = chunk.code[index + 1] else {
            continue;
        };
        let target = relative_target(index + 1, offset.offset());
        let Ok(relative) = i32::try_from(target as i64 - index as i64) else {
            continue;
        };
        if let Some(instruction) = fused(chunk.code[index], subject, relative) {
            chunk.code[index] = instruction;
            remove[index + 1] = true;
        }
    }
    compact_removed_instructions(chunk, &remove, statistics);
}

fn fused(instruction: Instruction, subject: Register, relative: i32) -> Option<Instruction> {
    Some(match instruction {
        Instruction::IndexGetOrNull {
            destination,
            container,
            index,
        } if destination == subject => Instruction::IndexCoalesce {
            destination,
            container,
            index,
            offset: NearJumpOffset::new(relative.try_into().ok()?),
        },
        Instruction::VecIndexGetOrNull {
            destination,
            container,
            index,
        } if destination == subject => Instruction::VecIndexCoalesce {
            destination,
            container,
            index,
            offset: NearJumpOffset::new(relative.try_into().ok()?),
        },
        Instruction::DictIndexGetIntKeyOrNull {
            destination,
            container,
            index,
        } if destination == subject => Instruction::DictIndexCoalesceIntKey {
            destination,
            container,
            index,
            offset: NearJumpOffset::new(relative.try_into().ok()?),
        },
        Instruction::DictIndexGetStringKeyOrNull {
            destination,
            container,
            index,
        } if destination == subject => Instruction::DictIndexCoalesceStringKey {
            destination,
            container,
            index,
            offset: NearJumpOffset::new(relative.try_into().ok()?),
        },
        Instruction::StringIndexGetOrNull {
            destination,
            container,
            index,
        } if destination == subject => Instruction::StringIndexCoalesce {
            destination,
            container,
            index,
            offset: NearJumpOffset::new(relative.try_into().ok()?),
        },
        Instruction::PropertyGetOrNull {
            destination,
            object,
            cache,
        } if destination == subject => Instruction::PropertyCoalesce {
            destination,
            object,
            cache,
            offset: NearJumpOffset::new(relative.try_into().ok()?),
        },
        Instruction::PropertyGetOrNullUnchecked {
            destination,
            object,
            slot,
        } if destination == subject => Instruction::PropertyCoalesceUnchecked {
            destination,
            object,
            slot,
            offset: NearJumpOffset::new(relative.try_into().ok()?),
        },
        Instruction::StaticPropertyGetOrNull { destination, cache } if destination == subject => {
            Instruction::StaticPropertyCoalesce {
                destination,
                cache,
                offset: ShortJumpOffset::new(relative.try_into().ok()?),
            }
        }
        Instruction::Move {
            destination,
            source,
        } if destination == subject || source == subject => Instruction::Coalesce {
            destination,
            source,
            offset: ShortJumpOffset::new(relative.try_into().ok()?),
        },
        _ => return None,
    })
}

pub(in crate::optimizer) fn normalized_chunk(chunk: &Chunk) -> Cow<'_, Chunk> {
    if chunk
        .code
        .iter()
        .any(|instruction| split(*instruction).is_some())
    {
        let mut normalized = chunk.clone();
        normalize_chunk(&mut normalized);
        Cow::Owned(normalized)
    } else {
        Cow::Borrowed(chunk)
    }
}

pub(in crate::optimizer) fn normalize_chunk(chunk: &mut Chunk) {
    if !chunk
        .code
        .iter()
        .any(|instruction| split(*instruction).is_some())
    {
        return;
    }
    let old_code = mem::take(&mut chunk.code);
    let old_spans = mem::take(&mut chunk.spans);
    let mut old_to_new = Vec::with_capacity(old_code.len() + 1);
    let mut position = 0;
    for instruction in &old_code {
        old_to_new.push(position);
        position += if split(*instruction).is_some() { 2 } else { 1 };
    }
    old_to_new.push(position);
    for (index, mut instruction) in old_code.into_iter().enumerate() {
        let new_index = chunk.code.len();
        if let Some((read, destination, offset)) = split(instruction) {
            let target = old_to_new[relative_target(index, offset)];
            chunk.code.push(read);
            chunk.spans.push(old_spans[index]);
            chunk.code.push(Instruction::JumpIfNotNull {
                subject: destination,
                offset: JumpOffset::new(
                    i32::try_from(target as i64 - (new_index + 1) as i64)
                        .expect("a chunk jump fits i32"),
                ),
            });
        } else {
            rebase_targets(chunk, &mut instruction, index, new_index, &old_to_new);
            chunk.code.push(instruction);
        }
        chunk.spans.push(old_spans[index]);
    }
    for entry in &mut chunk.catch_table {
        entry.start =
            u32::try_from(old_to_new[entry.start as usize]).expect("a chunk position fits u32");
        entry.end =
            u32::try_from(old_to_new[entry.end as usize]).expect("a chunk position fits u32");
        entry.handler =
            u32::try_from(old_to_new[entry.handler as usize]).expect("a chunk position fits u32");
    }
}

fn split(instruction: Instruction) -> Option<(Instruction, Register, i32)> {
    Some(match instruction {
        Instruction::IndexCoalesce {
            destination,
            container,
            index,
            offset,
        } => (
            Instruction::IndexGetOrNull {
                destination,
                container,
                index,
            },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::VecIndexCoalesce {
            destination,
            container,
            index,
            offset,
        } => (
            Instruction::VecIndexGetOrNull {
                destination,
                container,
                index,
            },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::DictIndexCoalesceIntKey {
            destination,
            container,
            index,
            offset,
        } => (
            Instruction::DictIndexGetIntKeyOrNull {
                destination,
                container,
                index,
            },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::DictIndexCoalesceStringKey {
            destination,
            container,
            index,
            offset,
        } => (
            Instruction::DictIndexGetStringKeyOrNull {
                destination,
                container,
                index,
            },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::StringIndexCoalesce {
            destination,
            container,
            index,
            offset,
        } => (
            Instruction::StringIndexGetOrNull {
                destination,
                container,
                index,
            },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::PropertyCoalesce {
            destination,
            object,
            cache,
            offset,
        } => (
            Instruction::PropertyGetOrNull {
                destination,
                object,
                cache,
            },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::PropertyCoalesceUnchecked {
            destination,
            object,
            slot,
            offset,
        } => (
            Instruction::PropertyGetOrNullUnchecked {
                destination,
                object,
                slot,
            },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::StaticPropertyCoalesce {
            destination,
            cache,
            offset,
        } => (
            Instruction::StaticPropertyGetOrNull { destination, cache },
            destination,
            i32::from(offset.offset()),
        ),
        Instruction::Coalesce {
            destination,
            source,
            offset,
        } => (
            Instruction::Move {
                destination,
                source,
            },
            destination,
            i32::from(offset.offset()),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use whim_span::Span;

    use super::normalize_chunk;
    use super::optimize_chunk;
    use crate::bytecode::chunk::Chunk;
    use crate::bytecode::instruction::Instruction;
    use crate::bytecode::instruction::operands::JumpOffset;
    use crate::bytecode::instruction::operands::Register;
    use crate::bytecode::verify::verify;
    use crate::optimizer::OptimizationStatistics;

    #[test]
    fn fusion_preserves_targets_and_round_trips_at_offset_boundaries() {
        for distance in [2, 126, 127, 128, 32768] {
            let mut chunk = Chunk::new();
            chunk.register_count = 3;
            let destination = Register::new(0);
            chunk.emit(
                Instruction::IndexGetOrNull {
                    destination,
                    container: Register::new(1),
                    index: Register::new(2),
                },
                Span::zero(),
            );
            chunk.emit(
                Instruction::JumpIfNotNull {
                    subject: destination,
                    offset: JumpOffset::new(distance - 1),
                },
                Span::zero(),
            );
            for _ in 2..distance {
                chunk.emit(Instruction::LoadNull { destination }, Span::zero());
            }
            chunk.emit(
                Instruction::Return {
                    source: destination,
                },
                Span::zero(),
            );
            let original = chunk.code.clone();
            optimize_chunk(&mut chunk, &mut OptimizationStatistics::default());
            assert_eq!(
                matches!(chunk.code[0], Instruction::IndexCoalesce { .. }),
                distance <= 127
            );
            verify(&chunk).unwrap();
            normalize_chunk(&mut chunk);
            assert_eq!(chunk.code, original);
            assert_eq!(chunk.code.len(), chunk.spans.len());
            verify(&chunk).unwrap();
        }
    }
}
