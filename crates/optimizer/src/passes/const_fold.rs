//! Folding of operations whose result is known at compile time.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ConstantIndex;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::rewrite::control_flow_targets;
use whim_value::heap::Heap;

use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;
use crate::analysis::Analysis;
use crate::candidates::CandidateSet;
use crate::cfg::successors;
use crate::liveness::LivenessQueries;
use crate::liveness::LivenessScratch;
use crate::liveness::register_is_dead_after;
use crate::liveness::register_is_dead_after_removals_with_scratch;
use crate::passes::compact_removed_instructions;
use crate::passes::dead_store::PreviousValueSafety;
use crate::passes::dead_store::scalar_write_is_unobservable;
use crate::passes::prune_unreachable;
use crate::rewrite::plan::RewritePlan;
use crate::type_flow::ConstantValue;
use crate::type_flow::TypeFlow;

#[cfg(test)]
mod tests;

pub(crate) fn optimize_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.const_fold {
        return;
    }

    let mut liveness = LivenessScratch::default();
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::CONSTANT) {
            continue;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) || !foldable(instruction) {
                continue;
            }

            if let Some(offset) = analyzed.flow.constant_branch_offset(index) {
                if analyzed.write(
                    plan,
                    index,
                    Instruction::Jump {
                        offset: JumpOffset::new(offset),
                    },
                ) {
                    statistics.constants_folded += 1;
                }

                continue;
            }

            let Some((destination, value)) = analyzed.flow.constant_result(index) else {
                continue;
            };

            let Some(replacement) = constant_instruction(destination, value, |literal| {
                plan.intern_constant(analyzed, literal)
            }) else {
                continue;
            };

            if analyzed.write(plan, index, replacement) {
                statistics.constants_folded += 1;
                if matches!(replacement, Instruction::LoadTrue { .. })
                    && feeds_terminal_assertion(analyzed.chunk, index, destination, &mut liveness)
                {
                    statistics.terminal_assertion_constants += 1;
                }
            }
        }
    }
}

fn feeds_terminal_assertion(
    chunk: &Chunk,
    index: usize,
    destination: Register,
    liveness: &mut LivenessScratch,
) -> bool {
    destination.index() >= chunk.local_register_count
        && !chunk.trace_argument_registers.contains(&destination)
        && matches!(
            chunk.code.get(index + 1),
            Some(Instruction::Assert { first_value, .. }) if *first_value == destination
        )
        && register_is_dead_after_removals_with_scratch(
            chunk,
            destination,
            index + 2,
            &[],
            liveness,
        )
}

pub(crate) fn remove_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> bool {
    if !configuration.const_fold {
        return false;
    }

    let mut changed = false;
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::CONSTANT) {
            continue;
        }

        let targets = control_flow_targets(analyzed.chunk);
        let previous_values = PreviousValueSafety::analyze(
            analyzed.chunk,
            &targets,
            analyzed.incoming_register_count,
        );
        let mut effective = Vec::new();
        let code = if plan.has_replacements(analyzed) {
            effective.clone_from(&analyzed.chunk.code);
            for (index, instruction) in effective.iter_mut().enumerate() {
                if let Some(replacement) = plan.replacement(analyzed, index) {
                    *instruction = replacement;
                }
            }

            &effective
        } else {
            &analyzed.chunk.code
        };

        let mut remove = vec![false; code.len()];
        let liveness = LivenessQueries::for_effective_code(analyzed.chunk, code);
        for (index, removed) in remove.iter_mut().enumerate() {
            let Some(destination) = analyzed.flow.pure_constant_destination(index) else {
                continue;
            };
            if plan
                .replacement(analyzed, index)
                .is_some_and(|replacement| literal_destination(replacement) != Some(destination))
            {
                continue;
            }

            if scalar_write_is_unobservable(
                analyzed.chunk,
                &targets,
                Some(&analyzed.flow),
                &previous_values,
                index,
                destination,
            ) && liveness.register_is_dead_after(analyzed.chunk, destination, index + 1)
            {
                *removed = true;
            }
        }

        let removed = remove.iter().filter(|removed| **removed).count();
        if removed == 0 {
            continue;
        }

        for (index, removed) in remove.iter().copied().enumerate() {
            if removed {
                plan.remove(analyzed, index);
            }
        }
        statistics.instructions_removed += removed;
        changed = true;
    }

    changed
}

