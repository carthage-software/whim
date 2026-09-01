//! Fusion of a two-element tuple immediately returned by a callable.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Count;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_return_pair {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, statistics);
    });
}

pub(in crate::optimizer) fn optimize_owned_sources_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_return_pair {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_owned_sources(chunk, statistics);
    });
}

fn optimize_chunk(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Instruction::NewTuple {
            element_count,
            destination,
            first_element,
        } = chunk.code[index]
        else {
            continue;
        };
        let Instruction::ReturnReferenceUnchecked { source } = chunk.code[index + 1] else {
            continue;
        };
        if element_count != Count::new(2) || source != destination || targets.contains(&(index + 1))
        {
            continue;
        }

        chunk.code[index] = Instruction::ReturnPairUnchecked {
            first: first_element,
            second: Register::new(first_element.index() + 1),
        };
        remove[index + 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn optimize_owned_sources(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 2..chunk.code.len() {
        let Instruction::ReturnPairUnchecked { first, second } = chunk.code[index] else {
            continue;
        };
        let Instruction::MoveOwned {
            destination: first_destination,
            source: first_source,
        } = chunk.code[index - 2]
        else {
            continue;
        };
        let Instruction::MoveOwned {
            destination: second_destination,
            source: second_source,
        } = chunk.code[index - 1]
        else {
            continue;
        };
        if targets.contains(&(index - 2)) || targets.contains(&(index - 1)) {
            continue;
        }

        let sources = if first_destination == first && second_destination == second {
            Some((first_source, second_source))
        } else if first_destination == second && second_destination == first {
            Some((second_source, first_source))
        } else {
            None
        };
        let Some((first, second)) = sources else {
            continue;
        };

        chunk.code[index] = Instruction::ReturnPairUnchecked { first, second };
        remove[index - 2] = true;
        remove[index - 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
