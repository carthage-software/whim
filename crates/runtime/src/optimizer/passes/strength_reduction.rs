//! Reduction of proven integer arithmetic to smaller immediate operations.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::ConstantValue;
use crate::optimizer::type_flow::TypeFlow;
use crate::value::heap::Heap;

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.strength_reduction {
        return;
    }

    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::ARITHMETIC) {
            continue;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) {
                continue;
            }

            let Some(replacement) = reduced_instruction(&analyzed.flow, index, instruction) else {
                continue;
            };

            if analyzed.write(plan, index, replacement) {
                statistics.operations_specialized += 1;
            }
        }
    }
}

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    heap: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.strength_reduction || chunk.code.is_empty() {
        return;
    }

    let mut replacements = vec![None; chunk.code.len()];
    let flow = TypeFlow::analyze(chunk, &[], false, None, &[], heap);
    for (index, instruction) in chunk.code.iter().copied().enumerate() {
        replacements[index] = reduced_instruction(&flow, index, instruction);
    }

    for (instruction, replacement) in chunk.code.iter_mut().zip(replacements) {
        if let Some(replacement) = replacement {
            *instruction = replacement;
            statistics.operations_specialized += 1;
        }
    }
}

fn reduced_instruction(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
) -> Option<Instruction> {
    match instruction {
        Instruction::AddImmediate {
            destination,
            source,
            immediate,
        }
        | Instruction::SubtractImmediate {
            destination,
            source,
            immediate,
        } if immediate.value() == 0 && flow.proves(index, source, &TypeDescriptor::Int) => {
            Some(Instruction::Move {
                destination,
                source,
            })
        }
        Instruction::IntMultiplyImmediate {
            destination,
            source,
            immediate,
        } if immediate.value() == 1 => Some(Instruction::Move {
            destination,
            source,
        }),
        Instruction::IntAdd {
            destination,
            left,
            right,
        } => immediate(flow, index, right)
            .map(|immediate| Instruction::AddImmediate {
                destination,
                source: left,
                immediate,
            })
            .or_else(|| {
                immediate(flow, index, left).map(|immediate| Instruction::AddImmediate {
                    destination,
                    source: right,
                    immediate,
                })
            }),
        Instruction::IntSubtract {
            destination,
            left,
            right,
        } => immediate(flow, index, right).map(|immediate| Instruction::SubtractImmediate {
            destination,
            source: left,
            immediate,
        }),
        _ => None,
    }
}

fn immediate(flow: &TypeFlow<'_>, index: usize, register: Register) -> Option<ImmediateInt> {
    let ConstantValue::Int(value) = flow.constant_value(index, register)? else {
        return None;
    };

    Some(ImmediateInt::new(i16::try_from(value).ok()?))
}
