//! Reuse of compiler temporary registers with non-overlapping lifetimes.

use std::cmp::Reverse;

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::operands::Access;
use crate::optimizer::operands::implicit_reads;
use crate::optimizer::operands::instruction_bytes;
use crate::optimizer::operands::operands;
use crate::optimizer::operands::register_at;
use crate::optimizer::operands::remap;
use crate::optimizer::operands::write_may_alias_inputs;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::optimizer::passes::reuse_temporaries::coloring::coalesce_single_argument_call_results;
use crate::optimizer::passes::reuse_temporaries::coloring::recolor_interference_graph;

mod coloring;

#[derive(Clone, Copy)]
struct Interval {
    register: u16,
    start: usize,
    end: usize,
    starts_with_write: bool,
    may_alias_inputs: bool,
}

#[derive(Clone, Copy)]
struct Active {
    end: usize,
    physical: u16,
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk_inner(chunk, configuration, statistics);
        coalesce_single_argument_call_results(chunk);
        recolor_interference_graph(chunk, statistics);
    });
}

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk_inner(chunk, configuration, statistics);
}

fn optimize_chunk_inner(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.reuse_temporaries
        || chunk.register_count <= chunk.local_register_count
        || !chunk.catch_table.is_empty()
    {
        return;
    }

    let register_count = usize::from(chunk.register_count);
    let local_count = usize::from(chunk.local_register_count);
    let pinned = pinned_window_registers(chunk);
    if pinned_high_water(&pinned, chunk.local_register_count) >= chunk.register_count {
        return;
    }
    let mut first = vec![usize::MAX; register_count];
    let mut last = vec![0; register_count];
    let mut starts_with_write = vec![false; register_count];
    let mut may_alias_inputs = vec![false; register_count];

    for (index, instruction) in chunk.code.iter().copied().enumerate() {
        let Some(operands) = operands(instruction.kind()) else {
            return;
        };

        let bytes = instruction_bytes(instruction);
        for operand in operands {
            if operand.offset == 4
                && matches!(
                    instruction,
                    Instruction::CallNamedUnchecked { argument_count, .. }
                        | Instruction::CallSelfUnchecked { argument_count, .. }
                        if argument_count.value() == 0
                )
            {
                continue;
            }
            let register = register_at(bytes, operand.offset);
            if register == Register::NONE {
                continue;
            }

            let register = usize::from(register.index());
            if first[register] == usize::MAX {
                first[register] = index;
                starts_with_write[register] = operand.access == Access::Write;
                may_alias_inputs[register] =
                    operand.access == Access::Write && write_may_alias_inputs(instruction.kind());
            }

            last[register] = index;
        }

        if let Some((implicit_first, count)) = implicit_reads(instruction) {
            for register in implicit_first.index()..implicit_first.index() + count as u16 {
                let register = usize::from(register);
                if first[register] == usize::MAX {
                    first[register] = index;
                }

                last[register] = index;
            }
        }
    }

    let mut edges = Vec::new();
    for source in 0..chunk.code.len() {
        edges.clear();
        successors(chunk, source, &mut edges);
        for &target in &edges {
            if target > source {
                continue;
            }

            for register in local_count..register_count {
                if first[register] < target && last[register] >= target {
                    last[register] = last[register].max(source);
                }
            }
        }
    }

    let mut intervals = Vec::new();
    for register in local_count..register_count {
        if first[register] == usize::MAX {
            continue;
        }

        if !starts_with_write[register] {
            return;
        }

        intervals.push(Interval {
            register: register as u16,
            start: first[register],
            end: last[register],
            starts_with_write: starts_with_write[register],
            may_alias_inputs: may_alias_inputs[register],
        });
    }

    intervals.sort_unstable_by_key(|interval| (interval.start, interval.starts_with_write));

    let mut mapping = vec![Register::NONE; register_count];
    for (register, mapped) in mapping.iter_mut().enumerate().take(local_count) {
        *mapped = Register::new(register as u16);
    }

    for register in &pinned {
        mapping[usize::from(*register)] = Register::new(*register);
    }

    let mut active: Vec<Active> = Vec::new();
    let mut free: Vec<u16> = Vec::new();
    let mut next = pinned
        .iter()
        .max()
        .map_or(chunk.local_register_count, |register| register + 1)
        .max(chunk.local_register_count);
    let mut high_water = next;

    for interval in intervals {
        if pinned.contains(&interval.register) {
            continue;
        }

        let mut index = 0;
        while index < active.len() {
            let reusable = if interval.may_alias_inputs {
                active[index].end <= interval.start
            } else {
                active[index].end < interval.start
            };
            if reusable {
                free.push(active.swap_remove(index).physical);
            } else {
                index += 1;
            }
        }

        let physical = if let Some((position, _)) = free
            .iter()
            .enumerate()
            .min_by_key(|(_, register)| **register)
        {
            free.swap_remove(position)
        } else {
            let physical = next;
            next += 1;
            high_water = high_water.max(next);
            physical
        };

        mapping[usize::from(interval.register)] = Register::new(physical);
        active.push(Active {
            end: interval.end,
            physical,
        });
    }

    if high_water < chunk.register_count {
        for instruction in &mut chunk.code {
            *instruction = remap(*instruction, &mapping);
        }

        statistics.registers_removed += usize::from(chunk.register_count - high_water);
        chunk.register_count = high_water;
        normalize_empty_window_starts(chunk);
    }
}

