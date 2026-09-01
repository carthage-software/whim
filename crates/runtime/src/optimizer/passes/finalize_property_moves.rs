//! Validation of ownership-moving property writes after register reuse.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::PropertyValueMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::register_is_unused_after;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
) {
    for_each_mutable_chunk(unit, configuration, optimize_chunk);
}

fn optimize_chunk(chunk: &mut Chunk) {
    for index in 0..chunk.code.len() {
        let Instruction::PropertySetUnchecked {
            object,
            value,
            slot,
            value_mode,
        } = chunk.code[index]
        else {
            continue;
        };

        let replacement = match value_mode {
            PropertyValueMode::MoveAndClear => PropertyValueMode::Move,
            PropertyValueMode::FreshMoveAndClear => PropertyValueMode::FreshMove,
            _ => continue,
        };

        if !register_is_unused_after(chunk, value, index + 1)
            && !exact_call_restores_reference_mask(chunk, value, index + 1)
        {
            chunk.code[index] = Instruction::PropertySetUnchecked {
                object,
                value,
                slot,
                value_mode: replacement,
            };
        }
    }
}

fn exact_call_restores_reference_mask(chunk: &Chunk, register: Register, start: usize) -> bool {
    if !chunk.catch_table.is_empty() {
        return false;
    }

    let targets = control_flow_targets(chunk);
    for index in start..chunk.code.len() {
        if targets.contains(&index) {
            return false;
        }

        let instruction = chunk.code[index];
        if matches!(
            instruction,
            Instruction::CallNamedUnchecked { destination, .. }
                | Instruction::CallSelfUnchecked { destination, .. }
                | Instruction::CallMethodUnchecked { destination, .. }
                | Instruction::CallMethodDirect { destination, .. }
                if destination == register
        ) {
            return true;
        }

        if scalar_write_to(instruction, register) {
            continue;
        }

        if !effect_on(chunk, instruction, register).is_none() {
            return false;
        }

        let mut edges = Vec::new();
        successors(chunk, index, &mut edges);
        if edges.as_slice() != [index + 1] {
            return false;
        }
    }

    false
}

fn scalar_write_to(instruction: Instruction, register: Register) -> bool {
    matches!(
        instruction,
        Instruction::LoadNull { destination }
            | Instruction::LoadTrue { destination }
            | Instruction::LoadFalse { destination }
            | Instruction::LoadInt { destination, .. }
            | Instruction::Negate { destination, .. }
            | Instruction::UnaryPlus { destination, .. }
            | Instruction::AddImmediate { destination, .. }
            | Instruction::SubtractImmediate { destination, .. }
            | Instruction::BitwiseNot { destination, .. }
            | Instruction::Not { destination, .. }
            | Instruction::Length { destination, .. }
            | Instruction::StringLength { destination, .. }
            | Instruction::IntAdd { destination, .. }
            | Instruction::IntSubtract { destination, .. }
            | Instruction::IntMultiply { destination, .. }
            | Instruction::IntModulo { destination, .. }
            | Instruction::IntMultiplyImmediate { destination, .. }
            | Instruction::IntModuloImmediate { destination, .. }
            | Instruction::FloatAdd { destination, .. }
            | Instruction::FloatSubtract { destination, .. }
            | Instruction::FloatMultiply { destination, .. }
            | Instruction::FloatMultiplyConstant { destination, .. }
            if destination == register
    )
}
