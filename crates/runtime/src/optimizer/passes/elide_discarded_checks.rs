//! Removal of discarded-result checks whose callable cannot require them.

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::must_use_note;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::analysis::AnalyzedChunk;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::rewrite::plan::RewritePlan;

pub(in crate::optimizer) fn optimize_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> bool {
    if !configuration.elide_discarded_checks {
        return false;
    }

    let mut changed = false;
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::DISCARDED_RESULT) {
            continue;
        }

        let calls = proven_calls(analyzed);
        if calls.is_empty() {
            continue;
        }

        for call in calls {
            if plan.replace(analyzed, call, ordinary_call(analyzed.chunk.code[call])) {
                plan.remove(analyzed, call + 1);
                statistics.discarded_checks_elided += 1;
                changed = true;
            }
        }
    }

    changed
}

fn proven_calls(analyzed: &AnalyzedChunk<'_>) -> Vec<usize> {
    let chunk = analyzed.chunk;
    if chunk.code.len() < 2 {
        return Vec::new();
    }

    let flow = &analyzed.flow;
    let mut calls = Vec::new();
    for index in 0..chunk.code.len() - 1 {
        let Some(destination) = discarded_destination(chunk.code[index]) else {
            continue;
        };
        if !matches!(
            chunk.code[index + 1],
            Instruction::CheckDiscardedResult { source } if source == destination
        ) {
            continue;
        }

        let unusable = flow
            .register_type_at(index + 1, destination, 0)
            .is_some_and(|descriptor| {
                matches!(descriptor, TypeDescriptor::Void | TypeDescriptor::Never)
            });
        let exact_non_must_use = match chunk.code[index] {
            Instruction::CallMethodDiscarded { .. } => flow
                .resolved_method_at(index, 0)
                .is_some_and(|method| must_use_note(&method.function.attributes).is_none()),
            Instruction::CallNamedDiscarded { .. } => {
                flow.resolved_function(index)
                    .is_some_and(|function| must_use_note(&function.attributes).is_none())
                    || flow
                        .resolved_built_in_function(index)
                        .is_some_and(|function| !function.attributes.must_use)
            }
            _ => false,
        };
        if unusable || exact_non_must_use {
            calls.push(index);
        }
    }

    calls
}

fn discarded_destination(instruction: Instruction) -> Option<Register> {
    match instruction {
        Instruction::CallValueDiscarded { destination, .. }
        | Instruction::CallNamedDiscarded { destination, .. }
        | Instruction::CallMethodDiscarded { destination, .. }
        | Instruction::CallStaticDiscarded { destination, .. }
        | Instruction::CallWithNamesDiscarded { destination, .. } => Some(destination),
        _ => None,
    }
}

fn ordinary_call(instruction: Instruction) -> Instruction {
    match instruction {
        Instruction::CallValueDiscarded {
            argument_count,
            destination,
            callee,
            first_argument,
        } => Instruction::CallValue {
            argument_count,
            destination,
            callee,
            first_argument,
        },
        Instruction::CallNamedDiscarded {
            argument_count,
            destination,
            first_argument,
            cache,
        } => Instruction::CallNamed {
            argument_count,
            destination,
            first_argument,
            cache,
        },
        Instruction::CallMethodDiscarded {
            argument_count,
            destination,
            first_argument,
            cache,
        } => Instruction::CallMethod {
            argument_count,
            destination,
            first_argument,
            cache,
        },
        Instruction::CallStaticDiscarded {
            argument_count,
            destination,
            first_argument,
            cache,
        } => Instruction::CallStatic {
            argument_count,
            destination,
            first_argument,
            cache,
        },
        Instruction::CallWithNamesDiscarded {
            destination,
            callee,
            descriptor,
        } => Instruction::CallWithNames {
            destination,
            callee,
            descriptor,
        },
        other => other,
    }
}
