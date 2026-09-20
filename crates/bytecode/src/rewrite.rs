//! Structural bytecode rewrites shared by compilation and optimization.

use std::mem;

use hashbrown::HashSet;
use whim_base::u32_index;

use crate::chunk::Chunk;
use crate::chunk::descriptors::SwitchTable;
use crate::instruction::Instruction;
use crate::instruction::operands::JumpOffset;
use crate::instruction::operands::NearJumpOffset;
use crate::instruction::operands::ShortJumpOffset;

#[must_use]
pub fn control_flow_targets(chunk: &Chunk) -> HashSet<usize> {
    let mut targets = HashSet::new();
    for_each_control_flow_target(chunk, |target| {
        targets.insert(target);
    });

    targets
}

/// Visits instructions reached by a non-fallthrough edge.
#[expect(
    clippy::too_many_lines,
    reason = "the branch target table stays exhaustive in one match"
)]
pub fn for_each_control_flow_target(chunk: &Chunk, mut visit: impl FnMut(usize)) {
    for (index, instruction) in chunk.code.iter().enumerate() {
        match instruction {
            Instruction::Jump { offset }
            | Instruction::NumericRegionJump { offset }
            | Instruction::JumpIfFalse { offset, .. }
            | Instruction::JumpIfTrue { offset, .. }
            | Instruction::JumpIfNull { offset, .. }
            | Instruction::JumpIfNotNull { offset, .. }
            | Instruction::FillDefault { offset, .. }
            | Instruction::FloatSquaresSumBranch { offset, .. } => {
                visit(relative_target(index, offset.offset()));
            }
            Instruction::IndexCoalesce { offset, .. }
            | Instruction::VecIndexCoalesce { offset, .. }
            | Instruction::DictIndexCoalesceIntKey { offset, .. }
            | Instruction::DictIndexCoalesceStringKey { offset, .. }
            | Instruction::StringIndexCoalesce { offset, .. }
            | Instruction::PropertyCoalesce { offset, .. }
            | Instruction::PropertyCoalesceUnchecked { offset, .. } => {
                visit(relative_target(index, i32::from(offset.offset())));
            }
            Instruction::Coalesce { offset, .. }
            | Instruction::StaticPropertyCoalesce { offset, .. }
            | Instruction::JumpUnless { offset, .. }
            | Instruction::IntJumpUnless { offset, .. }
            | Instruction::StringJumpUnless { offset, .. }
            | Instruction::StringByteJumpUnlessEqual { offset, .. }
            | Instruction::StringByteJumpUnlessNotEqual { offset, .. }
            | Instruction::IntJumpUnlessImmediate { offset, .. }
            | Instruction::JumpUnlessConstant { offset, .. }
            | Instruction::IntRangeJumpIf { offset, .. }
            | Instruction::IntRangeJumpUnless { offset, .. }
            | Instruction::IncrementJump { offset, .. }
            | Instruction::CounterLoop { offset, .. }
            | Instruction::IntCounterLoop { offset, .. }
            | Instruction::IntStepLoop { offset, .. }
            | Instruction::NumericLoop { offset, .. }
            | Instruction::IntNumericLoop { offset, .. }
            | Instruction::PreparedIntNumericLoop { offset, .. } => {
                visit(relative_target(index, i32::from(offset.offset())));
            }
            Instruction::BoolPatternBranch {
                false_offset,
                default_offset,
                ..
            } => {
                visit(relative_target(index, i32::from(false_offset.offset())));
                visit(relative_target(index, i32::from(default_offset.offset())));
            }
            Instruction::SwitchInt { table, .. }
            | Instruction::SwitchString { table, .. }
            | Instruction::SwitchBool { table, .. }
            | Instruction::SwitchFloat { table, .. }
            | Instruction::SwitchPattern { table, .. }
            | Instruction::SwitchTuplePattern { table, .. } => {
                match &chunk.switch_tables[usize::from(table.index())] {
                    SwitchTable::Int {
                        targets: offsets,
                        default,
                        ..
                    }
                    | SwitchTable::StringByte {
                        targets: offsets,
                        default,
                        ..
                    } => {
                        for offset in offsets {
                            visit(relative_target(index, *offset));
                        }
                        visit(relative_target(index, *default));
                    }
                    SwitchTable::String { arms, default, .. } => {
                        for (_, offset) in arms {
                            visit(relative_target(index, *offset));
                        }
                        visit(relative_target(index, *default));
                    }
                    SwitchTable::Pattern {
                        targets, default, ..
                    }
                    | SwitchTable::DictionaryShape {
                        targets, default, ..
                    }
                    | SwitchTable::Bool { targets, default }
                    | SwitchTable::Float {
                        targets, default, ..
                    } => {
                        for offset in targets {
                            visit(relative_target(index, *offset));
                        }
                        visit(relative_target(index, *default));
                    }
                }
            }
            _ => {}
        }
    }

    for entry in &chunk.catch_table {
        visit(native_index(entry.start));
        visit(native_index(entry.end));
        visit(native_index(entry.handler));
    }
}

