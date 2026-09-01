//! Fusion of compiler-expanded indexed property updates.

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_property_index_increment || chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for start in 0..chunk.code.len() {
        if remove[start] {
            continue;
        }

        let Some((replacement, end)) =
            fuse_increment(chunk, &targets, start).or_else(|| fuse_remove(chunk, &targets, start))
        else {
            continue;
        };

        chunk.code[start] = replacement;
        remove[start + 1..=end].fill(true);
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn fuse_increment(
    chunk: &Chunk,
    targets: &HashSet<usize>,
    start: usize,
) -> Option<(Instruction, usize)> {
    if start + 4 >= chunk.code.len() {
        return None;
    }

    let Instruction::PropertyGet {
        destination: container,
        object,
        cache,
    } = chunk.code[start]
    else {
        return None;
    };
    let Instruction::IndexGet {
        destination: previous,
        container: indexed_container,
        index,
    } = chunk.code[start + 1]
    else {
        return None;
    };
    let Instruction::AddImmediate {
        destination: incremented,
        source,
        immediate,
    } = chunk.code[start + 2]
    else {
        return None;
    };
    let Instruction::IndexSet {
        container: written_container,
        index: written_index,
        value,
    } = chunk.code[start + 3]
    else {
        return None;
    };
    let Instruction::PropertySet {
        object: written_object,
        value: written_property,
        cache: written_cache,
    } = chunk.code[start + 4]
    else {
        return None;
    };

    if indexed_container != container
        || source != previous
        || immediate.value() != 1
        || written_container != container
        || written_index != index
        || value != incremented
        || written_object != object
        || written_property != container
        || !same_property(chunk, written_cache, cache)
        || (start + 1..=start + 4).any(|index| targets.contains(&index))
        || !register_is_dead_after(chunk, container, start + 5)
        || !register_is_dead_after(chunk, previous, start + 5)
        || !register_is_dead_after(chunk, incremented, start + 5)
    {
        return None;
    }

    Some((
        Instruction::PropertyIndexUpdate {
            object,
            operand: index,
            cache,
            mode: PropertyIndexUpdateMode::Increment,
        },
        start + 4,
    ))
}

fn fuse_remove(
    chunk: &Chunk,
    targets: &HashSet<usize>,
    start: usize,
) -> Option<(Instruction, usize)> {
    if start + 2 >= chunk.code.len() {
        return None;
    }

    let Instruction::PropertyGet {
        destination: container,
        object,
        cache,
    } = chunk.code[start]
    else {
        return None;
    };
    let Instruction::Remove {
        destination,
        container: removed_container,
        key,
    } = chunk.code[start + 1]
    else {
        return None;
    };
    let Instruction::PropertySet {
        object: written_object,
        value,
        cache: written_cache,
    } = chunk.code[start + 2]
    else {
        return None;
    };

    if removed_container != container
        || written_object != object
        || value != container
        || !same_property(chunk, written_cache, cache)
        || targets.contains(&(start + 1))
        || targets.contains(&(start + 2))
        || !register_is_dead_after(chunk, container, start + 3)
        || !register_is_dead_after(chunk, destination, start + 3)
    {
        return None;
    }

    Some((
        Instruction::PropertyIndexUpdate {
            object,
            operand: key,
            cache,
            mode: PropertyIndexUpdateMode::Remove,
        },
        start + 2,
    ))
}

fn same_property(chunk: &Chunk, left: IcSlot, right: IcSlot) -> bool {
    match (
        &chunk.ic_descriptors[usize::from(left.index())],
        &chunk.ic_descriptors[usize::from(right.index())],
    ) {
        (IcDescriptor::Member { name: left, .. }, IcDescriptor::Member { name: right, .. }) => {
            left == right
        }
        _ => false,
    }
}
