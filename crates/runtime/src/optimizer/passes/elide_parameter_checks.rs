//! Exact method-call specialization proven by whole-unit type flow.

use crate::bytecode::instruction::Instruction;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::analysis::AnalyzedChunk;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::rewrite::plan::RewritePlan;

#[derive(Clone, Copy)]
struct ProvenSite {
    chunk: usize,
    instruction: usize,
    recursive: bool,
}

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.elide_parameter_checks {
        return;
    }

    let mut proven = vec![];
    for (position, analyzed) in analysis.chunks().iter().enumerate() {
        if !analyzed.candidates.contains(CandidateSet::CALL) {
            continue;
        }

        analyze_chunk(position, analyzed, &mut proven);
    }

    for site in proven {
        let analyzed = &analysis.chunks()[site.chunk];
        let unchecked = match analyzed.chunk.code[site.instruction] {
            Instruction::CallValue {
                argument_count,
                destination,
                callee,
                first_argument,
            } => Instruction::CallValueUnchecked {
                argument_count,
                destination,
                callee,
                first_argument,
            },
            Instruction::CallMethod {
                argument_count,
                destination,
                first_argument,
                cache,
            } => Instruction::CallMethodUnchecked {
                argument_count,
                destination,
                first_argument,
                cache,
            },
            Instruction::CallNamed {
                argument_count,
                destination,
                first_argument,
                cache,
            } => {
                if site.recursive {
                    Instruction::CallSelfUnchecked {
                        argument_count,
                        destination,
                        first_argument,
                    }
                } else {
                    Instruction::CallNamedUnchecked {
                        argument_count,
                        destination,
                        first_argument,
                        cache,
                    }
                }
            }
            _ => continue,
        };

        if analyzed.write(plan, site.instruction, unchecked) {
            statistics.parameter_checks_elided += 1;
        }
    }
}

fn analyze_chunk(position: usize, analyzed: &AnalyzedChunk<'_>, proven: &mut Vec<ProvenSite>) {
    let flow = &analyzed.flow;
    let current_function = analyzed.function_name;
    for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
        let proven_arguments = match instruction {
            Instruction::CallValue {
                argument_count,
                callee,
                first_argument,
                ..
            } => flow.callable_arguments_proven(
                index,
                callee,
                usize::from(first_argument.index()),
                usize::from(argument_count.value()),
            ),
            Instruction::CallMethod {
                argument_count,
                first_argument,
                ..
            } => {
                let count = usize::from(argument_count.value());
                count != 0
                    && flow.method_arguments_proven(
                        index,
                        usize::from(first_argument.index()) + 1,
                        count - 1,
                    )
            }
            Instruction::CallNamed {
                argument_count,
                first_argument,
                ..
            } => flow.function_arguments_proven(
                index,
                usize::from(first_argument.index()),
                usize::from(argument_count.value()),
            ),
            _ => continue,
        };

        if proven_arguments {
            let recursive = matches!(instruction, Instruction::CallNamed { .. })
                && current_function.is_some_and(|current| {
                    flow.resolved_function(index)
                        .is_some_and(|target| current.as_bytes() == target.name.as_bytes())
                });
            proven.push(ProvenSite {
                chunk: position,
                instruction: index,
                recursive,
            });
        }
    }
}
