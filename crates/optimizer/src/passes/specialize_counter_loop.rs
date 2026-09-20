//! Integer specialization of counted-loop control.

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
    if !configuration.specialize_counter_loop {
        return;
    }

    statistics.operations_specialized += plan_type_specializations(
        plan,
        analysis,
        CandidateSet::COUNTER_LOOP,
        specialized_instruction,
    );
}

pub(crate) fn optimize_chunk(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_counter_loop || chunk.code.is_empty() {
        return;
    }

    statistics.operations_specialized +=
        specialize_chunk_instructions(chunk, allocator, specialized_instruction);
}

fn specialized_instruction(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
) -> Option<Instruction> {
    specialize_with(instruction, |register| {
        flow.proves(index, register, &TypeDescriptor::Int)
    })
}

pub(super) fn specialize_with(
    instruction: Instruction,
    is_int: impl Fn(Register) -> bool,
) -> Option<Instruction> {
    let Instruction::CounterLoop {
        comparison,
        counter,
        limit,
        offset,
    } = instruction
    else {
        return None;
    };

    if !is_int(counter) || !is_int(limit) {
        return None;
    }

    Some(Instruction::IntCounterLoop {
        comparison,
        counter,
        limit,
        offset,
    })
}
