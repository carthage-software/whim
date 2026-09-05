//! Conflict-safe instruction replacement and removal for one analysis round.

#[cfg(test)]
mod tests;

use hashbrown::HashMap;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::LiteralKey;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::literal_key;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::SwitchTableIndex;
use crate::bytecode::rewrite::compact;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::analysis::AnalyzedChunk;
use crate::optimizer::passes::FunctionLocation;
use crate::optimizer::passes::chunk_mut;
use crate::unreachable_invariant;

struct ChunkRewrite {
    location: FunctionLocation,
    replacements: Vec<Option<Instruction>>,
    removals: Vec<bool>,
    constants: Vec<Literal>,
    switch_tables: Vec<SwitchTable>,
    constant_index: Option<HashMap<LiteralKey, ConstantIndex>>,
}

/// Every rewrite proven by the shared analysis in one optimizer round.
pub(in crate::optimizer) struct RewritePlan {
    chunks: Vec<ChunkRewrite>,
}

/// The structural changes applied by one rewrite plan.
#[derive(Clone, Copy, Default)]
pub(in crate::optimizer) struct RewriteResult {
    pub(in crate::optimizer) replacements: usize,
    pub(in crate::optimizer) removals: usize,
}

impl RewritePlan {
    pub(in crate::optimizer) fn for_analysis(analysis: &Analysis<'_>) -> Self {
        Self {
            chunks: analysis
                .chunks()
                .iter()
                .map(|analyzed| ChunkRewrite::new(analyzed.location))
                .collect(),
        }
    }

    pub(in crate::optimizer) fn is_available(
        &self,
        analyzed: &AnalyzedChunk<'_>,
        index: usize,
    ) -> bool {
        let rewrite = &self.chunks[analyzed.position];
        if rewrite.replacements.is_empty() {
            return true;
        }

        !rewrite.removals[index] && rewrite.replacements[index].is_none()
    }

    pub(in crate::optimizer) fn replacement(
        &self,
        analyzed: &AnalyzedChunk<'_>,
        index: usize,
    ) -> Option<Instruction> {
        self.chunks[analyzed.position]
            .replacements
            .get(index)
            .copied()
            .flatten()
    }

    pub(in crate::optimizer) fn has_replacements(&self, analyzed: &AnalyzedChunk<'_>) -> bool {
        self.chunks[analyzed.position]
            .replacements
            .iter()
            .any(Option::is_some)
    }

    /// Keeps the first replacement proven for an instruction in pass order.
    pub(in crate::optimizer) fn replace(
        &mut self,
        analyzed: &AnalyzedChunk<'_>,
        index: usize,
        instruction: Instruction,
    ) -> bool {
        let rewrite = &mut self.chunks[analyzed.position];
        rewrite.prepare(analyzed.chunk.code.len());
        if rewrite.removals[index] || rewrite.replacements[index].is_some() {
            return false;
        }

        rewrite.replacements[index] = Some(instruction);
        true
    }

    /// Removes an instruction even if an earlier pass planned to replace it.
    pub(in crate::optimizer) fn remove(
        &mut self,
        analyzed: &AnalyzedChunk<'_>,
        index: usize,
    ) -> bool {
        let rewrite = &mut self.chunks[analyzed.position];
        rewrite.prepare(analyzed.chunk.code.len());
        if rewrite.removals[index] {
            return false;
        }

        rewrite.replacements[index] = None;
        rewrite.removals[index] = true;
        true
    }

    pub(in crate::optimizer) fn intern_constant(
        &mut self,
        analyzed: &AnalyzedChunk<'_>,
        literal: Literal,
    ) -> Option<ConstantIndex> {
        let rewrite = &mut self.chunks[analyzed.position];
        let key = literal_key(&literal);
        let constants = &analyzed.chunk.constants;
        let constant_index = rewrite.constant_index.get_or_insert_with(|| {
            constants
                .iter()
                .enumerate()
                .filter_map(|(position, literal)| {
                    let position = u16::try_from(position).ok()?;
                    Some((literal_key(literal), ConstantIndex::new(position)))
                })
                .collect()
        });
        if let Some(index) = constant_index.get(&key) {
            return Some(*index);
        }

        let position = analyzed
            .chunk
            .constants
            .len()
            .checked_add(rewrite.constants.len())?;
        let index = u16::try_from(position).ok().map(ConstantIndex::new)?;
        rewrite.constants.push(literal);
        constant_index.insert(key, index);
        Some(index)
    }

