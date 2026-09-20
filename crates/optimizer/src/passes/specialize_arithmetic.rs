//! Specialization of arithmetic whose operand types are proven.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::Register;
use whim_value::heap::Heap;

use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;
use crate::analysis::Analysis;
use crate::candidates::CandidateSet;
use crate::passes::plan_type_specializations;
use crate::passes::specialize_chunk_instructions;
use crate::rewrite::plan::RewritePlan;
use crate::type_flow::TypeFlow;

pub(crate) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_arithmetic {
        return;
    }

    statistics.operations_specialized += plan_type_specializations(
        plan,
        analysis,
        CandidateSet::ARITHMETIC,
        specialized_instruction,
    );
}

pub(crate) fn optimize_chunk(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_arithmetic || chunk.code.is_empty() {
        return;
    }

    statistics.operations_specialized +=
        specialize_chunk_instructions(chunk, allocator, specialized_instruction);
}

pub(crate) fn specialized_instruction(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
) -> Option<Instruction> {
    specialize_with(
        instruction,
        |register| flow.proves(index, register, &TypeDescriptor::Int),
        |register| flow.proves(index, register, &TypeDescriptor::Float),
    )
}

pub(super) fn specialize_with(
    instruction: Instruction,
    is_int: impl Fn(Register) -> bool,
    is_float: impl Fn(Register) -> bool,
) -> Option<Instruction> {
    if let Instruction::BitwiseNot {
        destination,
        source,
    } = instruction
    {
        return is_int(source).then_some(Instruction::IntBitwiseNot {
            destination,
            source,
        });
    }

    let (left, right, integer, float) = match instruction {
        Instruction::Add {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(if destination == left {
                Instruction::IntAddAssign {
                    target: destination,
                    source: right,
                }
            } else if destination == right {
                Instruction::IntAddAssign {
                    target: destination,
                    source: left,
                }
            } else {
                Instruction::IntAdd {
                    destination,
                    left,
                    right,
                }
            }),
            Some(Instruction::FloatAdd {
                destination,
                left,
                right,
            }),
        ),
        Instruction::Subtract {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntSubtract {
                destination,
                left,
                right,
            }),
            Some(Instruction::FloatSubtract {
                destination,
                left,
                right,
            }),
        ),
        Instruction::Multiply {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntMultiply {
                destination,
                left,
                right,
            }),
            Some(Instruction::FloatMultiply {
                destination,
                left,
                right,
            }),
        ),
        Instruction::Modulo {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntModulo {
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::BitwiseAnd {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntBitwiseAnd {
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::BitwiseOr {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntBitwiseOr {
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::BitwiseXor {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntBitwiseXor {
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::ShiftLeft {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntShiftLeft {
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::ShiftRight {
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::IntShiftRight {
                destination,
                left,
                right,
            }),
            None,
        ),
        _ => return None,
    };
    if is_int(left) && is_int(right) {
        integer
    } else if is_float(left) && is_float(right) {
        float
    } else {
        None
    }
}
