//! Removal of runtime type checks already proven by forward type flow.

use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::AsMode;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::rewrite::for_each_control_flow_target;
use whim_bytecode::rewrite::relative_target;

use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;
use crate::analysis::Analysis;
use crate::analysis::AnalyzedChunk;
use crate::candidates::CandidateSet;
use crate::passes::FunctionLocation;
use crate::rewrite::plan::RewritePlan;
use crate::type_flow::TypeFlow;

pub(crate) fn optimize_unit(
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

        if let Some(instruction) = provided_default_branch(analyzed, plan)
            && analyzed.write(plan, 0, instruction)
        {
            statistics.type_checks_elided += 1;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) {
                continue;
            }

            let null_branch = match instruction {
                Instruction::JumpIfNull { subject, offset } => Some((subject, offset, true)),
                Instruction::JumpIfNotNull { subject, offset } => Some((subject, offset, false)),
                _ => None,
            };
            if let Some((subject, offset, branch_on_null)) = null_branch
                && let Some(is_null) = analyzed.flow.nullness(index, subject)
            {
                let changed = if is_null == branch_on_null {
                    analyzed.write(plan, index, Instruction::Jump { offset })
                } else {
                    plan.remove(analyzed, index)
                };
                if changed {
                    statistics.type_checks_elided += 1;
                }
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

fn provided_default_branch(
    analyzed: &AnalyzedChunk<'_>,
    plan: &RewritePlan,
) -> Option<Instruction> {
    let Instruction::FillDefault { target, offset } = *analyzed.chunk.code.first()? else {
        return None;
    };
    let check = relative_target(0, offset.offset());
    let Instruction::AsCheck {
        destination,
        source,
        descriptor,
        mode: AsMode::Boundary,
    } = *analyzed.chunk.code.get(check)?
    else {
        return None;
    };
    if destination != target
        || source != target
        || !plan.is_available(analyzed, 0)
        || !plan.is_available(analyzed, check)
        || check + 1 >= analyzed.chunk.code.len()
    {
        return None;
    }
    let parameter = usize::from(target.index()).checked_sub(usize::from(analyzed.has_receiver))?;
    if !analyzed.flow.provided_default_type_proven(
        parameter,
        &analyzed.chunk.type_descriptors[usize::from(descriptor.index())],
    ) {
        return None;
    }
    let mut reenters = false;
    for_each_control_flow_target(analyzed.chunk, |target| reenters |= target == 0);
    if reenters {
        return None;
    }

    Some(Instruction::FillDefault {
        target,
        offset: JumpOffset::new(offset.offset().checked_add(1)?),
    })
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
