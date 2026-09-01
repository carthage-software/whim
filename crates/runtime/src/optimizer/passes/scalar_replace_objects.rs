//! Scalar replacement of non-escaping fresh objects.

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::LiteralKey;
use crate::bytecode::chunk::descriptors::PropertyInitializationDescriptor;
use crate::bytecode::chunk::descriptors::literal_key;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::PropertySlot;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::rewrite::compact;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::is_block_boundary;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::value::atom::Atom;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.scalar_replace_objects {
        return;
    }

    let mut finalizable = unit
        .classes
        .iter()
        .filter(|class| {
            class
                .methods
                .iter()
                .any(|method| method.name.as_bytes() == b"__destruct")
        })
        .map(|class| class.name.clone())
        .collect::<HashSet<_>>();

    loop {
        let before = finalizable.len();
        for class in &unit.classes {
            if class
                .parent
                .as_ref()
                .is_some_and(|parent| finalizable.contains(&parent.name))
            {
                finalizable.insert(class.name.clone());
            }
        }

        if finalizable.len() == before {
            break;
        }
    }

    let classes = unit
        .classes
        .iter()
        .filter(|class| class.type_parameters.is_empty() && !finalizable.contains(&class.name))
        .map(|class| class.name.clone())
        .collect::<Vec<_>>();

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, &classes, statistics);
    });
}

fn optimize_chunk(chunk: &mut Chunk, classes: &[Atom], statistics: &mut OptimizationStatistics) {
    if chunk.code.is_empty() {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    let mut index = 0;
    while index < chunk.code.len() {
        let (cache, candidate) = match chunk.code[index] {
            Instruction::NewStatic { destination, cache } => {
                let first_register = chunk.register_count;
                (
                    cache,
                    candidate(chunk, index, destination, first_register, &targets),
                )
            }
            Instruction::InitializeProperties {
                object,
                cache,
                descriptor,
            } => {
                let descriptor = chunk.property_initialization_descriptor(descriptor);
                let candidate = descriptor
                    .allocates
                    .then(|| initialized_candidate(chunk, index, object, descriptor, &targets))
                    .flatten();
                (cache, candidate)
            }
            _ => {
                index += 1;
                continue;
            }
        };

        let IcDescriptor::Member {
            name,
            type_arguments,
        } = &chunk.ic_descriptors[usize::from(cache.index())]
        else {
            index += 1;
            continue;
        };

        if type_arguments.is_some()
            || !classes
                .iter()
                .any(|class| class.as_bytes() == name.as_bytes())
        {
            index += 1;
            continue;
        }

        let Some(candidate) = candidate else {
            index += 1;
            continue;
        };

        let Ok(slot_count) = u16::try_from(candidate.slots.len()) else {
            index += 1;
            continue;
        };

        let Some(register_count) = chunk.register_count.checked_add(slot_count) else {
            index += 1;
            continue;
        };

        chunk.register_count = register_count;
        remove[index] = true;
        for at in candidate.removals {
            remove[at] = true;
        }
        for (at, replacement) in candidate.replacements {
            chunk.code[at] = replacement;
        }

        statistics.objects_scalar_replaced += 1;
        statistics.instructions_removed += 1;
        index = candidate.end;
    }

    if remove.iter().any(|removed| *removed) {
        compact(chunk, &remove);
        chunk.refresh_runtime_metadata();
    }

    scalar_replace_dicts(chunk, statistics);
}

struct Candidate {
    slots: Vec<(PropertySlot, Register)>,
    removals: Vec<usize>,
    replacements: Vec<(usize, Instruction)>,
    end: usize,
}

fn candidate(
    chunk: &Chunk,
    start: usize,
    object: Register,
    first_register: u16,
    targets: &HashSet<usize>,
) -> Option<Candidate> {
    let mut slots = Vec::new();
    let mut replacements = Vec::new();
    let mut at = start + 1;
    while at < chunk.code.len() {
        if targets.contains(&at) {
            return None;
        }
        let instruction = chunk.code[at];
        match instruction {
            Instruction::PropertySetUnchecked {
                object: receiver,
                value,
                slot,
                ..
            } if receiver == object => {
                let property = slot_register(&mut slots, slot, first_register, true)?;
                replacements.push((
                    at,
                    Instruction::Move {
                        destination: property,
                        source: value,
                    },
                ));
            }
            Instruction::PropertyGetUnchecked {
                destination,
                object: receiver,
                slot,
                ..
            } if receiver == object => {
                let property = slot_register(&mut slots, slot, first_register, false)?;
                replacements.push((
                    at,
                    Instruction::Move {
                        destination,
                        source: property,
                    },
                ));
            }
            _ => {
                let effect = effect_on(chunk, instruction, object);
                if effect.reads() {
                    return None;
                }
                if effect.writes() {
                    break;
                }
            }
        }

        if is_block_boundary(instruction) {
            if !register_is_dead_after(chunk, object, at + 1) {
                return None;
            }
            at += 1;
            break;
        }
        at += 1;
    }

    (!replacements.is_empty()).then_some(Candidate {
        slots,
        removals: Vec::new(),
        replacements,
        end: at,
    })
}

fn initialized_candidate(
    chunk: &Chunk,
    start: usize,
    mut object: Register,
    descriptor: &PropertyInitializationDescriptor,
    targets: &HashSet<usize>,
) -> Option<Candidate> {
    let slots = descriptor
        .entries
        .iter()
        .map(|entry| (entry.slot, entry.value))
        .collect::<Vec<_>>();
    if slots.is_empty()
        || slots
            .iter()
            .any(|(_, register)| !register_is_scalar(chunk, *register))
    {
        return None;
    }

    let mut removals = Vec::new();
    let mut replacements = Vec::new();
    let mut at = start + 1;
    while at < chunk.code.len() {
        if targets.contains(&at) {
            return None;
        }
        let instruction = chunk.code[at];
        if slots
            .iter()
            .any(|(_, register)| effect_on(chunk, instruction, *register).writes())
        {
            return None;
        }
        match instruction {
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } if source == object && register_is_dead_after(chunk, object, at + 1) => {
                removals.push(at);
                object = destination;
            }
            Instruction::PropertyGetUnchecked {
                destination,
                object: receiver,
                slot,
                ..
            } if receiver == object => {
                let (_, source) = slots.iter().find(|(candidate, _)| *candidate == slot)?;
                replacements.push((
                    at,
                    Instruction::Move {
                        destination,
                        source: *source,
                    },
                ));
            }
            Instruction::Clear { target } if target == object => {
                removals.push(at);
                at += 1;
                break;
            }
            _ => {
                let effect = effect_on(chunk, instruction, object);
                if effect.reads() {
                    return None;
                }
                if effect.writes() {
                    break;
                }
            }
        }

        if is_block_boundary(instruction) {
            if !register_is_dead_after(chunk, object, at + 1) {
                return None;
            }
            at += 1;
            break;
        }
        at += 1;
    }

    (!replacements.is_empty()).then_some(Candidate {
        slots: Vec::new(),
        removals,
        replacements,
        end: at,
    })
}

