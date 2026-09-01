//! Conflict-safe instruction replacement and removal for one analysis round.

use hashbrown::HashMap;

use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::LiteralKey;
use crate::bytecode::chunk::descriptors::literal_key;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
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
        }

        result
    }
}

impl ChunkRewrite {
    fn new(location: FunctionLocation) -> Self {
        Self {
            location,
            replacements: Vec::new(),
            removals: Vec::new(),
            constants: Vec::new(),
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
