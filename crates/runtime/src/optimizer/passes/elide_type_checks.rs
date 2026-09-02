//! Removal of runtime type checks already proven by forward type flow.

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::passes::FunctionLocation;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::TypeFlow;

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.elide_type_checks {
        return;
    }

    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::TYPE_CHECK) {
            continue;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) {
                continue;
            }

            if let Instruction::CheckDestructure {
                subject,
                required,
                arity,
                rest,
            } = instruction
                && analyzed.flow.destructure_proven(
                    index,
                    subject,
                    required.value() as usize,
                    arity.value() as usize,
                    rest,
                )
            {
                if plan.remove(analyzed, index) {
                    statistics.type_checks_elided += 1;
                }
                continue;
            }

            if matches!(analyzed.location, FunctionLocation::Main) {
                continue;
            }

            if !return_is_proven(&analyzed.flow, index, instruction, analyzed.return_type) {
                continue;
            }

            let replacement = unchecked_return(
                instruction,
                return_is_reference_counted(&analyzed.flow, index, instruction),
                return_is_scalar(&analyzed.flow, index, instruction),
            );
            if analyzed.write(plan, index, replacement) {
                statistics.type_checks_elided += 1;
            }
        }
    }
}

fn return_is_proven(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
    return_type: Option<&TypeDescriptor>,
) -> bool {
    match (instruction, return_type) {
        (Instruction::Return { .. }, None)
        | (Instruction::ReturnNull, None)
        | (Instruction::ReturnNull, Some(TypeDescriptor::Void)) => true,
        (Instruction::Return { source }, Some(expected)) => {
            flow.proves(index, source, expected)
                || flow.proves_constructed_array(index, source, expected)
        }
        (Instruction::ReturnNull, Some(expected)) => {
            matches!(expected, TypeDescriptor::Null | TypeDescriptor::Mixed)
                || matches!(expected, TypeDescriptor::Union(members) if members.iter().any(|member| matches!(member, TypeDescriptor::Null)))
        }
        _ => false,
    }
}

fn return_is_reference_counted(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
) -> bool {
    matches!(
        instruction,
        Instruction::Return { source } if flow.proves_reference_counted(index, source)
    )
}

fn return_is_scalar(flow: &TypeFlow<'_>, index: usize, instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Return { source } if flow.proves_scalar(index, source)
    )
}

fn unchecked_return(
    instruction: Instruction,
    reference_counted: bool,
    scalar: bool,
) -> Instruction {
    match instruction {
        Instruction::Return { source } if reference_counted => {
            Instruction::ReturnReferenceUnchecked { source }
        }
        Instruction::Return { source } if scalar => Instruction::ReturnScalarUnchecked { source },
        Instruction::Return { source } => Instruction::ReturnUnchecked { source },
        Instruction::ReturnNull => Instruction::ReturnNullUnchecked,
        other => other,
    }
}
