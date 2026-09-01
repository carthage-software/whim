//! Register liveness queries shared by optimizer passes.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::operands::for_each_read_register;
use crate::optimizer::operands::for_each_write_register;

pub(super) mod effect;

const DENSE_QUERY_THRESHOLD: usize = 1024;
const MAX_DENSE_MATRIX_WORDS: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Effect {
    None,
    Read,
    Write,
    ReadWrite,
}

#[derive(Default)]
pub(super) struct LivenessScratch {
    work: Vec<usize>,
    seen: Vec<u32>,
    generation: u32,
}

impl LivenessScratch {
    fn begin(&mut self, length: usize) {
        self.work.clear();
        self.seen.resize(length, 0);
        if self.generation == u32::MAX {
            self.seen.fill(0);
            self.generation = 1;
        } else {
            self.generation += 1;
        }
    }
}

pub(super) struct Liveness {
    live_in: Vec<u64>,
    instruction_count: usize,
    word_count: usize,
}

pub(super) enum LivenessQueries {
    Cached(Liveness),
    Targeted,
    Skipped,
}

impl LivenessQueries {
    pub(crate) fn for_chunk(chunk: &Chunk, query_count: usize) -> Self {
        Self::build(chunk, &[], query_count)
    }

    pub(crate) fn for_effective_code(chunk: &Chunk, code: &[Instruction]) -> Self {
        let word_count = usize::from(chunk.register_count).div_ceil(u64::BITS as usize);
        let Some(matrix_words) = chunk.code.len().checked_mul(word_count) else {
            return Self::Skipped;
        };
        if matrix_words > MAX_DENSE_MATRIX_WORDS {
            return Self::Skipped;
        }

        Self::Cached(Liveness::analyze(chunk, &[], Some(code)))
    }

    fn build(chunk: &Chunk, removed: &[bool], query_count: usize) -> Self {
        if query_count < 2 || query_count.saturating_mul(chunk.code.len()) < DENSE_QUERY_THRESHOLD {
            return Self::Targeted;
        }

        let word_count = usize::from(chunk.register_count).div_ceil(u64::BITS as usize);
        let Some(matrix_words) = chunk.code.len().checked_mul(word_count) else {
            return Self::Skipped;
        };
        if matrix_words > MAX_DENSE_MATRIX_WORDS {
            return Self::Skipped;
        }

        Self::Cached(Liveness::analyze(chunk, removed, None))
    }

    pub(crate) fn register_is_dead_after(
        &self,
        chunk: &Chunk,
        register: Register,
        start: usize,
    ) -> bool {
        match self {
            Self::Cached(liveness) => liveness.register_is_dead_after(register, start),
            Self::Targeted => register_is_dead_after(chunk, register, start),
            Self::Skipped => false,
        }
    }
}

