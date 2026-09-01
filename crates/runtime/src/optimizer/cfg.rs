//! Control-flow graph queries shared by optimizer passes.

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::instruction::Instruction;
pub(in crate::optimizer) use crate::bytecode::rewrite::control_flow_targets;
pub(in crate::optimizer) use crate::bytecode::rewrite::for_each_control_flow_target;
use crate::unwrap_result_invariant;

pub(in crate::optimizer) fn successors(chunk: &Chunk, index: usize, successors: &mut Vec<usize>) {
    match chunk.code[index] {
        Instruction::Jump { offset } | Instruction::NumericRegionJump { offset } => {
            successors.push(relative_target(index, offset.offset()));
        }
        Instruction::IncrementJump { offset, .. } => {
            successors.push(relative_target(index, i32::from(offset.offset())));
        }
        Instruction::CounterLoop { offset, .. }
        | Instruction::IntCounterLoop { offset, .. }
        | Instruction::IntStepLoop { offset, .. }
        | Instruction::NumericLoop { offset, .. }
        | Instruction::IntNumericLoop { offset, .. }
        | Instruction::PreparedIntNumericLoop { offset, .. }
        | Instruction::JumpUnless { offset, .. }
        | Instruction::IntJumpUnless { offset, .. }
        | Instruction::StringJumpUnless { offset, .. }
        | Instruction::StringByteJumpUnlessEqual { offset, .. }
        | Instruction::StringByteJumpUnlessNotEqual { offset, .. }
        | Instruction::IntJumpUnlessImmediate { offset, .. }
        | Instruction::JumpUnlessConstant { offset, .. }
        | Instruction::IntRangeJumpIf { offset, .. }
        | Instruction::IntRangeJumpUnless { offset, .. } => {
            successors.push(index + 1);
            successors.push(relative_target(index, i32::from(offset.offset())));
        }
        Instruction::JumpIfFalse { offset, .. }
        | Instruction::JumpIfTrue { offset, .. }
        | Instruction::JumpIfNull { offset, .. }
        | Instruction::JumpIfNotNull { offset, .. }
        | Instruction::FillDefault { offset, .. }
        | Instruction::FloatSquaresSumBranch { offset, .. } => {
            successors.push(index + 1);
            successors.push(relative_target(index, offset.offset()));
        }
        Instruction::BoolPatternBranch {
            false_offset,
            default_offset,
            ..
        } => {
            successors.push(index + 1);
            successors.push(relative_target(index, i32::from(false_offset.offset())));
            successors.push(relative_target(index, i32::from(default_offset.offset())));
        }
        Instruction::SwitchInt { table, .. }
        | Instruction::SwitchString { table, .. }
        | Instruction::SwitchBool { table, .. }
        | Instruction::SwitchFloat { table, .. }
        | Instruction::SwitchPattern { table, .. }
        | Instruction::SwitchTuplePattern { table, .. } => {
            match &chunk.switch_tables[usize::from(table.index())] {
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
                        successors.push(relative_target(index, *offset));
                    }
                    successors.push(relative_target(index, *default));
                }
                SwitchTable::String { arms, default, .. } => {
                    for (_, offset) in arms {
                        successors.push(relative_target(index, *offset));
                    }
                    successors.push(relative_target(index, *default));
                }
            }
        }
        Instruction::ForeachNext { .. }
        | Instruction::VecForeachNext { .. }
        | Instruction::DictForeachNext { .. } => {
            successors.push(index + 1);
            successors.push(index + 2);
        }
        Instruction::Return { .. }
        | Instruction::ReturnUnchecked { .. }
        | Instruction::ReturnReferenceUnchecked { .. }
        | Instruction::ReturnPairUnchecked { .. }
        | Instruction::ReturnScalarUnchecked { .. }
        | Instruction::ReturnNull
        | Instruction::ReturnNullUnchecked
        | Instruction::ReturnIntUnchecked { .. }
        | Instruction::Throw { .. }
        | Instruction::Rethrow
        | Instruction::ThrowUnhandledMatch { .. }
        | Instruction::Exit { .. }
        | Instruction::Panic { .. } => {}
        _ => successors.push(index + 1),
    }
}

pub(in crate::optimizer) fn has_shared_switch_table(chunk: &Chunk) -> bool {
    let mut seen = HashSet::new();
    chunk.code.iter().any(|instruction| match instruction {
        Instruction::SwitchInt { table, .. }
        | Instruction::SwitchString { table, .. }
        | Instruction::SwitchBool { table, .. }
        | Instruction::SwitchFloat { table, .. }
        | Instruction::SwitchPattern { table, .. }
        | Instruction::SwitchTuplePattern { table, .. } => !seen.insert(table.index()),
        _ => false,
    })
}

