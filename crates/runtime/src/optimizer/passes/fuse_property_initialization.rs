//! Fusion of fresh property writes into one constructor-initialization dispatch.

use hashbrown::HashMap;
use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::PropertyInitializationDescriptor;
use crate::bytecode::chunk::descriptors::PropertyInitializationEntry;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::value::atom::Atom;

struct Candidate {
    allocation: Option<usize>,
    start: usize,
    end: usize,
    object: Register,
    cache: IcSlot,
    entries: Vec<PropertyInitializationEntry>,
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_property_initialization {
        return;
    }

    let eligible: HashMap<Atom, usize> = unit
        .classes
        .iter()
        .filter(|class| {
            class.kind == ClassLikeKind::Class
                && class.is_final
                && class.parent.is_none()
                && class
                    .properties
                    .iter()
                    .filter(|property| !property.is_static)
                    .all(|property| property.default.is_none())
        })
        .map(|class| {
            (
                class.name.clone(),
                class
                    .properties
                    .iter()
                    .filter(|property| !property.is_static)
                    .count(),
            )
        })
        .collect();
    if eligible.is_empty() {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, &eligible, statistics);
    });
}

fn optimize_chunk(
    chunk: &mut Chunk,
    eligible: &HashMap<Atom, usize>,
    statistics: &mut OptimizationStatistics,
) {
    if chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut candidates = Vec::new();
    let mut index = 0;
    while index < chunk.code.len() {
        let Instruction::PropertySetUnchecked {
            object,
            value,
            slot,
            value_mode,
        } = chunk.code[index]
        else {
            index += 1;
            continue;
        };
        if !value_mode.fresh_receiver() {
            index += 1;
            continue;
        }

        let Some((allocation, cache, slot_count, can_delay)) =
            eligible_origin(chunk, &targets, index, object, eligible)
        else {
            index += 1;
            continue;
        };

        let mut entries = vec![PropertyInitializationEntry {
            value,
            slot,
            value_mode,
        }];
        let mut end = index + 1;
        while end < chunk.code.len() && !targets.contains(&end) {
            let Instruction::PropertySetUnchecked {
                object: next_object,
                value,
                slot,
                value_mode,
            } = chunk.code[end]
            else {
                break;
            };
            if next_object != object || !value_mode.fresh_receiver() {
                break;
            }
            entries.push(PropertyInitializationEntry {
                value,
                slot,
                value_mode,
            });
            end += 1;
        }

        let complete = entries.len() == slot_count
            && entries
                .iter()
                .enumerate()
                .all(|(index, entry)| usize::from(entry.slot.index()) == index);
        if entries.len() >= 2 {
            candidates.push(Candidate {
                allocation: (complete && can_delay).then_some(allocation),
                start: index,
                end,
                object,
                cache,
                entries,
            });
            index = end;
        } else {
            index += 1;
        }
    }

    if candidates.is_empty() {
        return;
    }

    let mut remove = vec![false; chunk.code.len()];
    for candidate in candidates {
        let Ok(descriptor) =
            chunk.add_property_initialization_descriptor(PropertyInitializationDescriptor {
                allocates: candidate.allocation.is_some(),
                entries: candidate.entries,
            })
        else {
            continue;
        };
        chunk.code[candidate.start] = Instruction::InitializeProperties {
            object: candidate.object,
            cache: candidate.cache,
            descriptor,
        };
        if let Some(allocation) = candidate.allocation {
            remove[allocation] = true;
        }
        remove[candidate.start + 1..candidate.end].fill(true);
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn eligible_origin(
    chunk: &Chunk,
    targets: &HashSet<usize>,
    start: usize,
    mut register: Register,
    eligible: &HashMap<Atom, usize>,
) -> Option<(usize, IcSlot, usize, bool)> {
    let mut can_delay = true;
    for index in (0..start).rev() {
        if targets.contains(&(index + 1)) {
            return None;
        }
        let instruction = chunk.code[index];
        if branches_or_terminates(instruction) {
            return None;
        }
        let effect = effect_on(chunk, instruction, register);
        if !effect.writes() {
            if effect.reads() || !safe_before_allocation_fusion(instruction) {
                can_delay = false;
            }
            continue;
        }
        match instruction {
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } if destination == register => {
                register = source;
                can_delay = false;
            }
            Instruction::NewStatic { destination, cache } if destination == register => {
                let Some(IcDescriptor::Member { name, .. }) =
                    chunk.ic_descriptors.get(usize::from(cache.index()))
                else {
                    return None;
                };
                return eligible
                    .get(name)
                    .copied()
                    .map(|slot_count| (index, cache, slot_count, can_delay));
            }
            _ => return None,
        }
    }
    None
}

fn safe_before_allocation_fusion(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::LoadConstant { .. }
            | Instruction::LoadNull { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::LoadInt { .. }
    )
}
