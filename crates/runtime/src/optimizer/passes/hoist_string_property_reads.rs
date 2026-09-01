//! Loop-invariant motion for exact string property reads.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::PropertyReadMode;
use crate::bytecode::instruction::operands::PropertySlot;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::rewrite::compact;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::is_block_boundary;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::operands::replace_read_register;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::optimizer::rewrite::splice::can_insert_straight_line_before;
use crate::optimizer::rewrite::splice::insert_straight_line_before;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.licm {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, statistics);
    });
}

fn optimize_chunk(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 4
        || !chunk.catch_table.is_empty()
        || chunk.register_count == u16::MAX
        || !chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyGetUnchecked { .. }))
    {
        return;
    }

    while hoist_one(chunk, statistics) || hoist_interior_one(chunk, statistics) {}
}

fn hoist_one(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) -> bool {
    for header in 0..chunk.code.len() - 1 {
        let Instruction::PropertyGetUnchecked {
            destination,
            object,
            slot,
            value_mode: PropertyReadMode::Clone,
        } = chunk.code[header]
        else {
            continue;
        };
        if destination == object {
            continue;
        }

        let Some((tail, rewritten_back_edge)) = single_back_edge(chunk, header) else {
            continue;
        };
        if has_external_entry(chunk, header, tail)
            || !has_straight_loop_body(chunk, header, tail)
            || !exit_value_is_dead(chunk, header, tail, destination)
        {
            continue;
        }

        let invariant = Register::new(chunk.register_count);
        let mut available = true;
        let mut proven_string = false;
        let mut remove = vec![false; chunk.code.len()];
        let mut replacements = Vec::new();
        let mut valid = true;

        for (index, removed) in remove
            .iter_mut()
            .enumerate()
            .take(tail + 1)
            .skip(header + 1)
        {
            let instruction = chunk.code[index];
            if is_repeated_read(instruction, destination, object, slot) {
                *removed = true;
                available = true;
                continue;
            }
            if invalidates_property(instruction, slot)
                || effect_on(chunk, instruction, object).writes()
            {
                valid = false;
                break;
            }

            let effect = effect_on(chunk, instruction, destination);
            if available && effect.reads() {
                proven_string |= reads_as_string(instruction, destination);
                let Some(replacement) = replace_read_register(instruction, destination, invariant)
                else {
                    valid = false;
                    break;
                };
                replacements.push((index, replacement));
            }
            if effect.writes() {
                available = false;
            }
        }

        if !valid || !proven_string {
            continue;
        }

        chunk.code[header] = Instruction::PropertyGetUnchecked {
            destination: invariant,
            object,
            slot,
            value_mode: PropertyReadMode::Clone,
        };
        chunk.register_count += 1;
        for (index, replacement) in replacements {
            chunk.code[index] = replacement;
        }
        chunk.code[tail] = rewritten_back_edge;

        let removed = compact_removed_instructions(chunk, &remove, statistics);
        statistics.common_subexpressions_eliminated += removed;

        return true;
    }

    false
}

fn hoist_interior_one(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) -> bool {
    for header in 0..chunk.code.len() - 1 {
        let Some((tail, _)) = single_back_edge(chunk, header) else {
            continue;
        };
        if has_external_entry(chunk, header, tail)
            || !has_forward_loop_body(chunk, header, tail)
            || !can_insert_straight_line_before(chunk, header)
        {
            continue;
        }

        for candidate in header..=tail {
            let Instruction::PropertyGetUnchecked { object, slot, .. } = chunk.code[candidate]
            else {
                continue;
            };
            if let Some(rewrite) = interior_rewrite(chunk, header, tail, object, slot) {
                let invariant = Register::new(chunk.register_count);
                let span = chunk.spans[candidate];
                chunk.register_count += 1;

                for (index, instruction) in rewrite.replacements {
                    chunk.code[index] = instruction;
                }
                compact(chunk, &rewrite.remove);
                let inserted = insert_straight_line_before(
                    chunk,
                    header,
                    &[(
                        Instruction::PropertyGetUnchecked {
                            destination: invariant,
                            object,
                            slot,
                            value_mode: PropertyReadMode::Clone,
                        },
                        span,
                    )],
                );
                assert!(inserted, "validated loop preheader insertion must succeed");

                statistics.instructions_removed += rewrite.removed - 1;
                statistics.common_subexpressions_eliminated += rewrite.removed;
                return true;
            }
        }
    }

    false
}

