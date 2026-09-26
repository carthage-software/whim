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
        kind: None,
        destination,
        source,
    } = instruction
    {
        return if is_int(source) {
            Some(Instruction::BitwiseNot {
                kind: Some(IntegerKind::I64),
                destination,
                source,
            })
        } else if is_uint(source) {
            Some(Instruction::BitwiseNot {
                kind: Some(IntegerKind::U64),
                destination,
                source,
            })
        } else {
            None
        };
    }

    if let Instruction::Step {
        kind: None,
        destination,
        source,
        immediate,
    } = instruction
    {
        return is_uint(source).then_some(Instruction::Step {
            kind: Some(IntegerKind::U64),
            destination,
            source,
            immediate,
        });
    }

    let (left, right, integer, float) = match instruction {
        Instruction::Add {
            kind: None,
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
                Instruction::Add {
                    kind: Some(IntegerKind::I64),
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
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::Subtract {
                kind: Some(IntegerKind::I64),
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
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::Multiply {
                kind: Some(IntegerKind::I64),
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
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::Modulo {
                kind: Some(IntegerKind::I64),
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::BitwiseAnd {
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::BitwiseAnd {
                kind: Some(IntegerKind::I64),
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::BitwiseOr {
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::BitwiseOr {
                kind: Some(IntegerKind::I64),
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::BitwiseXor {
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::BitwiseXor {
                kind: Some(IntegerKind::I64),
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::ShiftLeft {
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::ShiftLeft {
                kind: Some(IntegerKind::I64),
                destination,
                left,
                right,
            }),
            None,
        ),
        Instruction::ShiftRight {
            kind: None,
            destination,
            left,
            right,
        } => (
            left,
            right,
            Some(Instruction::ShiftRight {
                kind: Some(IntegerKind::I64),
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
                    Instruction::ShiftLeft { kind: None, .. }
                        | Instruction::ShiftRight { kind: None, .. }
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
    match &mut instruction {
        Instruction::Add { kind, .. }
        | Instruction::Subtract { kind, .. }
        | Instruction::Multiply { kind, .. }
        | Instruction::Modulo { kind, .. }
        | Instruction::BitwiseAnd { kind, .. }
        | Instruction::BitwiseOr { kind, .. }
        | Instruction::BitwiseXor { kind, .. }
        | Instruction::ShiftLeft { kind, .. }
        | Instruction::ShiftRight { kind, .. } => *kind = Some(IntegerKind::U64),
        Instruction::IntegerAddAssign { kind, .. } => *kind = IntegerKind::U64,
        _ => return None,
    };
    Some(instruction)
}
