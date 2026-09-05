use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::chunk::descriptors::descriptor_is_trivial;
use crate::bytecode::chunk::descriptors::string_switch_buckets;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::liveness::LivenessQueries;
use crate::optimizer::passes::dead_store;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::optimizer::passes::prune_unreachable;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::value::atom::Atom;

pub(in crate::optimizer) fn optimize_unit(
    analysis: &Analysis<'_>,
    plan: &mut RewritePlan,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_comparison {
        return;
    }
    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::COMPARISON) {
            continue;
        }
        let mut targets = None;
        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) {
                continue;
            }
            match instruction {
                Instruction::Is {
                    destination,
                    source,
                    descriptor,
                } => {
                    let descriptor =
                        &analyzed.chunk.type_descriptors[usize::from(descriptor.index())];
                    if !descriptor_is_trivial(descriptor) {
                        continue;
                    }
                    let value = if analyzed.flow.proves(index, source, descriptor) {
                        true
                    } else if analyzed.flow.disproves(index, source, descriptor) {
                        false
                    } else {
                        continue;
                    };
                    let replacement = if value {
                        Instruction::LoadTrue { destination }
                    } else {
                        Instruction::LoadFalse { destination }
                    };
                    if analyzed.write(plan, index, replacement) {
                        statistics.operations_specialized += 1;
                        let replacement = match analyzed.chunk.code.get(index + 1) {
                            Some(Instruction::JumpIfFalse { condition, offset })
                                if *condition == destination =>
                            {
                                Some(Instruction::Jump {
                                    offset: if value { JumpOffset::new(1) } else { *offset },
                                })
                            }
                            Some(Instruction::JumpIfTrue { condition, offset })
                                if *condition == destination =>
                            {
                                Some(Instruction::Jump {
                                    offset: if value { *offset } else { JumpOffset::new(1) },
                                })
                            }
                            _ => None,
                        };
                        if let Some(replacement) = replacement
                            && !targets
                                .get_or_insert_with(|| control_flow_targets(analyzed.chunk))
                                .contains(&(index + 1))
                        {
                            analyzed.write(plan, index + 1, replacement);
                        }
                    }
                }
                Instruction::IntRangeJumpUnless {
                    subject,
                    descriptor,
                    ..
                } => {
                    let descriptor =
                        &analyzed.chunk.type_descriptors[usize::from(descriptor.index())];
                    if analyzed.flow.proves(index, subject, descriptor)
                        && analyzed.write(
                            plan,
                            index,
                            Instruction::Jump {
                                offset: JumpOffset::new(1),
                            },
                        )
                    {
                        statistics.operations_specialized += 1;
                    }
                }
                Instruction::SwitchPattern { subject, table } => {
                    let SwitchTable::Pattern {
                        descriptors,
                        targets,
                        default,
                    } = &analyzed.chunk.switch_tables[usize::from(table.index())]
                    else {
                        continue;
                    };
                    let guaranteed = descriptors
                        .iter()
                        .position(|descriptor| analyzed.flow.proves(index, subject, descriptor));
                    let prefix = guaranteed.unwrap_or(descriptors.len());
                    if guaranteed.is_none()
                        && !descriptors[..prefix]
                            .iter()
                            .any(|descriptor| analyzed.flow.disproves(index, subject, descriptor))
                    {
                        continue;
                    }
                    let default = guaranteed.map_or(*default, |position| targets[position]);
                    let retained = descriptors[..prefix]
                        .iter()
                        .zip(&targets[..prefix])
                        .filter(|(descriptor, _)| {
                            !analyzed.flow.disproves(index, subject, descriptor)
                        })
                        .collect::<Vec<_>>();
                    let replacement = if retained.is_empty() {
                        Instruction::Jump {
                            offset: JumpOffset::new(default),
                        }
                    } else {
                        let strings = retained
                            .iter()
                            .map(|(descriptor, target)| {
                                if let TypeDescriptor::StringLiteral(value) = descriptor {
                                    Some((value.clone(), **target))
                                } else {
                                    None
                                }
                            })
                            .collect::<Option<Vec<_>>>();
                        let (replacement, string) = if let Some(arms) = strings {
                            let buckets = string_switch_buckets(&arms);
                            (
                                SwitchTable::String {
                                    arms,
                                    buckets,
                                    default,
                                },
                                true,
                            )
                        } else {
                            (
                                SwitchTable::Pattern {
                                    descriptors: retained
                                        .iter()
                                        .map(|(descriptor, _)| (*descriptor).clone())
                                        .collect(),
                                    targets: retained.iter().map(|(_, target)| **target).collect(),
                                    default,
                                },
                                false,
                            )
                        };
                        let Some(table) = plan.add_switch_table(analyzed, replacement) else {
                            continue;
                        };
                        if string {
                            Instruction::SwitchString { subject, table }
                        } else {
                            Instruction::SwitchPattern { subject, table }
                        }
                    };
                    if analyzed.write(plan, index, replacement) {
                        statistics.operations_specialized += 1;
                    }
                }
                _ => {}
            }
        }
    }
}