struct InteriorRewrite {
    remove: Vec<bool>,
    replacements: Vec<(usize, Instruction)>,
    removed: usize,
}

fn interior_rewrite(
    chunk: &Chunk,
    header: usize,
    tail: usize,
    object: Register,
    slot: PropertySlot,
) -> Option<InteriorRewrite> {
    let invariant = Register::new(chunk.register_count);
    let mut definitions = Vec::new();
    for index in header..=tail {
        let Instruction::PropertyGetUnchecked {
            destination,
            object: read_object,
            slot: read_slot,
            value_mode: PropertyReadMode::Clone,
        } = chunk.code[index]
        else {
            continue;
        };
        if read_object == object && read_slot == slot {
            if destination == object {
                return None;
            }
            definitions.push((index, destination));
        }
    }
    if definitions.is_empty()
        || destinations_depend_on_previous_iteration(chunk, header, &definitions)
    {
        return None;
    }

    let mut aliases = vec![false; usize::from(chunk.register_count)];
    let mut remove = vec![false; chunk.code.len()];
    let mut replacements = Vec::new();
    let mut proven_string = false;

    for (index, removed) in remove.iter_mut().enumerate().take(tail + 1).skip(header) {
        let instruction = chunk.code[index];
        if effect_on(chunk, instruction, object).writes() || invalidates_property(instruction, slot)
        {
            return None;
        }

        if let Instruction::PropertyGetUnchecked {
            destination,
            object: read_object,
            slot: read_slot,
            value_mode: PropertyReadMode::Clone,
        } = instruction
            && read_object == object
            && read_slot == slot
        {
            *removed = true;
            aliases[usize::from(destination.index())] = true;
            continue;
        }

        let mut replacement = instruction;
        for (register_index, active) in aliases.iter_mut().enumerate() {
            if !*active {
                continue;
            }
            let register = Register::new(register_index as u16);
            let effect = effect_on(chunk, instruction, register);
            if effect.reads() {
                proven_string |= reads_as_string(instruction, register);
                let rewritten = replace_read_register(replacement, register, invariant)?;
                replacement = rewritten;
            }
            if effect.writes() {
                *active = false;
            }
        }
        if replacement != instruction {
            replacements.push((index, replacement));
        }

        let mut edges = Vec::new();
        successors(chunk, index, &mut edges);
        for target in edges {
            if target > index + 1
                && target <= tail
                && !forward_edge_preserves_aliases(chunk, index, target, object, slot, &aliases)
            {
                return None;
            }
            if (target >= header && target <= tail) || target == chunk.code.len() {
                continue;
            }
            for (register_index, active) in aliases.iter().enumerate() {
                if *active
                    && !register_is_dead_after(chunk, Register::new(register_index as u16), target)
                {
                    return None;
                }
            }
        }
    }

    if !proven_string {
        return None;
    }

    Some(InteriorRewrite {
        removed: definitions.len(),
        remove,
        replacements,
    })
}

fn forward_edge_preserves_aliases(
    chunk: &Chunk,
    source: usize,
    target: usize,
    object: Register,
    slot: PropertySlot,
    aliases: &[bool],
) -> bool {
    let mut skipped_aliases = aliases.to_vec();
    for skipped in source + 1..target {
        let instruction = chunk.code[skipped];
        if let Instruction::PropertyGetUnchecked {
            destination,
            object: read_object,
            slot: read_slot,
            value_mode: PropertyReadMode::Clone,
        } = instruction
            && read_object == object
            && read_slot == slot
        {
            skipped_aliases[usize::from(destination.index())] = true;
            continue;
        }
        for (register_index, active) in skipped_aliases.iter_mut().enumerate() {
            if effect_on(chunk, instruction, Register::new(register_index as u16)).writes() {
                *active = false;
            }
        }
    }

    skipped_aliases == aliases
}

