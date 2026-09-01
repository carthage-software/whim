//! Code splicing, compaction, and jump-target rebasing.

use std::mem;

use whim_span::Span;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::rewrite::rebase_targets;
use crate::optimizer::cfg::successors;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;

/// Replaces the single instruction at `at` with a straight-line sequence,
/// shifting later instructions and rebasing every jump target and catch
/// range across the splice.
pub(in crate::optimizer) fn splice_replace(
    chunk: &mut Chunk,
    at: usize,
    replacement: &[(Instruction, Span)],
) {
    splice_replacements(chunk, &[(at, replacement)]);
}

pub(in crate::optimizer) fn splice_replace_many(
    chunk: &mut Chunk,
    replacements: &[(usize, Vec<(Instruction, Span)>)],
) {
    splice_replacements(chunk, replacements);
}

fn splice_replacements<R>(chunk: &mut Chunk, replacements: &[(usize, R)])
where
    R: AsRef<[(Instruction, Span)]>,
{
    let Some((_, first)) = replacements.first() else {
        return;
    };

    debug_assert!(replacements.windows(2).all(|pair| pair[0].0 < pair[1].0));
    debug_assert!(!first.as_ref().is_empty());
    debug_assert!(
        replacements
            .iter()
            .all(|(at, replacement)| *at < chunk.code.len() && !replacement.as_ref().is_empty())
    );

    let old_code = mem::take(&mut chunk.code);
    let old_spans = mem::take(&mut chunk.spans);
    let mut old_to_new = Vec::with_capacity(old_code.len() + 1);
    let mut replacement_position = 0;
    let mut next = 0;
    for old_index in 0..old_code.len() {
        old_to_new.push(next);
        if replacements
            .get(replacement_position)
            .is_some_and(|(at, _)| *at == old_index)
        {
            next += replacements[replacement_position].1.as_ref().len();
            replacement_position += 1;
        } else {
            next += 1;
        }
    }
    old_to_new.push(next);

    chunk.code.reserve_exact(next);
    chunk.spans.reserve_exact(next);
    replacement_position = 0;
    for (old_index, mut instruction) in old_code.into_iter().enumerate() {
        if let Some((at, replacement)) = replacements.get(replacement_position)
            && *at == old_index
        {
            for (inlined, span) in replacement.as_ref() {
                chunk.code.push(*inlined);
                chunk.spans.push(*span);
            }
            replacement_position += 1;
            continue;
        }

        let new_index = old_to_new[old_index];
        rebase_targets(chunk, &mut instruction, old_index, new_index, &old_to_new);
        chunk.code.push(instruction);
        chunk.spans.push(old_spans[old_index]);
    }

    for entry in &mut chunk.catch_table {
        entry.start = old_to_new[entry.start as usize] as u32;
        entry.end = old_to_new[entry.end as usize] as u32;
        entry.handler = old_to_new[entry.handler as usize] as u32;
    }
}

/// Inserts straight-line instructions before `index` without rewriting branch
/// offsets when no explicit control-flow edge crosses that boundary.
pub(in crate::optimizer) fn can_insert_straight_line_before(chunk: &Chunk, index: usize) -> bool {
    straight_line_insertion_boundaries(chunk)
        .get(index)
        .copied()
        .unwrap_or(false)
}

pub(in crate::optimizer) fn straight_line_insertion_boundaries(chunk: &Chunk) -> Vec<bool> {
    let mut starts = vec![0usize; chunk.code.len() + 2];
    let mut ends = vec![0usize; chunk.code.len() + 2];
    let mut targets = Vec::new();
    for source in 0..chunk.code.len() {
        targets.clear();
        successors(chunk, source, &mut targets);
        for &target in &targets {
            if target == source + 1 {
                continue;
            }

            let first = source.min(target) + 1;
            let last = source.max(target);
            starts[first] += 1;
            ends[last + 1] += 1;
        }
    }

    let mut crossing = 0usize;
    (0..=chunk.code.len())
        .map(|index| {
            crossing -= ends[index];
            crossing += starts[index];
            crossing == 0
        })
        .collect()
}