fn register_is_scalar(chunk: &Chunk, register: Register) -> bool {
    let index = register.index();
    index < u64::BITS as u16 && chunk.reference_register_mask & (1u64 << index) == 0
}

fn scalar_replace_dicts(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Instruction::NewDict {
            pair_count,
            destination,
            first_pair,
        } = chunk.code[index]
        else {
            continue;
        };
        let count = usize::from(pair_count.value());
        let Some(entries) = static_scalar_dict_entries(chunk, index, first_pair, count, &targets)
        else {
            continue;
        };
        let Some(candidate) = dict_read_candidate(chunk, index, destination, &entries, &targets)
        else {
            continue;
        };

        remove[index] = true;
        for at in candidate.removals {
            remove[at] = true;
        }
        chunk.code[candidate.read] = Instruction::Move {
            destination: candidate.destination,
            source: candidate.source,
        };
        statistics.objects_scalar_replaced += 1;
    }

    if compact_removed_instructions(chunk, &remove, statistics) == 0 {
        return;
    }

    chunk.refresh_runtime_metadata();
}

struct DictReadCandidate {
    removals: Vec<usize>,
    read: usize,
    destination: Register,
    source: Register,
}

fn static_scalar_dict_entries(
    chunk: &Chunk,
    at: usize,
    first: Register,
    count: usize,
    targets: &HashSet<usize>,
) -> Option<Vec<(LiteralKey, Register)>> {
    let mut keys = HashSet::new();
    let mut entries = Vec::with_capacity(count);
    for pair in 0..count {
        let key = Register::new(first.index().checked_add(u16::try_from(pair * 2).ok()?)?);
        let value = Register::new(key.index().checked_add(1)?);
        if !has_scalar_origin(chunk, at, value, targets) {
            return None;
        }
        let key = static_dict_key(chunk, at, key, targets)?;
        if !keys.insert(key.clone()) {
            return None;
        }
        entries.push((key, value));
    }
    Some(entries)
}

