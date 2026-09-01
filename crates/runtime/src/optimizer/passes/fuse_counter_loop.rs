//! Fusion of the increment, comparison, and back edge of counted loops.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IntStepLoopDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_counter_loop || chunk.code.len() < 3 {
        return;
    }

    for header in 0..chunk.code.len() - 2 {
        let Instruction::JumpUnless {
            comparison,
            left: counter,
            right: limit,
            offset,
        } = chunk.code[header]
        else {
            continue;
        };

        let exit = relative_target(header, i32::from(offset.offset()));
        if exit <= header + 1 || exit > chunk.code.len() {
            continue;
        }

        let tail = exit - 1;
        let Instruction::IncrementJump {
            target,
            immediate,
            offset: back_edge,
        } = chunk.code[tail]
        else {
            continue;
        };

        if target != counter
            || immediate.value() != 1
            || relative_target(tail, i32::from(back_edge.offset())) != header
        {
            continue;
        }

        let Ok(body_offset) = i16::try_from(header as i64 + 1 - tail as i64) else {
            continue;
        };

        chunk.code[tail] = Instruction::CounterLoop {
            comparison,
            counter,
            limit,
            offset: ShortJumpOffset::new(body_offset),
        };
    }

    fuse_integer_step_loops(chunk, statistics);
}

fn fuse_integer_step_loops(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for header in 0..chunk.code.len() - 2 {
        let Instruction::IntJumpUnless {
            comparison,
            left: counter,
            right: limit,
            offset,
        } = chunk.code[header]
        else {
            continue;
        };

        let exit = relative_target(header, i32::from(offset.offset()));
        if exit <= header + 2 || exit > chunk.code.len() {
            continue;
        }

        let jump = exit - 1;
        let update = jump - 1;
        let Instruction::IntAddAssign {
            target,
            source: step,
        } = chunk.code[update]
        else {
            continue;
        };
        let Instruction::Jump { offset: back_edge } = chunk.code[jump] else {
            continue;
        };

        if target != counter
            || targets.contains(&jump)
            || relative_target(jump, back_edge.offset()) != header
        {
            continue;
        }

        let Ok(body_offset) = i16::try_from(header as i64 + 1 - update as i64) else {
            continue;
        };
        let Ok(descriptor) = chunk.add_int_step_loop_descriptor(IntStepLoopDescriptor {
            comparison,
            counter,
            limit,
            step,
        }) else {
            continue;
        };

        chunk.code[update] = Instruction::IntStepLoop {
            descriptor,
            offset: ShortJumpOffset::new(body_offset),
        };
        remove[jump] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