pub(in crate::optimizer) fn insert_straight_line_before_many(
    chunk: &mut Chunk,
    insertions: &[(usize, Vec<(Instruction, Span)>)],
) -> bool {
    if insertions.is_empty() {
        return true;
    }

    if !insertions.windows(2).all(|pair| pair[0].0 < pair[1].0)
        || insertions
            .iter()
            .any(|(index, instructions)| instructions.is_empty() || *index > chunk.code.len())
    {
        return false;
    }

    let boundaries = straight_line_insertion_boundaries(chunk);
    if insertions.iter().any(|(index, _)| !boundaries[*index]) {
        return false;
    }

    let total = insertions
        .iter()
        .map(|(_, instructions)| instructions.len())
        .sum::<usize>();
    let old_code = mem::take(&mut chunk.code);
    let old_spans = mem::take(&mut chunk.spans);
    chunk.code.reserve(old_code.len() + total);
    chunk.spans.reserve(old_spans.len() + total);

    let mut insertion = 0usize;
    for (index, (instruction, span)) in old_code.into_iter().zip(old_spans).enumerate() {
        if insertions
            .get(insertion)
            .is_some_and(|(at, _)| *at == index)
        {
            for (inserted, inserted_span) in &insertions[insertion].1 {
                chunk.code.push(*inserted);
                chunk.spans.push(*inserted_span);
            }
            insertion += 1;
        }

        chunk.code.push(instruction);
        chunk.spans.push(span);
    }
    if let Some((_, trailing)) = insertions.get(insertion) {
        for (instruction, span) in trailing {
            chunk.code.push(*instruction);
            chunk.spans.push(*span);
        }
    }

    for entry in &mut chunk.catch_table {
        entry.start = shifted_position(entry.start, insertions);
        entry.end = shifted_position(entry.end, insertions);
        entry.handler = shifted_position(entry.handler, insertions);
    }

    true
}

fn shifted_position(position: u32, insertions: &[(usize, Vec<(Instruction, Span)>)]) -> u32 {
    let shift = insertions
        .iter()
        .take_while(|(index, _)| *index <= position as usize)
        .map(|(_, instructions)| instructions.len())
        .sum::<usize>();
    let shift = u32::try_from(shift).unwrap_or_else(|_| {
        // SAFETY: bytecode insertion widths fit the chunk's u32 metadata.
        unsafe { unreachable_invariant("optimized insertion width must fit u32") }
    });
    position.checked_add(shift).unwrap_or_else(|| {
        // SAFETY: verified chunks and bounded insertions fit the u32 instruction space.
        unsafe { unreachable_invariant("optimized instruction position must fit u32") }
    })
}

pub(in crate::optimizer) fn insert_straight_line_before(
    chunk: &mut Chunk,
    index: usize,
    instructions: &[(Instruction, Span)],
) -> bool {
    if instructions.is_empty() {
        return true;
    }

    if !can_insert_straight_line_before(chunk, index) {
        return false;
    }

    // SAFETY: an insertion width is bounded by the chunk's u32 instruction metadata.
    let width = unsafe {
        unwrap_result_invariant(
            u32::try_from(instructions.len()),
            "an instruction insertion must fit the chunk's u32 metadata",
        )
    };

    for entry in &mut chunk.catch_table {
        if entry.start as usize >= index {
            // SAFETY: the new catch start fits `u32`.
            entry.start = unsafe {
                unwrap_option_invariant(
                    entry.start.checked_add(width),
                    "optimized catch start must fit u32",
                )
            };
        }

        if entry.end as usize >= index {
            // SAFETY: the new catch end fits `u32`.
            entry.end = unsafe {
                unwrap_option_invariant(
                    entry.end.checked_add(width),
                    "optimized catch end must fit u32",
                )
            };
        }

        if entry.handler as usize >= index {
            // SAFETY: the new handler fits `u32`.
            entry.handler = unsafe {
                unwrap_option_invariant(
                    entry.handler.checked_add(width),
                    "optimized catch handler must fit u32",
                )
            };
        }
    }

    chunk.code.splice(
        index..index,
        instructions.iter().map(|(instruction, _)| *instruction),
    );

    chunk
        .spans
        .splice(index..index, instructions.iter().map(|(_, span)| *span));

    true
}