fn destinations_depend_on_previous_iteration(
    chunk: &Chunk,
    header: usize,
    definitions: &[(usize, Register)],
) -> bool {
    let mut first_definitions = Vec::new();
    for &(index, destination) in definitions {
        if first_definitions
            .iter()
            .any(|(_, existing)| *existing == destination)
        {
            continue;
        }
        first_definitions.push((index, destination));
    }

    first_definitions.into_iter().any(|(definition, register)| {
        let mut initialized = false;
        for index in header..definition {
            let effect = effect_on(chunk, chunk.code[index], register);
            if effect.reads() && !initialized {
                return true;
            }
            initialized |= effect.writes();
        }
        false
    })
}

fn single_back_edge(chunk: &Chunk, header: usize) -> Option<(usize, Instruction)> {
    let mut found = None;
    let mut edges = Vec::new();
    for source in header + 1..chunk.code.len() {
        edges.clear();
        successors(chunk, source, &mut edges);
        if !edges.contains(&header) {
            continue;
        }
        if found.is_some() {
            return None;
        }

        found = Some((
            source,
            retarget_back_edge(chunk.code[source], source, header)?,
        ));
    }

    found
}

fn retarget_back_edge(
    instruction: Instruction,
    source: usize,
    header: usize,
) -> Option<Instruction> {
    match instruction {
        Instruction::Jump { offset } if relative_target(source, offset.offset()) == header => {
            Some(Instruction::Jump {
                offset: JumpOffset::new(offset.offset() + 1),
            })
        }
        Instruction::NumericRegionJump { offset }
            if relative_target(source, offset.offset()) == header =>
        {
            Some(Instruction::NumericRegionJump {
                offset: JumpOffset::new(offset.offset() + 1),
            })
        }
        Instruction::IncrementJump {
            target,
            immediate,
            offset,
        } if relative_target(source, i32::from(offset.offset())) == header => {
            Some(Instruction::IncrementJump {
                target,
                immediate,
                offset: ShortJumpOffset::new(offset.offset() + 1),
            })
        }
        Instruction::CounterLoop {
            comparison,
            counter,
            limit,
            offset,
        } if relative_target(source, i32::from(offset.offset())) == header => {
            Some(Instruction::CounterLoop {
                comparison,
                counter,
                limit,
                offset: ShortJumpOffset::new(offset.offset() + 1),
            })
        }
        Instruction::IntCounterLoop {
            comparison,
            counter,
            limit,
            offset,
        } if relative_target(source, i32::from(offset.offset())) == header => {
            Some(Instruction::IntCounterLoop {
                comparison,
                counter,
                limit,
                offset: ShortJumpOffset::new(offset.offset() + 1),
            })
        }
        Instruction::IntStepLoop { descriptor, offset }
            if relative_target(source, i32::from(offset.offset())) == header =>
        {
            Some(Instruction::IntStepLoop {
                descriptor,
                offset: ShortJumpOffset::new(offset.offset() + 1),
            })
        }
        _ => None,
    }
}

fn has_external_entry(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut edges = Vec::new();
    for source in 0..chunk.code.len() {
        if source >= header && source <= tail {
            continue;
        }

        edges.clear();
        successors(chunk, source, &mut edges);
        for &target in &edges {
            if target < header || target > tail {
                continue;
            }
            let initial_fallthrough =
                source + 1 == header && target == header && !is_block_boundary(chunk.code[source]);
            if !initial_fallthrough {
                return true;
            }
        }
    }

    false
}

fn has_straight_loop_body(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut edges = Vec::new();
    for source in header..=tail {
        edges.clear();
        successors(chunk, source, &mut edges);
        for &target in &edges {
            if target >= header
                && target <= tail
                && target != source + 1
                && !(source == tail && target == header)
            {
                return false;
            }
        }
    }

    true
}

fn has_forward_loop_body(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut edges = Vec::new();
    for source in header..=tail {
        edges.clear();
        successors(chunk, source, &mut edges);
        for &target in &edges {
            if target >= header && target <= source && !(source == tail && target == header) {
                return false;
            }
        }
    }

    true
}