/// Keeps encoded register windows consecutive while remapping temporaries.
fn pinned_window_registers(chunk: &Chunk) -> HashSet<u16> {
    let mut registers = HashSet::new();
    for instruction in &chunk.code {
        let window = match *instruction {
            Instruction::Assert {
                operand_count,
                first_value,
                ..
            } => Some((first_value, usize::from(operand_count.value()) + 1)),
            Instruction::CallNamed {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallNamedDiscarded {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethod {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethodDiscarded {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethodUnchecked {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethodDirect {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallStatic {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallStaticDiscarded {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallValue {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallValueDiscarded {
                argument_count,
                first_argument,
                ..
            } => Some((first_argument, usize::from(argument_count.value()))),
            Instruction::CallNamedUnchecked {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallSelfUnchecked {
                argument_count,
                first_argument,
                ..
            } if argument_count.value() != 1 => {
                Some((first_argument, usize::from(argument_count.value())))
            }
            Instruction::NewVec {
                element_count,
                first_element,
                ..
            }
            | Instruction::NewTuple {
                element_count,
                first_element,
                ..
            } => Some((first_element, usize::from(element_count.value()))),
            Instruction::NewDict {
                pair_count,
                first_pair,
                ..
            } => Some((first_pair, usize::from(pair_count.value()) * 2)),
            Instruction::MakeClosure {
                capture_count,
                first_capture,
                ..
            } => Some((first_capture, usize::from(capture_count.value()))),
            Instruction::Write {
                value_count,
                first_value,
            }
            | Instruction::WriteLine {
                value_count,
                first_value,
            }
            | Instruction::WriteError {
                value_count,
                first_value,
            }
            | Instruction::WriteErrorLine {
                value_count,
                first_value,
            }
            | Instruction::Debug {
                value_count,
                first_value,
            } => Some((first_value, usize::from(value_count.value()))),
            Instruction::PropertyIndexSet { first_operand, .. }
            | Instruction::PropertyIndexSetUnchecked { first_operand, .. } => {
                Some((first_operand, 2))
            }
            Instruction::PropertyRemove {
                destination, mode, ..
            }
            | Instruction::PropertyRemoveUnchecked {
                destination, mode, ..
            } if mode.uses_operand() => Some((destination, 2)),
            _ => None,
        };
        let Some((first, count)) = window else {
            continue;
        };
        for register in first.index()..first.index() + count as u16 {
            registers.insert(register);
        }
    }
    registers
}

fn pinned_high_water(pinned: &HashSet<u16>, local_count: u16) -> u16 {
    pinned
        .iter()
        .max()
        .map_or(local_count, |register| register + 1)
        .max(local_count)
}

fn normalize_empty_window_starts(chunk: &mut Chunk) {
    let placeholder = Register::new(chunk.register_count.saturating_sub(1));
    for instruction in &mut chunk.code {
        match instruction {
            Instruction::NewVec {
                element_count,
                first_element,
                ..
            }
            | Instruction::NewTuple {
                element_count,
                first_element,
                ..
            } if element_count.value() == 0 => *first_element = placeholder,
            Instruction::NewDict {
                pair_count,
                first_pair,
                ..
            } if pair_count.value() == 0 => *first_pair = placeholder,
            Instruction::MakeClosure {
                capture_count,
                first_capture,
                ..
            } if capture_count.value() == 0 => *first_capture = placeholder,
            Instruction::CallNamed {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallNamedDiscarded {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethod {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethodDiscarded {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethodUnchecked {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallMethodDirect {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallStatic {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallStaticDiscarded {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallNamedUnchecked {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallSelfUnchecked {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallValue {
                argument_count,
                first_argument,
                ..
            }
            | Instruction::CallValueDiscarded {
                argument_count,
                first_argument,
                ..
            } if argument_count.value() == 0 => *first_argument = placeholder,
            Instruction::Write {
                value_count,
                first_value,
            }
            | Instruction::WriteLine {
                value_count,
                first_value,
            }
            | Instruction::WriteError {
                value_count,
                first_value,
            }
            | Instruction::WriteErrorLine {
                value_count,
                first_value,
            }
            | Instruction::Debug {
                value_count,
                first_value,
            } if value_count.value() == 0 => *first_value = placeholder,
            _ => {}
        }
    }
}