#[cfg(test)]
mod tests {
    use whim_span::Span;

    use crate::bytecode::chunk::Chunk;
    use crate::bytecode::chunk::descriptors::CatchEntry;
    use crate::bytecode::instruction::Instruction;
    use crate::bytecode::instruction::operands::DescriptorIndex;
    use crate::bytecode::instruction::operands::JumpOffset;
    use crate::bytecode::instruction::operands::Register;
    use crate::optimizer::rewrite::splice::insert_straight_line_before;
    use crate::optimizer::rewrite::splice::insert_straight_line_before_many;
    use crate::optimizer::rewrite::splice::splice_replace_many;

    fn emit(chunk: &mut Chunk, instruction: Instruction) {
        chunk.emit(instruction, Span::zero());
    }

    #[test]
    fn replaces_multiple_instructions_and_rebases_crossing_jumps_once() {
        let mut chunk = Chunk::new();
        emit(
            &mut chunk,
            Instruction::Jump {
                offset: JumpOffset::new(4),
            },
        );
        emit(
            &mut chunk,
            Instruction::LoadNull {
                destination: Register::new(0),
            },
        );
        emit(
            &mut chunk,
            Instruction::LoadNull {
                destination: Register::new(1),
            },
        );
        emit(
            &mut chunk,
            Instruction::Jump {
                offset: JumpOffset::new(-3),
            },
        );
        emit(&mut chunk, Instruction::ReturnNull);
        chunk.catch_table.push(CatchEntry {
            start: 1,
            end: 3,
            handler: 4,
            type_descriptor: DescriptorIndex::new(0),
            temporary_floor: 0,
            binding: None,
        });

        let replacements = vec![
            (
                1,
                vec![
                    (
                        Instruction::LoadNull {
                            destination: Register::new(2),
                        },
                        Span::zero(),
                    ),
                    (
                        Instruction::LoadNull {
                            destination: Register::new(3),
                        },
                        Span::zero(),
                    ),
                ],
            ),
            (
                2,
                vec![
                    (
                        Instruction::LoadNull {
                            destination: Register::new(4),
                        },
                        Span::zero(),
                    ),
                    (
                        Instruction::LoadNull {
                            destination: Register::new(5),
                        },
                        Span::zero(),
                    ),
                    (
                        Instruction::LoadNull {
                            destination: Register::new(6),
                        },
                        Span::zero(),
                    ),
                ],
            ),
        ];

        splice_replace_many(&mut chunk, &replacements);

        assert_eq!(chunk.code.len(), 8);
        assert_eq!(
            chunk.code[0],
            Instruction::Jump {
                offset: JumpOffset::new(7)
            }
        );
        assert_eq!(
            chunk.code[6],
            Instruction::Jump {
                offset: JumpOffset::new(-6)
            }
        );
        assert!(matches!(chunk.code[7], Instruction::ReturnNull));
        let catch = chunk.catch_table[0];
        assert_eq!((catch.start, catch.end, catch.handler), (1, 6, 7));
    }

    #[test]
    fn batches_straight_line_insertions() {
        let mut source = Chunk::new();
        for index in 0..4u16 {
            emit(
                &mut source,
                Instruction::LoadNull {
                    destination: Register::new(index),
                },
            );
        }
        emit(&mut source, Instruction::ReturnNull);

        let insertions = [1usize, 3]
            .into_iter()
            .map(|index| {
                (
                    index,
                    vec![(
                        Instruction::LoadNull {
                            destination: Register::new((index + 4) as u16),
                        },
                        Span::zero(),
                    )],
                )
            })
            .collect::<Vec<_>>();

        let mut legacy = source.clone();
        for (index, instructions) in insertions.iter().rev() {
            assert!(insert_straight_line_before(
                &mut legacy,
                *index,
                instructions,
            ));
        }

        let mut batched = source;
        assert!(insert_straight_line_before_many(&mut batched, &insertions,));

        assert_eq!(legacy.code, batched.code);
        assert_eq!(legacy.spans, batched.spans);
    }
}
