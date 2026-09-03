//! Fusion of concatenation with adjacent string constants.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::dead_store::PreviousValueSafety;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_concatenation {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        fuse_constants(chunk, statistics);
    });
}

fn fuse_constants(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 2 || !has_candidate(chunk) {
        return;
    }

    let targets = control_flow_targets(chunk);
    let loop_members = loop_members(chunk);
    let previous_values = PreviousValueSafety::analyze(chunk, &targets, chunk.local_register_count);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        if loop_members[index] || targets.contains(&(index + 1)) {
            continue;
        }

        let Instruction::Concatenate {
            destination: target,
            left,
            right,
        } = chunk.code[index + 1]
        else {
            continue;
        };

        let Instruction::LoadConstant {
            destination,
            constant,
        } = chunk.code[index]
        else {
            continue;
        };
        if destination != right
            || left == right
            || !matches!(
                chunk.constants[usize::from(constant.index())],
                Literal::String(_)
            )
            || !previous_values.cannot_own_reference(index, right)
            || (target != right && !register_is_dead_after(chunk, right, index + 2))
        {
            continue;
        }

        chunk.code[index + 1] = Instruction::ConcatenateConstant {
            destination: target,
            source: left,
            constant,
        };
        remove[index] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn has_candidate(chunk: &Chunk) -> bool {
    chunk.code.windows(2).any(|instructions| {
        let Instruction::LoadConstant {
            destination,
            constant,
        } = instructions[0]
        else {
            return false;
        };
        matches!(
            instructions[1],
            Instruction::Concatenate { left, right, .. }
                if destination == right
                    && left != right
                    && matches!(
                        chunk.constants[usize::from(constant.index())],
                        Literal::String(_)
                    )
        )
    })
}

fn loop_members(chunk: &Chunk) -> Vec<bool> {
    let mut members = vec![false; chunk.code.len()];
    let mut edges = Vec::new();
    for source in 0..chunk.code.len() {
        edges.clear();
        successors(chunk, source, &mut edges);
        for &target in &edges {
            if target < source {
                members[target..=source].fill(true);
            }
        }
    }

    members
}