fn literal_destination(instruction: Instruction) -> Option<Register> {
    match instruction {
        Instruction::LoadConstant { destination, .. }
        | Instruction::LoadNull { destination }
        | Instruction::LoadTrue { destination }
        | Instruction::LoadFalse { destination }
        | Instruction::LoadInteger { destination, .. } => Some(destination),
        _ => None,
    }
}

pub(in crate::passes) fn prepare_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if configuration.const_fold {
        fold_joined_string_lengths(chunk, statistics);
    }
}

pub(in crate::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    allocator: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.const_fold || chunk.code.is_empty() {
        return;
    }

    prepare_chunk(chunk, configuration, statistics);

    let mut folds = vec![];
    let mut branches = Vec::new();
    let flow = TypeFlow::analyze(chunk, &[], false, None, &[], allocator);
    for index in 0..chunk.code.len() {
        if let Some(offset) = flow.constant_branch_offset(index) {
            branches.push((index, offset));
        }
        folds.push(if foldable(chunk.code[index]) {
            flow.constant_result(index)
        } else {
            None
        });
    }

    for (index, fold) in folds.into_iter().enumerate() {
        let Some((destination, value)) = fold else {
            continue;
        };

        let Some(replacement) = constant_instruction(destination, value, |literal| {
            chunk.add_constant(literal).ok()
        }) else {
            continue;
        };

        chunk.code[index] = replacement;
        statistics.constants_folded += 1;
    }

    if !branches.is_empty() {
        for (index, offset) in branches {
            chunk.code[index] = Instruction::Jump {
                offset: JumpOffset::new(offset),
            };

            statistics.constants_folded += 1;
        }

        prune_unreachable::optimize_chunk(chunk);
    }

    loop {
        let mut remove = vec![false; chunk.code.len()];
        let targets = control_flow_targets(chunk);
        let previous_values =
            PreviousValueSafety::analyze(chunk, &targets, chunk.local_register_count);
        let flow = TypeFlow::analyze(chunk, &[], false, None, &[], allocator);
        for (index, removed) in remove.iter_mut().enumerate() {
            let Some(destination) = flow.pure_constant_destination(index) else {
                continue;
            };

            if scalar_write_is_unobservable(
                chunk,
                &targets,
                Some(&flow),
                &previous_values,
                index,
                destination,
            ) && register_is_dead_after(chunk, destination, index + 1)
            {
                *removed = true;
            }
        }

        if compact_removed_instructions(chunk, &remove, statistics) == 0 {
            break;
        }
    }
}