/// Whether execution may leave the current straight-line basic block after
/// this instruction.
pub(in crate::optimizer) fn branches_or_terminates(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Jump { .. }
            | Instruction::NumericRegionJump { .. }
            | Instruction::JumpIfFalse { .. }
            | Instruction::JumpIfTrue { .. }
            | Instruction::JumpIfNull { .. }
            | Instruction::JumpIfNotNull { .. }
            | Instruction::JumpUnless { .. }
            | Instruction::IntJumpUnless { .. }
            | Instruction::StringJumpUnless { .. }
            | Instruction::StringByteJumpUnlessEqual { .. }
            | Instruction::StringByteJumpUnlessNotEqual { .. }
            | Instruction::IntJumpUnlessImmediate { .. }
            | Instruction::JumpUnlessConstant { .. }
            | Instruction::IntRangeJumpIf { .. }
            | Instruction::IntRangeJumpUnless { .. }
            | Instruction::BoolPatternBranch { .. }
            | Instruction::FloatSquaresSumBranch { .. }
            | Instruction::SwitchInt { .. }
            | Instruction::SwitchString { .. }
            | Instruction::SwitchBool { .. }
            | Instruction::SwitchFloat { .. }
            | Instruction::SwitchPattern { .. }
            | Instruction::SwitchTuplePattern { .. }
            | Instruction::IncrementJump { .. }
            | Instruction::CounterLoop { .. }
            | Instruction::NumericLoop { .. }
            | Instruction::IntCounterLoop { .. }
            | Instruction::IntNumericLoop { .. }
            | Instruction::PreparedIntNumericLoop { .. }
            | Instruction::IntStepLoop { .. }
            | Instruction::Return { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnNull
            | Instruction::ReturnNullUnchecked
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::Throw { .. }
            | Instruction::Rethrow
            | Instruction::ThrowUnhandledMatch { .. }
            | Instruction::ForeachNext { .. }
            | Instruction::VecForeachNext { .. }
            | Instruction::DictForeachNext { .. }
            | Instruction::Exit { .. }
            | Instruction::Panic { .. }
            | Instruction::FillDefault { .. }
    )
}

/// Whether every path from the chunk entry to `target` passes `candidate`.
pub(in crate::optimizer) fn dominates(chunk: &Chunk, candidate: usize, target: usize) -> bool {
    if candidate == 0 {
        return true;
    }

    let mut visited = vec![false; chunk.code.len()];
    let mut pending = vec![0usize];
    visited[candidate] = true;
    while let Some(index) = pending.pop() {
        if index == target {
            return false;
        }
        if visited[index] {
            continue;
        }
        visited[index] = true;

        let mut edges = Vec::new();
        successors(chunk, index, &mut edges);
        for edge in edges {
            if edge < chunk.code.len() && !visited[edge] {
                pending.push(edge);
            }
        }
    }

    true
}

pub(in crate::optimizer) struct Dominators {
    block_of: Vec<usize>,
    immediate: Vec<Option<usize>>,
}

impl Dominators {
    pub(in crate::optimizer) fn new(chunk: &Chunk) -> Self {
        let mut leaders = vec![false; chunk.code.len()];
        leaders[0] = true;
        for (index, instruction) in chunk.code.iter().copied().enumerate() {
            if !branches_or_terminates(instruction) {
                continue;
            }
            if index + 1 < chunk.code.len() {
                leaders[index + 1] = true;
            }
            let mut edges = Vec::new();
            successors(chunk, index, &mut edges);
            for edge in edges {
                if edge < leaders.len() {
                    leaders[edge] = true;
                }
            }
        }

        let starts: Vec<_> = leaders
            .iter()
            .enumerate()
            .filter_map(|(index, leader)| leader.then_some(index))
            .collect();
        let mut block_of = vec![0; chunk.code.len()];
        for (block, start) in starts.iter().copied().enumerate() {
            let end = starts.get(block + 1).copied().unwrap_or(chunk.code.len());
            block_of[start..end].fill(block);
        }

        let mut outgoing = vec![Vec::new(); starts.len()];
        let mut incoming = vec![Vec::new(); starts.len()];
        for (block, _) in starts.iter().enumerate() {
            let end = starts.get(block + 1).copied().unwrap_or(chunk.code.len());
            let mut edges = Vec::new();
            successors(chunk, end - 1, &mut edges);
            for edge in edges {
                if edge >= chunk.code.len() {
                    continue;
                }
                let target = block_of[edge];
                if outgoing[block].contains(&target) {
                    continue;
                }
                outgoing[block].push(target);
                incoming[target].push(block);
            }
        }

        let mut visited = vec![false; starts.len()];
        let mut postorder = Vec::with_capacity(starts.len());
        let mut work = vec![(0, false)];
        while let Some((block, exiting)) = work.pop() {
            if exiting {
                postorder.push(block);
                continue;
            }
            if visited[block] {
                continue;
            }
            visited[block] = true;
            work.push((block, true));
            for successor in outgoing[block].iter().rev() {
                if !visited[*successor] {
                    work.push((*successor, false));
                }
            }
        }
        postorder.reverse();

        let mut rank = vec![usize::MAX; starts.len()];
        for (position, block) in postorder.iter().copied().enumerate() {
            rank[block] = position;
        }
        let mut immediate = vec![None; starts.len()];
        immediate[0] = Some(0);
        let mut changed = true;
        while changed {
            changed = false;
            for block in postorder.iter().copied().skip(1) {
                let mut predecessors = incoming[block]
                    .iter()
                    .copied()
                    .filter(|predecessor| immediate[*predecessor].is_some());
                let Some(mut dominator) = predecessors.next() else {
                    continue;
                };
                for predecessor in predecessors {
                    dominator = intersect_dominators(&immediate, &rank, dominator, predecessor);
                }
                if immediate[block] != Some(dominator) {
                    immediate[block] = Some(dominator);
                    changed = true;
                }
            }
        }

        Self {
            block_of,
            immediate,
        }
    }

