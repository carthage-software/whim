//! Removal of redundant and dead scalar register clears.

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::LivenessQueries;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, statistics);
    });
}

fn optimize_chunk(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    let clear_count = chunk
        .code
        .iter()
        .filter(|instruction| matches!(instruction, Instruction::Clear { .. }))
        .count();
    if clear_count == 0 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let liveness = LivenessQueries::for_chunk(chunk, clear_count);
    let mut remove = vec![false; chunk.code.len()];
    for (index, removed) in remove.iter_mut().enumerate() {
        let Instruction::Clear { target } = chunk.code[index] else {
            continue;
        };
        if chunk.trace_argument_registers.contains(&target) {
            continue;
        }
        if index != 0
            && !targets.contains(&index)
            && matches!(chunk.code[index - 1], Instruction::Clear { target: previous } if previous == target)
        {
            *removed = true;
            continue;
        }

        let register = target.index();
        if register >= REFERENCE_REGISTER_LIMIT
            || chunk.reference_register_mask & (1u64 << register) != 0
        {
            continue;
        }
        let dead = liveness.register_is_dead_after(chunk, target, index + 1);
        if dead {
            *removed = true;
        }
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
