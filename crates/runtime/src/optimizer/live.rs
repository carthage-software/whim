//! Refinement of a not-yet-executed live chunk tail after the symbol world grows.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::successors;
use crate::optimizer::passes::inline_leaf_calls::leaf::inline_live_tail;
use crate::optimizer::passes::optimize_isolated_live_tail;
use crate::optimizer::passes::specialize_arithmetic;
use crate::optimizer::passes::specialize_arrays;
use crate::optimizer::passes::specialize_comparison;
use crate::optimizer::type_flow::IndexedUnit;
use crate::optimizer::type_flow::TypeFlow;
use crate::optimizer::type_flow::World;
use crate::value::Value;
use crate::value::heap::Heap;

pub(crate) struct Refinement<'a> {
    pub chunk: &'a Chunk,
    pub unit: &'a CompiledUnit,
    pub registers: &'a [Value],
    pub functions: &'a [CompiledFunction],
    pub world: &'a World<'a>,
    pub heap: &'a Heap,
    pub floor: usize,
    pub register_cap: u16,
}

pub(crate) fn refine(refinement: Refinement<'_>) -> Option<Chunk> {
    let Refinement {
        chunk,
        unit,
        registers,
        functions,
        world,
        heap,
        floor,
        register_cap,
    } = refinement;

    if registers.len() < usize::from(chunk.register_count) || !tail_is_isolated(chunk, floor) {
        return None;
    }

    let mut tail = chunk.clone_tail(floor);
    let indexed = IndexedUnit::with_world(unit, world);
    let replacements = {
        let flow = TypeFlow::analyze_live_with_unit(&tail, registers, &indexed, heap);
        tail.code
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, instruction)| {
                let replacement = match instruction {
                    Instruction::CallNamed {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    } if flow.function_arguments_proven(
                        index,
                        usize::from(first_argument.index()),
                        usize::from(argument_count.value()),
                    ) =>
                    {
                        Some(Instruction::CallNamedUnchecked {
                            argument_count,
                            destination,
                            first_argument,
                            cache,
                        })
                    }
                    _ => specialize_arrays::specialized_instruction(&flow, index, instruction)
                        .or_else(|| {
                            specialize_arithmetic::specialized_instruction(
                                &flow,
                                index,
                                instruction,
                            )
                        })
                        .or_else(|| {
                            specialize_comparison::specialized_instruction(
                                &flow,
                                index,
                                instruction,
                            )
                        }),
                };
                replacement.map(|replacement| (index, replacement))
            })
            .collect::<Vec<_>>()
    };

    let changed = !replacements.is_empty();
    for (index, replacement) in replacements {
        tail.code[index] = replacement;
    }

    let mut statistics = OptimizationStatistics::default();
    let inlined = inline_live_tail(&mut tail, functions, 0, register_cap, &mut statistics);
    if inlined && tail.switch_tables.is_empty() {
        optimize_isolated_live_tail(
            &mut tail,
            heap,
            OptimizationConfiguration::default(),
            &mut statistics,
        );
    }

    (changed || inlined).then(|| chunk.with_replaced_tail(floor, tail))
}

fn tail_is_isolated(chunk: &Chunk, floor: usize) -> bool {
    if floor >= chunk.code.len() || !chunk.catch_table.is_empty() {
        return false;
    }

    let mut edges = Vec::new();
    for index in 0..floor {
        edges.clear();
        successors(chunk, index, &mut edges);
        if edges.iter().any(|target| *target > floor) {
            return false;
        }
    }
    for index in floor..chunk.code.len() {
        edges.clear();
        successors(chunk, index, &mut edges);
        if edges.iter().any(|target| *target < floor) {
            return false;
        }
    }

    true
}
