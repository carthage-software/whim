//! Fusion of compiler-expanded scalar property updates.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_property_update || chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for start in 0..=chunk.code.len() - 3 {
        if remove[start] {
            continue;
        }

        let Instruction::PropertyGet {
            destination: previous,
            object,
            cache,
        } = chunk.code[start]
        else {
            continue;
        };

        let Instruction::PropertySet {
            object: written_object,
            value: written_value,
            cache: written_cache,
        } = chunk.code[start + 2]
        else {
            continue;
        };

        if written_object != object
            || written_cache != cache
            || targets.contains(&(start + 1))
            || targets.contains(&(start + 2))
        {
            continue;
        }

        let replacement = match chunk.code[start + 1] {
            Instruction::AddImmediate {
                destination,
                source,
                immediate,
            } if source == previous && destination == written_value => Instruction::PropertyStep {
                object,
                cache,
                immediate,
            },
            Instruction::Add {
                destination,
                left,
                right,
            } if left == previous && destination == written_value => Instruction::PropertyAdd {
                object,
                source: right,
                cache,
            },
            _ => continue,
        };

        if !register_is_dead_after(chunk, previous, start + 3)
            || !register_is_dead_after(chunk, written_value, start + 3)
        {
            continue;
        }

        chunk.code[start] = replacement;
        remove[start + 1..=start + 2].fill(true);
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