pub(in crate::optimizer) fn canonicalize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if configuration.specialize_comparison {
        for_each_mutable_chunk(unit, configuration, |chunk| {
            let changes = canonicalize_string_chains(chunk);
            if changes.changed {
                statistics.operations_specialized += 1;
                prune_unreachable::optimize_chunk(chunk);
                if changes.retained_initial_load {
                    dead_store::optimize_chunk_without_flow(chunk, configuration, statistics);
                }
            }
        });
    }
}

struct StringTest {
    subject: Register,
    temporary: Register,
    literal: Atom,
    rejected: usize,
}

fn string_test(chunk: &Chunk, load: usize) -> Option<StringTest> {
    let Instruction::LoadConstant {
        destination: temporary,
        constant,
    } = *chunk.code.get(load)?
    else {
        return None;
    };
    if temporary.index() < chunk.local_register_count {
        return None;
    }
    let Literal::String(literal) = &chunk.constants[usize::from(constant.index())] else {
        return None;
    };
    let Instruction::StringJumpUnless {
        comparison: Comparison::Equal,
        left,
        right,
        offset,
    } = *chunk.code.get(load + 1)?
    else {
        return None;
    };
    let subject = if right == temporary && left != temporary {
        left
    } else if left == temporary && right != temporary {
        right
    } else {
        return None;
    };
    let rejected = relative_target(load + 1, i32::from(offset.offset()));
    if rejected <= load + 2 {
        return None;
    }
    Some(StringTest {
        subject,
        temporary,
        literal: literal.clone(),
        rejected,
    })
}

#[derive(Default)]
struct StringChainChanges {
    changed: bool,
    retained_initial_load: bool,
}

fn returns_without_reading(instruction: Instruction, register: Register) -> bool {
    match instruction {
        Instruction::ReturnIntUnchecked { .. } | Instruction::ReturnNullUnchecked => true,
        Instruction::ReturnUnchecked { source }
        | Instruction::ReturnReferenceUnchecked { source }
        | Instruction::ReturnScalarUnchecked { source } => source != register,
        Instruction::ReturnPairUnchecked { first, second } => {
            first != register && second != register
        }
        _ => false,
    }
}

fn canonicalize_string_chains(chunk: &mut Chunk) -> StringChainChanges {
    if chunk.code.len() < 6 || !chunk.catch_table.is_empty() {
        return StringChainChanges::default();
    }
    if !chunk
        .code
        .iter()
        .any(|instruction| matches!(instruction, Instruction::StringJumpUnless { .. }))
    {
        return StringChainChanges::default();
    }
    let targets = control_flow_targets(chunk);
    let mut liveness = None;
    let mut claimed = vec![false; chunk.code.len()];
    let mut changes = StringChainChanges::default();
    for first in 0..chunk.code.len() - 1 {
        if claimed[first] || targets.contains(&(first + 1)) {
            continue;
        }
        let Some(initial) = string_test(chunk, first) else {
            continue;
        };
        let Some(second) = string_test(chunk, initial.rejected) else {
            continue;
        };
        if second.subject != initial.subject || second.temporary != initial.temporary {
            continue;
        }
        let fresh_entry = first == 0
            && !targets.contains(&0)
            && !chunk.trace_argument_registers.contains(&initial.temporary);
        let switch = if fresh_entry { first } else { first + 1 };
        let mut seen = HashSet::new();
        seen.insert(initial.literal.clone());
        let mut arms = vec![(initial.literal, if fresh_entry { 2 } else { 1 })];
        let mut bodies = vec![first + 2];
        let mut loads = vec![first];
        let mut next = initial.rejected;
        while let Some(test) = string_test(chunk, next) {
            if test.subject != initial.subject || test.temporary != initial.temporary {
                break;
            }
            let Ok(offset) = i32::try_from(next + 2 - switch) else {
                break;
            };
            if seen.insert(test.literal.clone()) {
                arms.push((test.literal, offset));
            }
            loads.push(next);
            bodies.push(next + 2);
            next = test.rejected;
        }
        for load in loads {
            claimed[load] = true;
        }
        if bodies.len() < 2
            || bodies.iter().copied().chain([next]).any(|body| {
                !chunk.code.get(body).is_some_and(|instruction| {
                    returns_without_reading(*instruction, initial.temporary)
                }) && !liveness
                    .get_or_insert_with(|| LivenessQueries::for_chunk(chunk, chunk.code.len()))
                    .register_is_dead_after(chunk, initial.temporary, body)
            })
        {
            continue;
        }
        let Ok(default) = i32::try_from(next - switch) else {
            continue;
        };
        let buckets = string_switch_buckets(&arms);
        let Ok(table) = chunk.add_switch_table(SwitchTable::String {
            arms,
            buckets,
            default,
        }) else {
            continue;
        };
        chunk.code[switch] = Instruction::SwitchString {
            subject: initial.subject,
            table,
        };
        changes.changed = true;
        changes.retained_initial_load |= !fresh_entry;
    }
    changes
}

#[cfg(test)]
mod tests;
