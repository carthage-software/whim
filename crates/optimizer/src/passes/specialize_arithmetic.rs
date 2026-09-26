//! Specialization of arithmetic whose operand types are proven.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::IntegerKind;
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
        |register| flow.proves(index, register, &TypeDescriptor::Uint),
        |register| flow.proves(index, register, &TypeDescriptor::Float),
    )
}

pub(super) fn specialize_with(
    instruction: Instruction,
    is_int: impl Fn(Register) -> bool,
    is_uint: impl Fn(Register) -> bool,
    is_float: impl Fn(Register) -> bool,
) -> Option<Instruction> {
    if let Instruction::BitwiseNot {
        destination,
        source,
    } = instruction
    {
        return if is_int(source) {
            Some(Instruction::IntegerBitwiseNot {
                kind: IntegerKind::I64,
                destination,
                source,
            })
        } else if is_uint(source) {
            Some(Instruction::IntegerBitwiseNot {
                kind: IntegerKind::U64,
                destination,
                source,
            })
        } else {
            None
        };
    }

    if let Instruction::Step {
        destination,
        source,
        immediate,
    } = instruction
    {
        return is_uint(source).then_some(Instruction::IntegerStep {
            kind: IntegerKind::U64,
            destination,
            source,
            immediate,
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
                Instruction::IntegerAddAssign {
                    kind: IntegerKind::I64,
                    target: destination,
                    source: right,
                }
            } else if destination == right {
                Instruction::IntegerAddAssign {
                    kind: IntegerKind::I64,
                    target: destination,
                    source: left,
                }
            } else {
                Instruction::IntegerAdd {
                    kind: IntegerKind::I64,
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
            Some(Instruction::IntegerSubtract {
                kind: IntegerKind::I64,
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
            Some(Instruction::IntegerMultiply {
                kind: IntegerKind::I64,
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
            Some(Instruction::IntegerModulo {
                kind: IntegerKind::I64,
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
            Some(Instruction::IntegerBitwiseAnd {
                kind: IntegerKind::I64,
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
            Some(Instruction::IntegerBitwiseOr {
                kind: IntegerKind::I64,
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
            Some(Instruction::IntegerBitwiseXor {
                kind: IntegerKind::I64,
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
            Some(Instruction::IntegerShiftLeft {
                kind: IntegerKind::I64,
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
            Some(Instruction::IntegerShiftRight {
                kind: IntegerKind::I64,
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
    } else if is_uint(left)
        && (is_uint(right)
            || (is_int(right)
                && matches!(
                    instruction,
                    Instruction::ShiftLeft { .. } | Instruction::ShiftRight { .. }
                )))
    {
        unsigned_instruction(integer?)
    } else if is_float(left) && is_float(right) {
        float
    } else {
        None
    }
}

fn unsigned_instruction(mut instruction: Instruction) -> Option<Instruction> {
    let kind = match &mut instruction {
        Instruction::IntegerAdd { kind, .. }
        | Instruction::IntegerAddAssign { kind, .. }
        | Instruction::IntegerSubtract { kind, .. }
        | Instruction::IntegerMultiply { kind, .. }
        | Instruction::IntegerModulo { kind, .. }
        | Instruction::IntegerBitwiseAnd { kind, .. }
        | Instruction::IntegerBitwiseOr { kind, .. }
        | Instruction::IntegerBitwiseXor { kind, .. }
        | Instruction::IntegerShiftLeft { kind, .. }
        | Instruction::IntegerShiftRight { kind, .. } => kind,
        _ => return None,
    };
    *kind = IntegerKind::U64;
    Some(instruction)
}
