//! Fusion of concatenation with string constants.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::operands::for_each_register;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::dead_store::PreviousValueSafety;
use crate::optimizer::passes::for_each_mutable_chunk;

#[derive(Clone, Copy)]
struct AvailableConstant {
    load: usize,
    constant: ConstantIndex,
}

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
    let loops = loop_members(chunk);
    let protected = protected_members(chunk);
    let previous_values = PreviousValueSafety::analyze(chunk, &targets, chunk.local_register_count);
    let mut available = vec![None; usize::from(chunk.register_count)];
    let mut remove = vec![false; chunk.code.len()];

    for index in 0..chunk.code.len() {
        if targets.contains(&index) || loops[index] || protected[index] {
            available.fill(None);
        }

        if let Instruction::Concatenate {
            destination,
            left,
            right,
        } = chunk.code[index]
        {
            if let Some(load) = available[usize::from(right.index())].filter(|load| {
                fusable_constant(
                    chunk,
                    &previous_values,
                    *load,
                    index,
                    destination,
                    right,
                    left,
                )
            }) {
                chunk.code[index] = Instruction::ConcatenateRightConstant {
                    destination,
                    source: left,
                    constant: load.constant,
                };
                available[usize::from(right.index())] = None;
                remove[load.load] = true;
            } else if let Some(load) = available[usize::from(left.index())].filter(|load| {
                fusable_constant(
                    chunk,
                    &previous_values,
                    *load,
                    index,
                    destination,
                    left,
                    right,
                )
            }) {
                chunk.code[index] = Instruction::ConcatenateLeftConstant {
                    destination,
                    source: right,
                    constant: load.constant,
                };
                available[usize::from(left.index())] = None;
                remove[load.load] = true;
            }
        }

        update_available(chunk, index, &mut available);
        if branches_or_terminates(chunk.code[index]) {
            available.fill(None);
        }
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn fusable_constant(
    chunk: &Chunk,
    previous_values: &PreviousValueSafety,
    load: AvailableConstant,
    use_index: usize,
    destination: Register,
    operand: Register,
    other: Register,
) -> bool {
    operand != other
        && previous_values.cannot_own_reference(load.load, operand)
        && (destination == operand || register_is_dead_after(chunk, operand, use_index + 1))
}

fn update_available(chunk: &Chunk, index: usize, available: &mut [Option<AvailableConstant>]) {
    let instruction = chunk.code[index];
    if !for_each_register(instruction, |register| {
        available[usize::from(register.index())] = None;
    }) {
        for register in 0..chunk.register_count {
            let candidate = &mut available[usize::from(register)];
            if candidate.is_some()
                && !effect_on(chunk, instruction, Register::new(register)).is_none()
            {
                *candidate = None;
            }
        }
    }

    let Instruction::LoadConstant {
        destination,
        constant,
    } = instruction
    else {
        return;
    };
    if matches!(
        chunk.constants[usize::from(constant.index())],
        Literal::String(_)
    ) {
        available[usize::from(destination.index())] = Some(AvailableConstant {
            load: index,
            constant,
        });
    }
}

fn has_candidate(chunk: &Chunk) -> bool {
    let has_concatenation = chunk
        .code
        .iter()
        .any(|instruction| matches!(instruction, Instruction::Concatenate { .. }));
    has_concatenation
        && chunk.code.iter().any(|instruction| {
            let Instruction::LoadConstant { constant, .. } = instruction else {
                return false;
            };
            matches!(
                chunk.constants[usize::from(constant.index())],
                Literal::String(_)
            )
        })
}

fn protected_members(chunk: &Chunk) -> Vec<bool> {
    let mut members = vec![false; chunk.code.len()];
    for entry in &chunk.catch_table {
        members[entry.start as usize..entry.end as usize].fill(true);
    }
    members
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