pub fn compact(chunk: &mut Chunk, remove: &[bool]) {
    let old_code = mem::take(&mut chunk.code);
    let old_spans = mem::take(&mut chunk.spans);
    let mut old_to_new = Vec::with_capacity(old_code.len() + 1);
    let mut next = 0;
    for removed in remove {
        old_to_new.push(next);
        if !removed {
            next += 1;
        }
    }

    old_to_new.push(next);
    for (old_index, mut instruction) in old_code.into_iter().enumerate() {
        if remove[old_index] {
            continue;
        }

        let new_index = old_to_new[old_index];
        rebase_targets(chunk, &mut instruction, old_index, new_index, &old_to_new);
        chunk.code.push(instruction);
        chunk.spans.push(old_spans[old_index]);
    }

    for entry in &mut chunk.catch_table {
        entry.start = u32_index(old_to_new[native_index(entry.start)]);
        entry.end = u32_index(old_to_new[native_index(entry.end)]);
        entry.handler = u32_index(old_to_new[native_index(entry.handler)]);
    }
}

/// Rewrites one instruction's relative targets after an index remap.
///
/// # Panics
///
/// Panics if the remap omits a target or a coalescing offset no longer fits.
pub fn rebase_targets(
    chunk: &mut Chunk,
    instruction: &mut Instruction,
    old_index: usize,
    new_index: usize,
    old_to_new: &[usize],
) {
    match instruction {
        Instruction::Jump { offset }
        | Instruction::NumericRegionJump { offset }
        | Instruction::JumpIfFalse { offset, .. }
        | Instruction::JumpIfTrue { offset, .. }
        | Instruction::JumpIfNull { offset, .. }
        | Instruction::JumpIfNotNull { offset, .. }
        | Instruction::FillDefault { offset, .. }
        | Instruction::FloatSquaresSumBranch { offset, .. } => {
            let target = relative_target(old_index, offset.offset());
            *offset = JumpOffset::new(new_offset(new_index, old_to_new[target]));
        }
        Instruction::BoolPatternBranch {
            false_offset,
            default_offset,
            ..
        } => {
            for offset in [false_offset, default_offset] {
                let target = relative_target(old_index, i32::from(offset.offset()));
                let relative = short_offset(new_index, old_to_new[target]);
                *offset = ShortJumpOffset::new(relative);
            }
        }
        Instruction::IndexCoalesce { offset, .. }
        | Instruction::VecIndexCoalesce { offset, .. }
        | Instruction::DictIndexCoalesceIntKey { offset, .. }
        | Instruction::DictIndexCoalesceStringKey { offset, .. }
        | Instruction::StringIndexCoalesce { offset, .. }
        | Instruction::PropertyCoalesce { offset, .. }
        | Instruction::PropertyCoalesceUnchecked { offset, .. } => {
            let target = relative_target(old_index, i32::from(offset.offset()));
            *offset = NearJumpOffset::new(
                i8::try_from(new_offset(new_index, old_to_new[target]))
                    .expect("a compacted coalescing branch fits its original offset"),
            );
        }
        Instruction::Coalesce { offset, .. }
        | Instruction::StaticPropertyCoalesce { offset, .. }
        | Instruction::JumpUnless { offset, .. }
        | Instruction::IntJumpUnless { offset, .. }
        | Instruction::StringJumpUnless { offset, .. }
        | Instruction::StringByteJumpUnlessEqual { offset, .. }
        | Instruction::StringByteJumpUnlessNotEqual { offset, .. }
        | Instruction::IntJumpUnlessImmediate { offset, .. }
        | Instruction::JumpUnlessConstant { offset, .. }
        | Instruction::IntRangeJumpIf { offset, .. }
        | Instruction::IntRangeJumpUnless { offset, .. }
        | Instruction::IncrementJump { offset, .. }
        | Instruction::CounterLoop { offset, .. }
        | Instruction::IntCounterLoop { offset, .. }
        | Instruction::NumericLoop { offset, .. }
        | Instruction::IntNumericLoop { offset, .. }
        | Instruction::PreparedIntNumericLoop { offset, .. }
        | Instruction::IntStepLoop { offset, .. } => {
            let target = relative_target(old_index, i32::from(offset.offset()));
            let relative = short_offset(new_index, old_to_new[target]);
            *offset = ShortJumpOffset::new(relative);
        }
        Instruction::SwitchInt { table, .. }
        | Instruction::SwitchString { table, .. }
        | Instruction::SwitchBool { table, .. }
        | Instruction::SwitchFloat { table, .. }
        | Instruction::SwitchPattern { table, .. }
        | Instruction::SwitchTuplePattern { table, .. } => {
            match &mut chunk.switch_tables[usize::from(table.index())] {
                SwitchTable::Int {
                    targets, default, ..
                }
                | SwitchTable::StringByte {
                    targets, default, ..
                }
                | SwitchTable::Pattern {
                    targets, default, ..
                }
                | SwitchTable::DictionaryShape {
                    targets, default, ..
                }
                | SwitchTable::Bool { targets, default }
                | SwitchTable::Float {
                    targets, default, ..
                } => {
                    for offset in targets {
                        let target = relative_target(old_index, *offset);
                        *offset = new_offset(new_index, old_to_new[target]);
                    }

                    let target = relative_target(old_index, *default);
                    *default = new_offset(new_index, old_to_new[target]);
                }
                SwitchTable::String { arms, default, .. } => {
                    for (_, offset) in arms {
                        let target = relative_target(old_index, *offset);
                        *offset = new_offset(new_index, old_to_new[target]);
                    }

                    let target = relative_target(old_index, *default);
                    *default = new_offset(new_index, old_to_new[target]);
                }
            }
        }
        _ => {}
    }
}

/// Resolves a jump offset from its source instruction.
///
/// # Panics
///
/// Panics if the source exceeds `u32::MAX` or the target is negative.
#[must_use]
#[inline]
pub fn relative_target(source: usize, offset: i32) -> usize {
    usize::try_from(wide_index(source) + i64::from(offset))
        .expect("a bytecode branch target must be non-negative")
}

fn short_offset(source: usize, target: usize) -> i16 {
    i16::try_from(new_offset(source, target)).expect("a short jump offset must fit in i16")
}

fn new_offset(source: usize, target: usize) -> i32 {
    i32::try_from(wide_index(target) - wide_index(source)).expect("a jump offset must fit in i32")
}

fn native_index(index: u32) -> usize {
    usize::try_from(index).expect("a bytecode index must fit in usize")
}

fn wide_index(index: usize) -> i64 {
    i64::from(u32_index(index))
}
