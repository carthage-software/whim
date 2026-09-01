//! Straight-line propagation of stable register copies.

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::operands::replace_read_register;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.copy_propagation || chunk.code.len() < 2 {
        return;
    }
    if chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::CallNamedUnchecked { argument_count, .. }
                | Instruction::CallSelfUnchecked { argument_count, .. }
                if argument_count.value() > 1
        )
    }) {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for (index, removed) in remove.iter_mut().enumerate().take(chunk.code.len() - 1) {
        let Instruction::Move {
            destination,
            source,
        } = chunk.code[index]
        else {
            continue;
        };
        if destination == source {
            continue;
        }
        if destination.index() < chunk.local_register_count
            && (chunk.register_count > REFERENCE_REGISTER_LIMIT
                || chunk.reference_register_mask & (1u64 << destination.index()) != 0)
        {
            continue;
        }
        if targets.contains(&(index + 1)) {
            continue;
        }

        for cursor in index + 1..chunk.code.len() {
            if cursor != index + 1 && targets.contains(&cursor) {
                break;
            }

            let instruction = chunk.code[cursor];
            let replacement = replace_read_register(instruction, destination, source);
            match replacement {
                Some(replacement) => chunk.code[cursor] = replacement,
                None if effect_on(chunk, instruction, destination).reads() => {
                    break;
                }
                None => {}
            }

            let writes_destination = effect_on(chunk, instruction, destination).writes();
            let writes_source = effect_on(chunk, instruction, source).writes();
            if writes_destination || writes_source || branches_or_terminates(instruction) {
                break;
            }
        }

        if register_is_dead_after(chunk, destination, index + 1) {
            *removed = true;
        }
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
