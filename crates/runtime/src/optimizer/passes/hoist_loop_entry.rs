//! Reuse of immutable values computed at the guarded entry of a counted loop.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::register_is_untouched_between;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
) {
    if !configuration.licm || chunk.code.len() < 5 || !chunk.catch_table.is_empty() {
        return;
    }

    for header in 0..chunk.code.len() - 4 {
        let exit = match chunk.code[header] {
            Instruction::NumericLoop { offset, .. }
            | Instruction::IntNumericLoop { offset, .. }
            | Instruction::PreparedIntNumericLoop { offset, .. } => {
                relative_target(header, i32::from(offset.offset()))
            }
            _ => continue,
        };
        if exit <= header + 2 || exit > chunk.code.len() {
            continue;
        }

        let entry = header + 1;
        let Instruction::VecIndexGet {
            destination,
            container,
            index,
            ..
        } = chunk.code[entry]
        else {
            continue;
        };
        let tail = exit - 1;
        let (Instruction::CounterLoop { offset, .. } | Instruction::IntCounterLoop { offset, .. }) =
            chunk.code[tail]
        else {
            continue;
        };
        if relative_target(tail, i32::from(offset.offset())) != entry
            || !register_is_untouched_between(chunk, container, entry + 1, exit)
            || !register_is_untouched_between(chunk, index, entry + 1, exit)
            || chunk.code[entry + 1..exit]
                .iter()
                .any(|instruction| effect_on(chunk, *instruction, destination).writes())
        {
            continue;
        }

        let next = ShortJumpOffset::new(offset.offset() + 1);
        chunk.code[tail] = match chunk.code[tail] {
            Instruction::CounterLoop {
                comparison,
                counter,
                limit,
                ..
            } => Instruction::CounterLoop {
                comparison,
                counter,
                limit,
                offset: next,
            },
            Instruction::IntCounterLoop {
                comparison,
                counter,
                limit,
                ..
            } => Instruction::IntCounterLoop {
                comparison,
                counter,
                limit,
                offset: next,
            },
            _ => continue,
        };
    }
}