fn exit_value_is_dead(chunk: &Chunk, header: usize, tail: usize, destination: Register) -> bool {
    let mut edges = Vec::new();
    for source in header..=tail {
        edges.clear();
        successors(chunk, source, &mut edges);
        for &target in &edges {
            if (target < header || target > tail)
                && target != chunk.code.len()
                && !register_is_dead_after(chunk, destination, target)
            {
                return false;
            }
        }
    }

    true
}

fn is_repeated_read(
    instruction: Instruction,
    destination: Register,
    object: Register,
    slot: PropertySlot,
) -> bool {
    matches!(
        instruction,
        Instruction::PropertyGetUnchecked {
            destination: repeated_destination,
            object: repeated_object,
            slot: repeated_slot,
            value_mode: PropertyReadMode::Clone,
        } if repeated_destination == destination
            && repeated_object == object
            && repeated_slot == slot
    )
}

fn reads_as_string(instruction: Instruction, register: Register) -> bool {
    matches!(
        instruction,
        Instruction::StringLength { source, .. } if source == register
    ) || matches!(
        instruction,
        Instruction::StringIndexGet { container, .. }
            | Instruction::StringByteJumpUnlessEqual { container, .. }
            | Instruction::StringByteJumpUnlessNotEqual { container, .. }
            | Instruction::StringByteEqual { container, .. }
            | Instruction::StringByteNotEqual { container, .. }
            | Instruction::StringByteLessThan { container, .. }
            | Instruction::StringByteLessThanOrEqual { container, .. }
            | Instruction::StringByteGreaterThan { container, .. }
            | Instruction::StringByteGreaterThanOrEqual { container, .. }
            if container == register
    ) || matches!(
        instruction,
        Instruction::StringJumpUnless { left, right, .. }
            if left == register || right == register
    )
}

fn invalidates_property(instruction: Instruction, slot: PropertySlot) -> bool {
    matches!(
        instruction,
        Instruction::PropertySet { .. }
            | Instruction::PropertyInitRaw { .. }
            | Instruction::PropertyIndexUpdate { .. }
            | Instruction::PropertyRemove { .. }
            | Instruction::PropertyStep { .. }
            | Instruction::PropertyAdd { .. }
            | Instruction::PropertyFillIntRange { .. }
            | Instruction::CallValue { .. }
            | Instruction::CallNamed { .. }
            | Instruction::CallMethod { .. }
            | Instruction::CallStatic { .. }
            | Instruction::CallWithNames { .. }
            | Instruction::CallValueDiscarded { .. }
            | Instruction::CallNamedDiscarded { .. }
            | Instruction::CallMethodDiscarded { .. }
            | Instruction::CallStaticDiscarded { .. }
            | Instruction::CallWithNamesDiscarded { .. }
            | Instruction::CallValueUnchecked { .. }
            | Instruction::CallNamedUnchecked { .. }
            | Instruction::CallMethodUnchecked { .. }
            | Instruction::CallMethodDirect { .. }
            | Instruction::CallSelfUnchecked { .. }
            | Instruction::CallNamedConstantUnchecked { .. }
            | Instruction::NewStatic { .. }
            | Instruction::NewDynamic { .. }
            | Instruction::NewTyped { .. }
            | Instruction::CloneObject { .. }
            | Instruction::StaticPropertyGet { .. }
            | Instruction::ConstantGet { .. }
            | Instruction::ClassConstantGet { .. }
            | Instruction::Is { .. }
            | Instruction::AsCheck { .. }
            | Instruction::AsOrNull { .. }
            | Instruction::ForeachInit { .. }
            | Instruction::ForeachNext { .. }
            | Instruction::Require { .. }
            | Instruction::DrainFinalizers
    ) || matches!(instruction, Instruction::PropertySetUnchecked { .. })
        || matches!(
            instruction,
            Instruction::PropertyIndexUpdateUnchecked {
                slot: written_slot,
                ..
            } | Instruction::PropertyRemoveUnchecked {
                slot: written_slot,
                ..
            } | Instruction::PropertyStepUnchecked {
                slot: written_slot,
                ..
            } | Instruction::PropertyAddUnchecked {
                slot: written_slot,
                ..
            } if written_slot == slot
        )
}