fn has_scalar_origin(
    chunk: &Chunk,
    before: usize,
    mut register: Register,
    targets: &HashSet<usize>,
) -> bool {
    for at in (0..before).rev() {
        if targets.contains(&(at + 1)) || branches_or_terminates(chunk.code[at]) {
            return false;
        }
        if !effect_on(chunk, chunk.code[at], register).writes() {
            continue;
        }
        match chunk.code[at] {
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } if destination == register => register = source,
            Instruction::LoadNull { destination }
            | Instruction::LoadTrue { destination }
            | Instruction::LoadFalse { destination }
            | Instruction::LoadInt { destination, .. }
                if destination == register =>
            {
                return true;
            }
            _ => return false,
        }
    }
    register_is_scalar(chunk, register)
}

fn dict_read_candidate(
    chunk: &Chunk,
    start: usize,
    mut dict: Register,
    entries: &[(LiteralKey, Register)],
    targets: &HashSet<usize>,
) -> Option<DictReadCandidate> {
    let mut removals = Vec::new();
    for at in start + 1..chunk.code.len() {
        if targets.contains(&at) {
            return None;
        }
        let instruction = chunk.code[at];
        match instruction {
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } if source == dict && register_is_dead_after(chunk, dict, at + 1) => {
                removals.push(at);
                dict = destination;
            }
            Instruction::IndexGet {
                destination,
                container,
                index,
            }
            | Instruction::DictIndexGetIntKey {
                destination,
                container,
                index,
                ..
            }
            | Instruction::DictIndexGetStringKey {
                destination,
                container,
                index,
                ..
            } if container == dict && register_is_dead_after(chunk, dict, at + 1) => {
                let key = static_dict_key(chunk, at, index, targets)?;
                let (_, source) = entries.iter().find(|(candidate, _)| *candidate == key)?;
                if chunk.code[start + 1..at]
                    .iter()
                    .any(|instruction| effect_on(chunk, *instruction, *source).writes())
                {
                    return None;
                }
                return Some(DictReadCandidate {
                    removals,
                    read: at,
                    destination,
                    source: *source,
                });
            }
            _ => {
                let effect = effect_on(chunk, instruction, dict);
                if effect.reads() || effect.writes() || branches_or_terminates(instruction) {
                    return None;
                }
            }
        }
    }
    None
}

fn static_dict_key(
    chunk: &Chunk,
    before: usize,
    mut register: Register,
    targets: &HashSet<usize>,
) -> Option<LiteralKey> {
    for at in (0..before).rev() {
        if targets.contains(&(at + 1)) || branches_or_terminates(chunk.code[at]) {
            return None;
        }
        if !effect_on(chunk, chunk.code[at], register).writes() {
            continue;
        }
        match chunk.code[at] {
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } if destination == register => register = source,
            Instruction::LoadInt {
                destination,
                immediate,
            } if destination == register => {
                return Some(LiteralKey::Int(i64::from(immediate.value())));
            }
            Instruction::LoadConstant {
                destination,
                constant,
            } if destination == register => {
                let literal = &chunk.constants[usize::from(constant.index())];
                return matches!(literal, Literal::Int(_) | Literal::String(_))
                    .then(|| literal_key(literal));
            }
            _ => return None,
        }
    }
    None
}

fn slot_register(
    slots: &mut Vec<(PropertySlot, Register)>,
    slot: PropertySlot,
    first_register: u16,
    initialize: bool,
) -> Option<Register> {
    if let Some((_, register)) = slots.iter().find(|(candidate, _)| *candidate == slot) {
        return Some(*register);
    }
    if !initialize {
        return None;
    }
    let offset = u16::try_from(slots.len()).ok()?;
    let register = Register::new(first_register.checked_add(offset)?);
    slots.push((slot, register));
    Some(register)
}