/// Distributes a string-length consumer across the canonical two-arm join
/// emitted by `match` and conditional expressions. Both arms already carry
/// literal strings, so preserving the temporary string at the join only to
/// measure it would perform avoidable runtime work.
fn fold_joined_string_lengths(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 5
        || !chunk.catch_table.is_empty()
        || !chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::StringLength { .. } | Instruction::Length { .. }
            )
        })
    {
        return;
    }

    let mut predecessors = vec![0; chunk.code.len() + 1];
    let mut edges = Vec::new();
    for index in 0..chunk.code.len() {
        edges.clear();
        successors(chunk, index, &mut edges);
        edges.sort_unstable();
        edges.dedup();
        for target in &edges {
            predecessors[*target] += 1;
        }
    }

    for consumer in 4..chunk.code.len() {
        let (Instruction::StringLength {
            destination,
            source,
        }
        | Instruction::Length {
            destination,
            source,
        }) = chunk.code[consumer]
        else {
            continue;
        };

        let mut join = consumer;
        while join > 0
            && matches!(chunk.code[join - 1], Instruction::Clear { target } if target != source)
        {
            join -= 1;
        }
        if join == 0 {
            continue;
        }
        let (join, joined) = match chunk.code[join - 1] {
            Instruction::Move {
                destination: moved,
                source: joined,
            }
            | Instruction::MoveOwned {
                destination: moved,
                source: joined,
            } if moved == source => (join - 1, joined),
            _ => (join, source),
        };
        if join == 0
            || [joined, source].iter().any(|register| {
                register
                    .index()
                    .wrapping_sub(chunk.parameter_register_start)
                    < chunk.parameter_register_count
                    || chunk.trace_argument_registers.contains(register)
                    || (*register != destination
                        && !register_is_dead_after(chunk, *register, consumer + 1))
            })
        {
            continue;
        }

        let second_jumps = matches!(chunk.code[join - 1], Instruction::Jump { .. });
        let arm_size = if second_jumps { 2 } else { 1 };
        if join < arm_size + 3 {
            continue;
        }
        let second = join - arm_size;
        let first = second - 2;
        if predecessors[join] != 2
            || (first..join).any(|index| predecessors[index] != 1)
            || (join + 1..=consumer).any(|index| predecessors[index] != 1)
        {
            continue;
        }
        let (
            Instruction::LoadConstant {
                destination: first_destination,
                constant: first_constant,
            },
            Instruction::Jump { .. },
            Instruction::LoadConstant {
                destination: second_destination,
                constant: second_constant,
            },
        ) = (chunk.code[first], chunk.code[first + 1], chunk.code[second])
        else {
            continue;
        };
        if first_destination != joined || second_destination != joined {
            continue;
        }

        let (Literal::String(first_value), Literal::String(second_value)) = (
            &chunk.constants[usize::from(first_constant.index())],
            &chunk.constants[usize::from(second_constant.index())],
        ) else {
            continue;
        };
        let (Ok(first_length), Ok(second_length)) = (
            i16::try_from(first_value.as_bytes().len()),
            i16::try_from(second_value.as_bytes().len()),
        ) else {
            continue;
        };

        edges.clear();
        successors(chunk, first - 1, &mut edges);
        edges.sort_unstable();
        edges.dedup();
        if edges.len() != 2 || !edges.contains(&first) || !edges.contains(&second) {
            continue;
        }
        edges.clear();
        successors(chunk, first + 1, &mut edges);
        if edges.as_slice() != [join] {
            continue;
        }
        edges.clear();
        successors(chunk, join - 1, &mut edges);
        if edges.as_slice() != [join] {
            continue;
        }

        chunk.code[first] = Instruction::LoadInteger {
            kind: IntegerKind::I64,
            destination: joined,
            immediate: ImmediateInteger::signed(first_length),
        };
        chunk.code[second] = Instruction::LoadInteger {
            kind: IntegerKind::I64,
            destination: joined,
            immediate: ImmediateInteger::signed(second_length),
        };
        chunk.code[consumer] = Instruction::Move {
            destination,
            source,
        };
        statistics.constants_folded += 2;
    }
}

fn foldable(instruction: Instruction) -> bool {
    !matches!(
        instruction,
        Instruction::LoadConstant { .. }
            | Instruction::LoadNull { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::LoadInteger { .. }
    )
}

fn constant_instruction(
    destination: Register,
    value: ConstantValue,
    mut intern: impl FnMut(Literal) -> Option<ConstantIndex>,
) -> Option<Instruction> {
    match value {
        ConstantValue::Null => Some(Instruction::LoadNull { destination }),
        ConstantValue::Bool(true) => Some(Instruction::LoadTrue { destination }),
        ConstantValue::Bool(false) => Some(Instruction::LoadFalse { destination }),
        ConstantValue::Int(value) => {
            if let Ok(immediate) = i16::try_from(value) {
                Some(Instruction::LoadInteger {
                    kind: IntegerKind::I64,
                    destination,
                    immediate: ImmediateInteger::signed(immediate),
                })
            } else {
                let constant = intern(Literal::Int(value))?;
                Some(Instruction::LoadConstant {
                    destination,
                    constant,
                })
            }
        }
        ConstantValue::Float(value) => {
            let constant = intern(Literal::Float(value))?;
            Some(Instruction::LoadConstant {
                destination,
                constant,
            })
        }
        ConstantValue::Uint(value) => Some(if let Ok(immediate) = u16::try_from(value) {
            Instruction::LoadInteger {
                kind: IntegerKind::U64,
                destination,
                immediate: ImmediateInteger::unsigned(immediate),
            }
        } else {
            Instruction::LoadConstant {
                destination,
                constant: intern(Literal::Uint(value))?,
            }
        }),
        ConstantValue::String(value) => {
            let constant = intern(Literal::String(value))?;
            Some(Instruction::LoadConstant {
                destination,
                constant,
            })
        }
    }
}
