//! Sinking of pure scalar producers into later temporary-consuming moves.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::liveness::register_is_untouched_between;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::rewrite::destination::with_destination;

pub(super) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.sink_move || chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];

    for move_index in 2..chunk.code.len() {
        let Instruction::Move {
            destination,
            source,
        } = chunk.code[move_index]
        else {
            continue;
        };

        if targets.contains(&move_index) || !register_is_dead_after(chunk, source, move_index + 1) {
            continue;
        }

        for producer in (0..move_index).rev() {
            if remove[producer]
                || targets.contains(&(producer + 1))
                || branches_or_terminates(chunk.code[producer + 1])
            {
                break;
            }

            if register_is_untouched_between(chunk, source, producer, producer + 1) {
                continue;
            }

            let Some((inputs, input_count)) = pure_scalar_inputs(chunk.code[producer]) else {
                break;
            };

            let Some(rewritten) = with_destination(chunk.code[producer], destination, source)
            else {
                break;
            };

            if chunk
                .catch_table
                .iter()
                .any(|entry| (entry.start as usize) < move_index && (entry.end as usize) > producer)
                || targets
                    .iter()
                    .any(|target| *target > producer && *target <= move_index)
                || inputs[..input_count].iter().any(|input| {
                    !register_is_untouched_between(chunk, *input, producer + 1, move_index)
                })
            {
                break;
            }

            chunk.code[move_index] = rewritten;
            remove[producer] = true;
            break;
        }
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn pure_scalar_inputs(instruction: Instruction) -> Option<([Register; 3], usize)> {
    let none = Register::NONE;
    match instruction {
        Instruction::FloatAdd { left, right, .. }
        | Instruction::FloatSubtract { left, right, .. }
        | Instruction::FloatMultiply { left, right, .. }
        | Instruction::IntAdd { left, right, .. }
        | Instruction::IntSubtract { left, right, .. }
        | Instruction::IntMultiply { left, right, .. }
        | Instruction::IntModulo { left, right, .. } => Some(([left, right, none], 2)),
        Instruction::FloatMultiplyConstant { source, .. }
        | Instruction::IntMultiplyImmediate { source, .. }
        | Instruction::IntModuloImmediate { source, .. } => Some(([source, none, none], 1)),
        Instruction::FloatDifferenceAdd {
            first_operand,
            addend,
            ..
        } => Some((
            [
                first_operand,
                Register::new(first_operand.index() + 1),
                addend,
            ],
            3,
        )),
        Instruction::FloatScaleProductAdd { first_operand, .. } => Some((
            [
                first_operand,
                Register::new(first_operand.index() + 1),
                Register::new(first_operand.index() + 2),
            ],
            3,
        )),
        _ => None,
    }
}
