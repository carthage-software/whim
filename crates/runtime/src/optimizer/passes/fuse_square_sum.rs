//! Fusion of adjacent float squares and their immediate sum.

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::rewrite::compact;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::passes::compact_removed_instructions;
use crate::unwrap_result_invariant;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_square_sum || chunk.code.len() < 2 {
        return;
    }

    if chunk.catch_table.is_empty() {
        rotate_loop_squares(chunk, statistics);
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Instruction::FloatSquares {
            first_destination: square,
            first_source,
            second_source,
        } = chunk.code[index]
        else {
            continue;
        };

        let Instruction::FloatAdd {
            destination: sum,
            left,
            right,
        } = chunk.code[index + 1]
        else {
            continue;
        };

        if left != square
            || right.index().checked_sub(1) != Some(square.index())
            || sum.index().checked_add(1) != Some(square.index())
            || targets.contains(&(index + 1))
        {
            continue;
        }

        chunk.code[index] = Instruction::FloatSquaresSum {
            first_destination: sum,
            first_source,
            second_source,
        };

        remove[index + 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn rotate_loop_squares(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    loop {
        let targets = control_flow_targets(chunk);
        let mut rotation = None;
        for producer in 0..chunk.code.len().saturating_sub(1) {
            let Instruction::FloatSquares {
                first_destination: square,
                first_source,
                second_source,
            } = chunk.code[producer]
            else {
                continue;
            };
            let consumer = producer + 1;
            let Instruction::FloatAdd {
                destination: _,
                left,
                right,
            } = chunk.code[consumer]
            else {
                continue;
            };

            if left != square
                || right.index().checked_sub(1) != Some(square.index())
                || !targets.contains(&consumer)
            {
                continue;
            }

            let Some(back_edge) = sole_back_edge_to(chunk, producer, consumer) else {
                continue;
            };
            let Some(tail_producer) = matching_tail_producer(
                chunk,
                &targets,
                consumer,
                back_edge,
                square,
                first_source,
                second_source,
            ) else {
                continue;
            };

            rotation = Some((producer, tail_producer, back_edge));
            break;
        }

        let Some((producer, tail_producer, back_edge)) = rotation else {
            return;
        };

        chunk.code[back_edge] = Instruction::Jump {
            // SAFETY: the surrounding invariant proves this result is successful.
            offset: JumpOffset::new(unsafe {
                unwrap_result_invariant(
                    i32::try_from(producer as i64 - back_edge as i64),
                    "a bytecode back edge must fit its jump operand",
                )
            }),
        };

        let mut remove = vec![false; chunk.code.len()];
        remove[tail_producer] = true;
        compact(chunk, &remove);
        statistics.instructions_removed += 1;
    }
}

fn sole_back_edge_to(chunk: &Chunk, producer: usize, consumer: usize) -> Option<usize> {
    if chunk.catch_table.iter().any(|entry| {
        entry.start as usize == consumer
            || entry.end as usize == consumer
            || entry.handler as usize == consumer
    }) {
        return None;
    }

    let mut edges = Vec::new();
    let mut back_edge = None;
    for source in 0..chunk.code.len() {
        edges.clear();
        successors(chunk, source, &mut edges);
        if !edges.contains(&consumer) || source == producer {
            continue;
        }

        let Instruction::Jump { offset } = chunk.code[source] else {
            return None;
        };
        if source <= consumer || relative_target(source, offset.offset()) != consumer {
            return None;
        }
        if back_edge.replace(source).is_some() {
            return None;
        }
    }

    back_edge
}

fn matching_tail_producer(
    chunk: &Chunk,
    targets: &HashSet<usize>,
    consumer: usize,
    back_edge: usize,
    square: Register,
    first_source: Register,
    second_source: Register,
) -> Option<usize> {
    let second_square = Register::new(square.index() + 1);
    for index in (consumer + 1..back_edge).rev() {
        let instruction = chunk.code[index];
        if targets.contains(&index) || branches_or_terminates(instruction) {
            return None;
        }
        if matches!(
            instruction,
            Instruction::FloatSquares {
                first_destination,
                first_source: tail_first,
                second_source: tail_second,
            } if first_destination == square
                && tail_first == first_source
                && tail_second == second_source
        ) {
            return Some(index);
        }
        if !effect_on(chunk, instruction, square).is_none()
            || !effect_on(chunk, instruction, second_square).is_none()
            || effect_on(chunk, instruction, first_source).writes()
            || effect_on(chunk, instruction, second_source).writes()
        {
            return None;
        }
    }

    None
}
