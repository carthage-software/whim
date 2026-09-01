//! Structural bytecode rewrites shared by compilation and optimization.

use std::mem;

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::unwrap_result_invariant;

pub(crate) fn control_flow_targets(chunk: &Chunk) -> HashSet<usize> {
    let mut targets = HashSet::new();
    for_each_control_flow_target(chunk, |target| {
        targets.insert(target);
    });

    targets
}

/// Visits instructions reached by a non-fallthrough edge.
pub(crate) fn for_each_control_flow_target(chunk: &Chunk, mut visit: impl FnMut(usize)) {
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
            Instruction::JumpUnless { offset, .. }
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

pub(crate) fn compact(chunk: &mut Chunk, remove: &[bool]) {
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
        entry.start = narrow_index(old_to_new[native_index(entry.start)]);
        entry.end = narrow_index(old_to_new[native_index(entry.end)]);
        entry.handler = narrow_index(old_to_new[native_index(entry.handler)]);
    }
}

/// Rewrites one instruction's relative targets after an index remap.
pub(crate) fn rebase_targets(
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
        Instruction::JumpUnless { offset, .. }
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

fn relative_target(source: usize, offset: i32) -> usize {
    // SAFETY: verified bytecode keeps every branch within its chunk.
    unsafe {
        unwrap_result_invariant(
            usize::try_from(wide_index(source) + i64::from(offset)),
            "a bytecode branch target must be non-negative",
        )
    }
}

fn short_offset(source: usize, target: usize) -> i16 {
    // SAFETY: removing instructions cannot widen an existing short branch.
    unsafe {
        unwrap_result_invariant(
            i16::try_from(new_offset(source, target)),
            "compaction cannot widen a short jump beyond its old range",
        )
    }
}

fn new_offset(source: usize, target: usize) -> i32 {
    // SAFETY: bytecode keeps every jump distance within the i32 range.
    unsafe {
        unwrap_result_invariant(
            i32::try_from(wide_index(target) - wide_index(source)),
            "bytecode has an in-range jump offset",
        )
    }
}

fn narrow_index(index: usize) -> u32 {
    // SAFETY: bytecode positions fit in the chunk's 32-bit index space.
    unsafe {
        unwrap_result_invariant(
            u32::try_from(index),
            "bytecode has a thirty-two-bit instruction index",
        )
    }
}

fn native_index(index: u32) -> usize {
    // SAFETY: supported hosts can address every 32-bit bytecode position.
    unsafe {
        unwrap_result_invariant(
            usize::try_from(index),
            "bytecode has a host-sized instruction index",
        )
    }
}

fn wide_index(index: usize) -> i64 {
    i64::from(narrow_index(index))
}