    pub(in crate::optimizer) fn dominates(&self, candidate: usize, target: usize) -> bool {
        let candidate_block = self.block_of[candidate];
        let mut target_block = self.block_of[target];
        if candidate_block == target_block {
            return candidate <= target;
        }
        while let Some(parent) = self.immediate[target_block] {
            if parent == target_block {
                return false;
            }
            if parent == candidate_block {
                return true;
            }
            target_block = parent;
        }
        false
    }
}

fn intersect_dominators(
    immediate: &[Option<usize>],
    rank: &[usize],
    mut left: usize,
    mut right: usize,
) -> usize {
    while left != right {
        while rank[left] > rank[right] {
            left = immediate[left].expect("a ranked block has an immediate dominator");
        }
        while rank[right] > rank[left] {
            right = immediate[right].expect("a ranked block has an immediate dominator");
        }
    }
    left
}

#[inline(always)]
pub(crate) fn relative_target(source: usize, offset: i32) -> usize {
    // SAFETY: a verified chunk only ever forms in-range jump targets, so the
    // conversion cannot fail on bytecode the engine executes.
    unsafe {
        unwrap_result_invariant(
            usize::try_from(source as i64 + i64::from(offset)),
            "verified bytecode has an in-range jump target",
        )
    }
}

pub(in crate::optimizer) fn is_block_boundary(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Jump { .. }
            | Instruction::NumericRegionJump { .. }
            | Instruction::JumpIfFalse { .. }
            | Instruction::JumpIfTrue { .. }
            | Instruction::JumpIfNull { .. }
            | Instruction::JumpIfNotNull { .. }
            | Instruction::SwitchInt { .. }
            | Instruction::SwitchString { .. }
            | Instruction::SwitchBool { .. }
            | Instruction::SwitchFloat { .. }
            | Instruction::SwitchPattern { .. }
            | Instruction::SwitchTuplePattern { .. }
            | Instruction::Return { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnNull
            | Instruction::ReturnNullUnchecked
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::Throw { .. }
            | Instruction::Rethrow
            | Instruction::ThrowUnhandledMatch { .. }
            | Instruction::ForeachNext { .. }
            | Instruction::VecForeachNext { .. }
            | Instruction::DictForeachNext { .. }
            | Instruction::Exit { .. }
            | Instruction::Panic { .. }
            | Instruction::FillDefault { .. }
            | Instruction::JumpUnless { .. }
            | Instruction::IntJumpUnless { .. }
            | Instruction::StringJumpUnless { .. }
            | Instruction::StringByteJumpUnlessEqual { .. }
            | Instruction::StringByteJumpUnlessNotEqual { .. }
            | Instruction::IntJumpUnlessImmediate { .. }
            | Instruction::JumpUnlessConstant { .. }
            | Instruction::IntRangeJumpIf { .. }
            | Instruction::IntRangeJumpUnless { .. }
            | Instruction::BoolPatternBranch { .. }
            | Instruction::IncrementJump { .. }
            | Instruction::CounterLoop { .. }
            | Instruction::IntCounterLoop { .. }
            | Instruction::NumericLoop { .. }
            | Instruction::IntNumericLoop { .. }
            | Instruction::PreparedIntNumericLoop { .. }
            | Instruction::IntStepLoop { .. }
            | Instruction::FloatSquaresSumBranch { .. }
    )
}