    pub(in crate::optimizer) fn apply(self, unit: &mut CompiledUnit) -> RewriteResult {
        let mut result = RewriteResult::default();
        for rewrite in self.chunks {
            if rewrite.replacements.is_empty() && rewrite.constants.is_empty() {
                continue;
            }

            let chunk = chunk_mut(unit, rewrite.location);
            let added_switch_tables = !rewrite.switch_tables.is_empty();
            for table in rewrite.switch_tables {
                chunk
                    .add_switch_table(table)
                    .expect("a reserved switch table fits its chunk");
            }

            for literal in rewrite.constants {
                if chunk.push_constant(literal).is_err() {
                    // SAFETY: constant indexes are reserved before a rewrite is planned.
                    unsafe { unreachable_invariant("a planned constant fits its chunk") }
                }
            }

            let mut removals = rewrite.removals;
            for (index, removed) in removals.iter_mut().enumerate().take(chunk.code.len()) {
                if *removed {
                    result.removals += 1;
                } else if let Some(replacement) = rewrite.replacements[index] {
                    chunk.code[index] = replacement;
                    result.replacements += 1;
                }
            }

            if removals.iter().any(|removed| *removed) {
                compact(chunk, &removals);
            }
            if added_switch_tables {
                compact_switch_tables(chunk);
            }
        }

        result
    }

    pub(in crate::optimizer) fn add_switch_table(
        &mut self,
        analyzed: &AnalyzedChunk<'_>,
        table: SwitchTable,
    ) -> Option<SwitchTableIndex> {
        let rewrite = &mut self.chunks[analyzed.position];
        let position = analyzed
            .chunk
            .switch_tables
            .len()
            .checked_add(rewrite.switch_tables.len())?;
        let index = SwitchTableIndex::new(u16::try_from(position).ok()?);
        rewrite.switch_tables.push(table);
        Some(index)
    }
}

fn compact_switch_tables(chunk: &mut Chunk) {
    let mut mapping = vec![None; chunk.switch_tables.len()];
    for instruction in &mut chunk.code {
        if let Some(table) = switch_table_index(instruction) {
            mapping[usize::from(table.index())] = Some(*table);
        }
    }

    if mapping.iter().all(Option::is_some) {
        return;
    }

    let mut old = 0;
    let mut next = 0;
    chunk.switch_tables.retain(|_| {
        let mapped = &mut mapping[old];
        old += 1;
        if mapped.is_none() {
            return false;
        }

        *mapped = Some(SwitchTableIndex::new(
            u16::try_from(next).expect("retained switch tables fit their original index space"),
        ));

        next += 1;
        true
    });

    for instruction in &mut chunk.code {
        if let Some(table) = switch_table_index(instruction) {
            *table = mapping[usize::from(table.index())]
                .expect("every referenced switch table was retained");
        }
    }
}

const fn switch_table_index(instruction: &mut Instruction) -> Option<&mut SwitchTableIndex> {
    match instruction {
        Instruction::SwitchInt { table, .. }
        | Instruction::SwitchString { table, .. }
        | Instruction::SwitchBool { table, .. }
        | Instruction::SwitchFloat { table, .. }
        | Instruction::SwitchPattern { table, .. }
        | Instruction::SwitchTuplePattern { table, .. } => Some(table),
        _ => None,
    }
}

impl ChunkRewrite {
    fn new(location: FunctionLocation) -> Self {
        Self {
            location,
            replacements: Vec::new(),
            removals: Vec::new(),
            constants: Vec::new(),
            switch_tables: Vec::new(),
            constant_index: None,
        }
    }

    fn prepare(&mut self, instruction_count: usize) {
        if !self.replacements.is_empty() {
            return;
        }

        self.replacements.resize(instruction_count, None);
        self.removals.resize(instruction_count, false);
    }
}
