//! Folding of operations whose result is known at compile time.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::LivenessQueries;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::dead_store::PreviousValueSafety;
use crate::optimizer::passes::dead_store::scalar_write_is_unobservable;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::ConstantValue;
use crate::optimizer::type_flow::TypeFlow;
use crate::value::heap::Heap;

pub(in crate::optimizer) fn optimize_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.const_fold {
        return;
    }

    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::CONSTANT) {
            continue;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) || !foldable(instruction) {
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
            }
        }
    }
}

pub(in crate::optimizer) fn remove_unit(
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
        | Instruction::LoadInt { destination, .. } => Some(destination),
        _ => None,
    }
}

pub(in crate::optimizer::passes) fn prepare_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if configuration.const_fold {
        fold_joined_string_lengths(chunk, statistics);
    }
}

pub(in crate::optimizer::passes) fn optimize_chunk(
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
    let flow = TypeFlow::analyze(chunk, &[], false, None, &[], allocator);
    for index in 0..chunk.code.len() {
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
    if chunk.code.len() < 7 {
        return;
    }

    for consumer in 6..chunk.code.len() {
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

        let (join, joined) = match chunk.code[consumer - 1] {
            Instruction::Move {
                destination: moved,
                source: joined,
            }
            | Instruction::MoveOwned {
                destination: moved,
                source: joined,
            } if moved == source => (consumer - 1, joined),
            _ => (consumer, source),
        };
        if join < 5
            || !register_is_dead_after(chunk, joined, consumer + 1)
            || !register_is_dead_after(chunk, source, consumer + 1)
        {
            continue;
        }

        let first = join - 4;
        let second = join - 2;
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
            Instruction::Jump { .. },
        ) = (
            chunk.code[first],
            chunk.code[first + 1],
            chunk.code[second],
            chunk.code[second + 1],
        )
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

        let mut edges = Vec::new();
        successors(chunk, first - 1, &mut edges);
        if edges.len() != 2 || !edges.contains(&first) || !edges.contains(&second) {
            continue;
        }
        edges.clear();
        successors(chunk, first + 1, &mut edges);
        if edges.as_slice() != [join] {
            continue;
        }
        edges.clear();
        successors(chunk, second + 1, &mut edges);
        if edges.as_slice() != [join] {
            continue;
        }

        chunk.code[first] = Instruction::LoadInt {
            destination: joined,
            immediate: ImmediateInt::new(first_length),
        };
        chunk.code[second] = Instruction::LoadInt {
            destination: joined,
            immediate: ImmediateInt::new(second_length),
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
            | Instruction::LoadInt { .. }
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
                Some(Instruction::LoadInt {
                    destination,
                    immediate: ImmediateInt::new(immediate),
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
        ConstantValue::String(value) => {
            let constant = intern(Literal::String(value))?;
            Some(Instruction::LoadConstant {
                destination,
                constant,
            })
        }
    }
}
