//! Property-write specialization proven by whole-unit type flow.

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::bytecode::instruction::operands::PropertySlot;
use crate::bytecode::instruction::operands::PropertyValueMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::Visibility;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::analysis::AnalyzedChunk;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::rewrite::plan::RewritePlan;

#[derive(Clone, Copy)]
struct ProvenSite {
    chunk: usize,
    instruction: usize,
    slot: PropertySlot,
    move_value: bool,
    fresh_receiver: bool,
}

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.elide_property_checks {
        return;
    }

    let mut proven = vec![];
    for (position, analyzed) in analysis.chunks().iter().enumerate() {
        if !analyzed.candidates.contains(CandidateSet::PROPERTY) {
            continue;
        }

        analyze_chunk(position, analyzed, &mut proven);
    }

    for site in proven {
        let analyzed = &analysis.chunks()[site.chunk];
        let unchecked = match analyzed.chunk.code[site.instruction] {
            Instruction::PropertySet {
                object,
                value,
                cache: _,
            }
            | Instruction::PropertyInitRaw {
                object,
                value,
                cache: _,
            } => {
                let value_mode = match (site.fresh_receiver, site.move_value) {
                    (false, false) => PropertyValueMode::Clone,
                    (false, true) => PropertyValueMode::MoveAndClear,
                    (true, false) => PropertyValueMode::FreshClone,
                    (true, true) => PropertyValueMode::FreshMoveAndClear,
                };
                Instruction::PropertySetUnchecked {
                    object,
                    value,
                    slot: site.slot,
                    value_mode,
                }
            }
            Instruction::PropertyIndexUpdate {
                object,
                operand,
                cache: _,
                mode,
            } => Instruction::PropertyIndexUpdateUnchecked {
                object,
                operand,
                slot: site.slot,
                mode,
            },
            Instruction::PropertyIndexSet {
                object,
                first_operand,
                cache: _,
            } => Instruction::PropertyIndexSetUnchecked {
                object,
                first_operand,
                slot: site.slot,
            },
            Instruction::PropertyRemove {
                object,
                destination,
                cache: _,
                mode,
            } => Instruction::PropertyRemoveUnchecked {
                object,
                destination,
                slot: site.slot,
                mode,
            },
            Instruction::PropertyStep {
                object,
                cache: _,
                immediate,
            } => Instruction::PropertyStepUnchecked {
                object,
                slot: site.slot,
                immediate,
            },
            Instruction::PropertyAdd {
                object,
                source,
                cache: _,
            } => Instruction::PropertyAddUnchecked {
                object,
                source,
                slot: site.slot,
            },
            _ => continue,
        };

        if analyzed.write(plan, site.instruction, unchecked) {
            statistics.property_checks_elided += 1;
        }
    }
}

fn analyze_chunk(position: usize, analyzed: &AnalyzedChunk<'_>, proven: &mut Vec<ProvenSite>) {
    let flow = &analyzed.flow;
    let class_name = analyzed.class_name;
    let has_receiver = analyzed.has_receiver;
    for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
        let (object, cache) = match instruction {
            Instruction::PropertySet { object, cache, .. }
            | Instruction::PropertyInitRaw { object, cache, .. }
            | Instruction::PropertyIndexSet { object, cache, .. }
            | Instruction::PropertyIndexUpdate { object, cache, .. }
            | Instruction::PropertyRemove { object, cache, .. }
            | Instruction::PropertyStep { object, cache, .. }
            | Instruction::PropertyAdd { object, cache, .. } => (object, cache),
            _ => continue,
        };

        let Some(resolved) = flow.resolved_property(index, object, cache) else {
            continue;
        };

        let property = resolved.property;
        let raw_initialization = matches!(instruction, Instruction::PropertyInitRaw { .. });
        if raw_initialization && !(has_receiver && object.index() == 0) {
            continue;
        }

        if !raw_initialization
            && (property.is_readonly
                || resolved.class.is_readonly
                || (property.visibility != Visibility::Public
                    && class_name != Some(&resolved.class.name)))
        {
            continue;
        }

        let valid = match instruction {
            Instruction::PropertySet { value, .. } | Instruction::PropertyInitRaw { value, .. } => {
                property
                    .declared_type
                    .as_ref()
                    .is_none_or(|descriptor| flow.proves(index, value, descriptor))
            }
            Instruction::PropertyIndexUpdate {
                operand: key_register,
                mode: PropertyIndexUpdateMode::Increment,
                ..
            } => {
                let Some(TypeDescriptor::Dictionary(Some((key, value)))) =
                    property.declared_type.as_ref()
                else {
                    continue;
                };
                matches!(key.as_ref(), TypeDescriptor::Int)
                    && matches!(value.as_ref(), TypeDescriptor::Int)
                    && flow.proves(index, key_register, &TypeDescriptor::Int)
            }
            Instruction::PropertyIndexUpdate {
                mode: PropertyIndexUpdateMode::Remove,
                ..
            }
            | Instruction::PropertyRemove { .. } => true,
            Instruction::PropertyIndexUpdate {
                operand: value,
                mode: PropertyIndexUpdateMode::Append,
                ..
            } => {
                let Some(TypeDescriptor::Vector(element)) = property.declared_type.as_ref() else {
                    continue;
                };
                element
                    .as_ref()
                    .is_none_or(|element| flow.proves(index, value, element))
            }
            Instruction::PropertyIndexSet { first_operand, .. } => {
                let key = first_operand;
                let value = Register::new(first_operand.index() + 1);
                match property.declared_type.as_ref() {
                    None
                    | Some(
                        TypeDescriptor::Wildcard
                        | TypeDescriptor::Mixed
                        | TypeDescriptor::Dictionary(None),
                    ) => true,
                    Some(TypeDescriptor::Vector(element)) => {
                        flow.proves(index, key, &TypeDescriptor::Int)
                            && element
                                .as_ref()
                                .is_none_or(|element| flow.proves(index, value, element))
                    }
                    Some(TypeDescriptor::Dictionary(Some((key_type, value_type)))) => {
                        flow.proves(index, key, key_type) && flow.proves(index, value, value_type)
                    }
                    _ => false,
                }
            }
            Instruction::PropertyStep { .. } => matches!(
                property.declared_type.as_ref(),
                Some(TypeDescriptor::Int | TypeDescriptor::Float)
            ),
            Instruction::PropertyAdd { source, .. } => match property.declared_type.as_ref() {
                Some(TypeDescriptor::Int) => flow.proves(index, source, &TypeDescriptor::Int),
                Some(TypeDescriptor::Float) => flow.proves(index, source, &TypeDescriptor::Float),
                _ => false,
            },
            _ => false,
        };

        if valid {
            proven.push(ProvenSite {
                chunk: position,
                instruction: index,
                slot: PropertySlot::new(resolved.slot),
                move_value: match instruction {
                    Instruction::PropertySet { object, value, .. }
                    | Instruction::PropertyInitRaw { object, value, .. } => {
                        object != value
                            && !(has_receiver && value.index() == 0)
                            && register_is_dead_after(analyzed.chunk, value, index + 1)
                    }
                    _ => false,
                },
                fresh_receiver: matches!(instruction, Instruction::PropertyInitRaw { .. }),
            });
        }
    }
}