impl Liveness {
    fn analyze(chunk: &Chunk, removed: &[bool], effective_code: Option<&[Instruction]>) -> Self {
        let instruction_count = chunk.code.len();
        let code = effective_code.unwrap_or(&chunk.code);
        let register_count = usize::from(chunk.register_count);
        let word_count = register_count.div_ceil(u64::BITS as usize);
        let mut live_in = vec![0; instruction_count * word_count];
        let mut next = vec![0; word_count];
        let mut edges = Vec::new();

        loop {
            let mut changed = false;
            for index in (0..instruction_count).rev() {
                next.fill(0);
                if removed.get(index).copied().unwrap_or(false) {
                    if index + 1 < instruction_count {
                        next.copy_from_slice(words_at(&live_in, word_count, index + 1));
                    }
                    let current = words_at_mut(&mut live_in, word_count, index);
                    if current != next {
                        current.copy_from_slice(&next);
                        changed = true;
                    }
                    continue;
                }

                edges.clear();
                successors(chunk, index, &mut edges);
                for successor in edges
                    .iter()
                    .copied()
                    .filter(|successor| *successor < instruction_count)
                {
                    union_words(&mut next, words_at(&live_in, word_count, successor));
                }

                let instruction = code[index];
                if for_each_write_register(instruction, |register| {
                    clear_register(&mut next, register)
                }) {
                    for entry in &chunk.catch_table {
                        if index >= entry.start as usize && index < entry.end as usize {
                            union_words(
                                &mut next,
                                words_at(&live_in, word_count, entry.handler as usize),
                            );
                        }
                    }
                    for_each_read_register(instruction, |register| {
                        set_register(&mut next, register)
                    });
                } else {
                    apply_unclassified_effects(chunk, instruction, &mut next, register_count);
                    for entry in &chunk.catch_table {
                        if index >= entry.start as usize && index < entry.end as usize {
                            union_words(
                                &mut next,
                                words_at(&live_in, word_count, entry.handler as usize),
                            );
                        }
                    }
                }

                let current = words_at_mut(&mut live_in, word_count, index);
                if current != next {
                    current.copy_from_slice(&next);
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        Self {
            live_in,
            instruction_count,
            word_count,
        }
    }

    pub(crate) fn register_is_dead_after(&self, register: Register, start: usize) -> bool {
        if start >= self.instruction_count || self.word_count == 0 {
            return true;
        }

        let bit = usize::from(register.index());
        self.live_in[start * self.word_count + bit / u64::BITS as usize]
            & (1u64 << (bit % u64::BITS as usize))
            == 0
    }
}

fn apply_unclassified_effects(
    chunk: &Chunk,
    instruction: Instruction,
    words: &mut [u64],
    register_count: usize,
) {
    for index in 0..register_count {
        let register = Register::new(index as u16);
        let effect = effect_on(chunk, instruction, register);
        if effect.writes() {
            clear_register(words, register);
        }
        if effect.reads() {
            set_register(words, register);
        }
    }
}

fn words_at(words: &[u64], word_count: usize, index: usize) -> &[u64] {
    &words[index * word_count..(index + 1) * word_count]
}

fn words_at_mut(words: &mut [u64], word_count: usize, index: usize) -> &mut [u64] {
    &mut words[index * word_count..(index + 1) * word_count]
}

fn union_words(destination: &mut [u64], source: &[u64]) {
    for (destination, source) in destination.iter_mut().zip(source) {
        *destination |= *source;
    }
}

fn set_register(words: &mut [u64], register: Register) {
    let bit = usize::from(register.index());
    words[bit / u64::BITS as usize] |= 1u64 << (bit % u64::BITS as usize);
}

fn clear_register(words: &mut [u64], register: Register) {
    let bit = usize::from(register.index());
    words[bit / u64::BITS as usize] &= !(1u64 << (bit % u64::BITS as usize));
}

impl Effect {
    pub(crate) const fn reads(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    pub(crate) const fn writes(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }

    pub(crate) const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

pub(super) fn register_is_dead_after(chunk: &Chunk, register: Register, start: usize) -> bool {
    register_is_dead_after_removals(chunk, register, start, &[])
}

/// Like [`register_is_dead_after`], treating every instruction marked in
/// `removed` as an already-deleted no-op that only falls through.
pub(super) fn register_is_dead_after_removals(
    chunk: &Chunk,
    register: Register,
    start: usize,
    removed: &[bool],
) -> bool {
    register_is_dead_after_removals_with_scratch(
        chunk,
        register,
        start,
        removed,
        &mut LivenessScratch::default(),
    )
}

pub(super) fn register_is_dead_after_removals_with_scratch(
    chunk: &Chunk,
    register: Register,
    start: usize,
    removed: &[bool],
    scratch: &mut LivenessScratch,
) -> bool {
    scratch.begin(chunk.code.len());
    scratch.work.push(start);
    while let Some(index) = scratch.work.pop() {
        if index == chunk.code.len() || scratch.seen[index] == scratch.generation {
            continue;
        }
        scratch.seen[index] = scratch.generation;

        if removed.get(index).copied().unwrap_or(false) {
            scratch.work.push(index + 1);
            continue;
        }

        for entry in &chunk.catch_table {
            if index >= entry.start as usize && index < entry.end as usize {
                scratch.work.push(entry.handler as usize);
            }
        }

        if let Instruction::Clear { target } = chunk.code[index]
            && target == register
        {
            continue;
        }

        let effect = effect_on(chunk, chunk.code[index], register);
        if effect.reads() {
            return false;
        }
        if effect.writes() {
            continue;
        }
        successors(chunk, index, &mut scratch.work);
    }

    true
}

pub(super) fn register_is_unused_after(chunk: &Chunk, register: Register, start: usize) -> bool {
    let mut work = vec![start];
    let mut seen = vec![false; chunk.code.len()];
    while let Some(index) = work.pop() {
        if index == chunk.code.len() || seen[index] {
            continue;
        }

        seen[index] = true;

        for entry in &chunk.catch_table {
            if index >= entry.start as usize && index < entry.end as usize {
                work.push(entry.handler as usize);
            }
        }

        if !effect_on(chunk, chunk.code[index], register).is_none() {
            return false;
        }

        successors(chunk, index, &mut work);
    }

    true
}

pub(super) fn register_is_untouched_between(
    chunk: &Chunk,
    register: Register,
    start: usize,
    end: usize,
) -> bool {
    chunk.code[start..end]
        .iter()
        .all(|instruction| effect_on(chunk, *instruction, register).is_none())
}

/// Whether `register` is read before it is next overwritten in the half-open
/// instruction range. A read-modify-write counts as a read.
pub(super) fn register_is_read_before_write(
    chunk: &Chunk,
    register: Register,
    start: usize,
    end: usize,
) -> bool {
    for instruction in &chunk.code[start..end] {
        let effect = effect_on(chunk, *instruction, register);
        if effect.reads() {
            return true;
        }
        if effect.writes() {
            return false;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use whim_span::Span;

    use crate::bytecode::chunk::Chunk;
    use crate::bytecode::instruction::Instruction;
    use crate::optimizer::liveness::LivenessQueries;

    #[test]
    fn skips_a_dense_matrix_above_the_memory_budget() {
        let mut chunk = Chunk::new();
        chunk.register_count = u16::MAX;
        for _ in 0..1025 {
            chunk.emit(Instruction::ReturnNull, Span::zero());
        }

        assert!(matches!(
            LivenessQueries::for_chunk(&chunk, 2),
            LivenessQueries::Skipped
        ));
    }

    #[test]
    fn caches_a_dense_matrix_within_the_memory_budget() {
        let mut chunk = Chunk::new();
        chunk.register_count = 64;
        for _ in 0..512 {
            chunk.emit(Instruction::ReturnNull, Span::zero());
        }

        assert!(matches!(
            LivenessQueries::for_chunk(&chunk, 2),
            LivenessQueries::Cached(_)
        ));
    }
}
