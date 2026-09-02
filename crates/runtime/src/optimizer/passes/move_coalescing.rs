//! Coalescing of compiler-generated temporary result registers.

use hashbrown::HashSet;

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::cfg::has_shared_switch_table;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::register_is_dead_after_removals;
use crate::optimizer::operands::for_each_read_register;
use crate::optimizer::operands::for_each_write_register;
use crate::optimizer::operands::operands;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::optimizer::rewrite::destination::with_destination;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, configuration, statistics);
    });
}

fn move_operands(instruction: Instruction) -> Option<(Register, Register)> {
    match instruction {
        Instruction::Move {
            destination,
            source,
        }
        | Instruction::MoveOwned {
            destination,
            source,
        } => Some((destination, source)),
        _ => None,
    }
}

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if chunk.code.len() < 2 || has_shared_switch_table(chunk) {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];

    if configuration.move_coalescing {
        let predecessors = predecessor_counts(chunk);
        coalesce_joined_producers(chunk, &predecessors, &mut remove);
        coalesce_foreach_assignments(chunk, &targets, &mut remove);
        coalesce_consumed_property_moves(chunk, &targets, &mut remove);
    }

    let straight_line_dead_moves =
        if configuration.move_coalescing && targets.is_empty() && chunk.catch_table.is_empty() {
            straight_line_dead_moves(chunk, &remove)
        } else {
            None
        };

    for index in 0..chunk.code.len() {
        if remove[index] {
            continue;
        }
        let Some((destination, source)) = move_operands(chunk.code[index]) else {
            continue;
        };

        if configuration.self_move && destination == source && !targets.contains(&index) {
            remove[index] = true;
            continue;
        }

        if !configuration.move_coalescing
            || index == 0
            || targets.contains(&index)
            || remove[index - 1]
            || local_may_own_reference(chunk, source)
        {
            continue;
        }
        let source_is_dead = match &straight_line_dead_moves {
            Some(dead_moves) => dead_moves[index],
            None => register_is_dead_after_removals(chunk, source, index + 1, &remove),
        };
        if !source_is_dead {
            continue;
        }

        if let Some(rewritten) = with_destination(chunk.code[index - 1], destination, source) {
            if source.index() < REFERENCE_REGISTER_LIMIT
                && destination.index() < REFERENCE_REGISTER_LIMIT
                && chunk.reference_register_mask & (1u64 << source.index()) != 0
            {
                chunk.reference_register_mask |= 1u64 << destination.index();
            }

            chunk.code[index - 1] = rewritten;
            remove[index] = true;
        }
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn straight_line_dead_moves(chunk: &Chunk, remove: &[bool]) -> Option<Vec<bool>> {
    let mut dead_moves = vec![false; chunk.code.len()];
    let mut live = vec![false; usize::from(chunk.register_count)];
    for index in (0..chunk.code.len()).rev() {
        if remove[index] {
            continue;
        }

        let instruction = chunk.code[index];
        operands(instruction.kind())?;
        if branches_or_terminates(instruction) {
            live.fill(false);
        }
        if let Some((_, source)) = move_operands(instruction) {
            let source = usize::from(source.index());
            if source >= live.len() {
                live.resize(source + 1, false);
            }
            dead_moves[index] = !live[source];
        }
        for_each_write_register(instruction, |register| {
            let register = usize::from(register.index());
            if register >= live.len() {
                live.resize(register + 1, false);
            }
            live[register] = false;
        });
        for_each_read_register(instruction, |register| {
            let register = usize::from(register.index());
            if register >= live.len() {
                live.resize(register + 1, false);
            }
            live[register] = true;
        });
    }

    Some(dead_moves)
}

fn coalesce_consumed_property_moves(
    chunk: &mut Chunk,
    targets: &HashSet<usize>,
    remove: &mut [bool],
) {
    for index in 0..chunk.code.len().saturating_sub(1) {
        if remove[index] || remove[index + 1] || targets.contains(&(index + 1)) {
            continue;
        }
        let Instruction::MoveOwned {
            destination,
            source,
        } = chunk.code[index]
        else {
            continue;
        };
        let Instruction::PropertySetUnchecked {
            object,
            value,
            slot,
            value_mode,
        } = chunk.code[index + 1]
        else {
            continue;
        };
        if value != destination
            || local_may_own_reference(chunk, source)
            || !value_mode.moves()
            || !register_is_dead_after_removals(chunk, destination, index + 2, remove)
        {
            continue;
        }

        chunk.code[index + 1] = Instruction::PropertySetUnchecked {
            object,
            value: source,
            slot,
            value_mode,
        };
        remove[index] = true;
    }
}

fn coalesce_foreach_assignments(chunk: &mut Chunk, targets: &HashSet<usize>, remove: &mut [bool]) {
    for index in 0..chunk.code.len().saturating_sub(2) {
        let Some((mut key_destination, mut value_destination)) =
            foreach_destinations(chunk.code[index])
        else {
            continue;
        };
        if !matches!(chunk.code[index + 1], Instruction::Jump { .. }) {
            continue;
        }

        let mut assignment = index + 2;
        if key_destination != Register::NONE
            && let Some(destination) =
                assignment_destination(chunk, assignment, key_destination, targets, remove)
        {
            key_destination = destination;
            remove[assignment] = true;
            assignment += 1;
        }
        if let Some(destination) =
            assignment_destination(chunk, assignment, value_destination, targets, remove)
        {
            value_destination = destination;
            remove[assignment] = true;
        }

        chunk.code[index] =
            with_foreach_destinations(chunk.code[index], key_destination, value_destination);
    }
}

fn foreach_destinations(instruction: Instruction) -> Option<(Register, Register)> {
    match instruction {
        Instruction::ForeachNext {
            key_destination,
            value_destination,
            ..
        }
        | Instruction::VecForeachNext {
            key_destination,
            value_destination,
            ..
        }
        | Instruction::DictForeachNext {
            key_destination,
            value_destination,
            ..
        } => Some((key_destination, value_destination)),
        _ => None,
    }
}

fn assignment_destination(
    chunk: &Chunk,
    index: usize,
    source: Register,
    targets: &HashSet<usize>,
    remove: &[bool],
) -> Option<Register> {
    if index >= chunk.code.len() || targets.contains(&index) || remove[index] {
        return None;
    }
    let (destination, actual_source) = move_operands(chunk.code[index])?;
    if actual_source != source || !register_is_dead_after_removals(chunk, source, index + 1, remove)
    {
        return None;
    }

    Some(destination)
}

fn with_foreach_destinations(
    instruction: Instruction,
    key_destination: Register,
    value_destination: Register,
) -> Instruction {
    match instruction {
        Instruction::ForeachNext { iterator, .. } => Instruction::ForeachNext {
            iterator,
            key_destination,
            value_destination,
        },
        Instruction::VecForeachNext {
            iterator,
            value_mode,
            ..
        } => Instruction::VecForeachNext {
            iterator,
            key_destination,
            value_destination,
            value_mode,
        },
        Instruction::DictForeachNext {
            iterator,
            value_mode,
            ..
        } => Instruction::DictForeachNext {
            iterator,
            key_destination,
            value_destination,
            value_mode,
        },
        _ => instruction,
    }
}

/// Retargets both producers of the canonical two-arm join into the move's
/// final destination. No observable instruction lies between either
/// producer and the join, so the temporary never needs to materialize.
fn coalesce_joined_producers(chunk: &mut Chunk, predecessors: &[usize], remove: &mut [bool]) {
    for join in 4..remove.len() {
        if remove[join] {
            continue;
        }
        let Some((destination, source)) = move_operands(chunk.code[join]) else {
            continue;
        };
        if local_may_own_reference(chunk, source)
            || !register_is_dead_after_removals(chunk, source, join + 1, remove)
            || predecessors[join] != 2
        {
            continue;
        }

        let first = join - 4;
        let second = join - 2;
        let Some(first_replacement) = with_destination(chunk.code[first], destination, source)
        else {
            continue;
        };
        let Some(second_replacement) = with_destination(chunk.code[second], destination, source)
        else {
            continue;
        };

        if !matches!(chunk.code[first + 1], Instruction::Jump { .. })
            || !matches!(chunk.code[second + 1], Instruction::Jump { .. })
        {
            continue;
        }

        let mut edges = Vec::new();
        successors(chunk, first - 1, &mut edges);
        if edges.len() != 2 || !edges.contains(&first) || !edges.contains(&second) {
            continue;
        }
        edges.clear();
        successors(chunk, first + 1, &mut edges);
        if edges.as_slice() != [join] {
            continue;
        }
        edges.clear();
        successors(chunk, second + 1, &mut edges);
        if edges.as_slice() != [join] {
            continue;
        }

        chunk.code[first] = first_replacement;
        chunk.code[second] = second_replacement;
        remove[join] = true;
    }
}

fn predecessor_counts(chunk: &Chunk) -> Vec<usize> {
    let mut counts = vec![0; chunk.code.len()];
    let mut edges = Vec::new();
    for index in 0..chunk.code.len() {
        edges.clear();
        successors(chunk, index, &mut edges);
        edges.sort_unstable();
        edges.dedup();
        for target in edges.iter().copied() {
            if let Some(count) = counts.get_mut(target) {
                *count += 1;
            }
        }
    }

    counts
}

fn local_may_own_reference(chunk: &Chunk, register: Register) -> bool {
    register.index() < chunk.local_register_count
        && (register.index() >= REFERENCE_REGISTER_LIMIT
            || chunk.reference_register_mask & (1u64 << register.index()) != 0)
}

#[cfg(test)]
mod tests {
    use whim_span::Span;

    use crate::bytecode::chunk::Chunk;
    use crate::bytecode::instruction::Instruction;
    use crate::bytecode::instruction::operands::ArrayValueMode;
    use crate::bytecode::instruction::operands::JumpOffset;
    use crate::bytecode::instruction::operands::Register;
    use crate::optimizer::OptimizationConfiguration;
    use crate::optimizer::OptimizationStatistics;
    use crate::optimizer::passes::move_coalescing::optimize_chunk;

    #[test]
    fn coalesces_a_specialized_array_read_into_its_local() {
        let mut chunk = Chunk::new();
        chunk.emit(
            Instruction::VecIndexGet {
                destination: Register::new(2),
                container: Register::new(0),
                index: Register::new(1),
                value_mode: ArrayValueMode::Float,
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::MoveOwned {
                destination: Register::new(3),
                source: Register::new(2),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::Return {
                source: Register::new(3),
            },
            Span::zero(),
        );

        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.instructions_removed, 1);
        assert_eq!(
            chunk.code,
            vec![
                Instruction::VecIndexGet {
                    destination: Register::new(3),
                    container: Register::new(0),
                    index: Register::new(1),
                    value_mode: ArrayValueMode::Float,
                },
                Instruction::Return {
                    source: Register::new(3),
                },
            ]
        );
    }

    #[test]
    fn coalesces_many_straight_line_moves() {
        let mut chunk = Chunk::new();
        chunk.register_count = 2;
        let destination = Register::new(0);
        let temporary = Register::new(1);
        for _ in 0..32 {
            chunk.emit(
                Instruction::LoadTrue {
                    destination: temporary,
                },
                Span::zero(),
            );
            chunk.emit(
                Instruction::Move {
                    destination,
                    source: temporary,
                },
                Span::zero(),
            );
        }
        chunk.emit(Instruction::ReturnNull, Span::zero());

        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.instructions_removed, 32);
        assert_eq!(chunk.code.len(), 33);
        assert!(chunk.code[..32].iter().all(|instruction| matches!(
            instruction,
            Instruction::LoadTrue {
                destination: register,
            } if *register == destination
        )));
    }

    #[test]
    fn keeps_a_generic_array_read_before_an_owned_move() {
        let mut chunk = Chunk::new();
        chunk.emit(
            Instruction::VecIndexGet {
                destination: Register::new(2),
                container: Register::new(0),
                index: Register::new(1),
                value_mode: ArrayValueMode::Generic,
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::MoveOwned {
                destination: Register::new(3),
                source: Register::new(2),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::Return {
                source: Register::new(3),
            },
            Span::zero(),
        );

        let before = chunk.code.clone();
        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.instructions_removed, 0);
        assert_eq!(chunk.code, before);
    }

    #[test]
    fn keeps_a_reference_local_before_its_last_copy() {
        let mut chunk = Chunk::new();
        chunk.local_register_count = 1;
        chunk.reference_register_mask = 1;
        chunk.emit(
            Instruction::Move {
                destination: Register::new(0),
                source: Register::new(1),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::Move {
                destination: Register::new(2),
                source: Register::new(0),
            },
            Span::zero(),
        );
        chunk.emit(Instruction::ReturnNull, Span::zero());

        let before = chunk.code.clone();
        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.instructions_removed, 0);
        assert_eq!(chunk.code, before);
    }

    #[test]
    fn keeps_a_join_with_more_than_two_predecessors() {
        let mut chunk = Chunk::new();
        let temporary = Register::new(2);
        chunk.emit(
            Instruction::JumpIfFalse {
                condition: Register::new(0),
                offset: JumpOffset::new(3),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::LoadTrue {
                destination: temporary,
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(6),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::JumpIfFalse {
                condition: Register::new(1),
                offset: JumpOffset::new(3),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::LoadFalse {
                destination: temporary,
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(3),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::LoadTrue {
                destination: temporary,
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(1),
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::Move {
                destination: Register::new(3),
                source: temporary,
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::ReturnScalarUnchecked {
                source: Register::new(3),
            },
            Span::zero(),
        );

        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert!(chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::Move {
                    destination,
                    source,
                } if *destination == Register::new(3) && *source == temporary
            )
        }));
    }
}
