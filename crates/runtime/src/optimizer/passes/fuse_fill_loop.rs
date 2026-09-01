//! Fusion of ascending integer loops that fill one indexed property.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Comparison as BytecodeComparison;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;

pub(super) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_fill_loop || chunk.code.len() < 9 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];

    for header in 2..chunk.code.len() - 5 {
        let Instruction::JumpUnless {
            comparison: BytecodeComparison::LessThanOrEqual,
            left: counter,
            right: limit,
            offset,
        } = chunk.code[header]
        else {
            continue;
        };

        let exit = relative_target(header, i32::from(offset.offset()));
        if exit != header + 6 || exit > chunk.code.len() {
            continue;
        }

        let body = header + 1;
        let tail = exit - 1;
        let Instruction::PropertyGet {
            destination: container,
            object,
            cache,
        } = chunk.code[body]
        else {
            continue;
        };

        let value_load = chunk.code[body + 1];
        let Some(value) = literal_destination(value_load) else {
            continue;
        };

        let Instruction::IndexSet {
            container: indexed_container,
            index,
            value: indexed_value,
        } = chunk.code[body + 2]
        else {
            continue;
        };

        let Instruction::PropertySet {
            object: written_object,
            value: written_container,
            cache: written_cache,
        } = chunk.code[body + 3]
        else {
            continue;
        };

        let Instruction::CounterLoop {
            comparison: BytecodeComparison::LessThanOrEqual,
            counter: loop_counter,
            limit: loop_limit,
            offset: back_edge,
        } = chunk.code[tail]
        else {
            continue;
        };

        let Instruction::LoadInt {
            destination: initialized_counter,
            immediate: initial,
        } = chunk.code[header - 2]
        else {
            continue;
        };

        let Instruction::LoadInt {
            destination: loaded_limit,
            immediate: maximum,
        } = chunk.code[header - 1]
        else {
            continue;
        };

        if initialized_counter != counter
            || initial.value() != 0
            || loaded_limit != limit
            || maximum.value() < 0
            || indexed_container != container
            || index != counter
            || indexed_value != value
            || written_object != object
            || written_container != container
            || written_cache != cache
            || loop_counter != counter
            || loop_limit != limit
            || relative_target(tail, i32::from(back_edge.offset())) != body
            || value.index().checked_add(1) != Some(limit.index())
            || [
                header - 2,
                header - 1,
                header,
                body + 1,
                body + 2,
                body + 3,
                tail,
            ]
            .into_iter()
            .any(|index| targets.contains(&index))
            || !register_is_dead_after(chunk, counter, exit)
            || !register_is_dead_after(chunk, container, exit)
            || !register_is_dead_after(chunk, value, exit)
        {
            continue;
        }

        chunk.code[header - 2] = value_load;
        chunk.code[body] = Instruction::PropertyFillIntRange {
            object,
            first_operand: value,
            cache,
        };

        remove[header] = true;
        remove[body + 1..=tail].fill(true);
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn literal_destination(instruction: Instruction) -> Option<Register> {
    match instruction {
        Instruction::LoadConstant { destination, .. }
        | Instruction::LoadNull { destination }
        | Instruction::LoadTrue { destination }
        | Instruction::LoadFalse { destination }
        | Instruction::LoadInt { destination, .. } => Some(destination),
        _ => None,
    }
}
