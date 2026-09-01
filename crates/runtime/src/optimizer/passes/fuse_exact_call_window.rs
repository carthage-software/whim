//! Direct exact-method calls from contiguous caller registers.

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.elide_parameter_checks && !configuration.move_coalescing {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, configuration, statistics);
    });
}

fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];

    for index in 0..chunk.code.len() {
        if configuration.move_coalescing
            && let Some((start, rewritten)) =
                named_call_without_argument_copies(chunk, index, &targets, &remove)
        {
            remove[start..index].fill(true);
            chunk.code[index] = rewritten;
            continue;
        }

        if configuration.move_coalescing
            && let Some((move_index, rewritten)) =
                value_call_without_callee_copy(chunk, index, &targets, &remove)
        {
            remove[move_index] = true;
            chunk.code[index] = rewritten;
            continue;
        }

        if !configuration.elide_parameter_checks {
            continue;
        }

        let Instruction::CallMethodUnchecked {
            argument_count,
            destination,
            first_argument,
            cache,
        } = chunk.code[index]
        else {
            continue;
        };

        let count = usize::from(argument_count.value());
        let Some(start) = index.checked_sub(count) else {
            continue;
        };

        if count == 0 || targets.contains(&index) {
            continue;
        }

        let Some(first_source) =
            direct_source_window(chunk, start, count, first_argument, &targets, &remove)
        else {
            continue;
        };

        if windows_overlap(first_source, first_argument, count) {
            continue;
        }

        remove[start..index].fill(true);
        chunk.code[index] = Instruction::CallMethodDirect {
            argument_count,
            destination,
            first_argument: first_source,
            cache,
        };
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn named_call_without_argument_copies(
    chunk: &Chunk,
    index: usize,
    targets: &HashSet<usize>,
    remove: &[bool],
) -> Option<(usize, Instruction)> {
    let Instruction::CallNamedUnchecked {
        argument_count,
        destination,
        first_argument,
        cache,
    } = chunk.code[index]
    else {
        return None;
    };
    let count = usize::from(argument_count.value());
    let start = index.checked_sub(count)?;
    if count == 0 || (start..=index).any(|position| targets.contains(&position) || remove[position])
    {
        return None;
    }

    let Instruction::MoveOwned {
        destination: first_destination,
        source: first_source,
    } = chunk.code[start]
    else {
        return None;
    };
    if first_destination != first_argument || windows_overlap(first_source, first_argument, count) {
        return None;
    }

    for offset in 0..count {
        let Instruction::MoveOwned {
            destination,
            source,
        } = chunk.code[start + offset]
        else {
            return None;
        };
        if destination.index() as usize != first_argument.index() as usize + offset
            || source.index() as usize != first_source.index() as usize + offset
            || !register_is_dead_after(chunk, source, index + 1)
        {
            return None;
        }
    }

    Some((
        start,
        Instruction::CallNamedUnchecked {
            argument_count,
            destination,
            first_argument: first_source,
            cache,
        },
    ))
}

fn value_call_without_callee_copy(
    chunk: &Chunk,
    index: usize,
    targets: &HashSet<usize>,
    remove: &[bool],
) -> Option<(usize, Instruction)> {
    let (count, destination, callee, first_argument, unchecked, discarded) = match chunk.code[index]
    {
        Instruction::CallValue {
            argument_count,
            destination,
            callee,
            first_argument,
        } => (
            usize::from(argument_count.value()),
            destination,
            callee,
            first_argument,
            false,
            false,
        ),
        Instruction::CallValueUnchecked {
            argument_count,
            destination,
            callee,
            first_argument,
        } => (
            usize::from(argument_count.value()),
            destination,
            callee,
            first_argument,
            true,
            false,
        ),
        Instruction::CallValueDiscarded {
            argument_count,
            destination,
            callee,
            first_argument,
        } => (
            usize::from(argument_count.value()),
            destination,
            callee,
            first_argument,
            false,
            true,
        ),
        _ => return None,
    };
    let move_index = index.checked_sub(count + 1)?;
    if (move_index..=index).any(|position| targets.contains(&position) || remove[position]) {
        return None;
    }

    let Instruction::Move {
        destination: temporary,
        source,
    } = chunk.code[move_index]
    else {
        return None;
    };
    if temporary != callee
        || !register_is_dead_after(chunk, callee, index + 1)
        || register_in_window(source, first_argument, count)
    {
        return None;
    }

    for offset in 0..count {
        let Instruction::Move {
            destination,
            source: argument_source,
        } = chunk.code[move_index + 1 + offset]
        else {
            return None;
        };
        if destination.index() as usize != first_argument.index() as usize + offset
            || argument_source == callee
        {
            return None;
        }
    }

    let (Instruction::CallValue { argument_count, .. }
    | Instruction::CallValueUnchecked { argument_count, .. }
    | Instruction::CallValueDiscarded { argument_count, .. }) = chunk.code[index]
    else {
        return None;
    };
    let rewritten = if discarded {
        Instruction::CallValueDiscarded {
            argument_count,
            destination,
            callee: source,
            first_argument,
        }
    } else if unchecked {
        Instruction::CallValueUnchecked {
            argument_count,
            destination,
            callee: source,
            first_argument,
        }
    } else {
        Instruction::CallValue {
            argument_count,
            destination,
            callee: source,
            first_argument,
        }
    };

    Some((move_index, rewritten))
}

fn register_in_window(register: Register, first: Register, count: usize) -> bool {
    let register = register.index() as usize;
    let first = first.index() as usize;
    register >= first && register < first + count
}

fn direct_source_window(
    chunk: &Chunk,
    start: usize,
    count: usize,
    first_destination: Register,
    targets: &HashSet<usize>,
    remove: &[bool],
) -> Option<Register> {
    let Instruction::Move {
        destination,
        source: first_source,
    } = chunk.code[start]
    else {
        return None;
    };

    if destination != first_destination {
        return None;
    }

    for offset in 0..count {
        let index = start + offset;
        if (index != start && targets.contains(&index)) || remove[index] {
            return None;
        }

        let Instruction::Move {
            destination,
            source,
        } = chunk.code[index]
        else {
            return None;
        };

        if destination.index() as usize != first_destination.index() as usize + offset
            || source.index() as usize != first_source.index() as usize + offset
        {
            return None;
        }
    }

    Some(first_source)
}

fn windows_overlap(left: Register, right: Register, count: usize) -> bool {
    let left = left.index() as usize;
    let right = right.index() as usize;
    left < right + count && right < left + count
}

#[cfg(test)]
mod tests {
    use whim_span::Span;

    use crate::bytecode::chunk::Chunk;
    use crate::bytecode::instruction::Instruction;
    use crate::bytecode::instruction::operands::Count;
    use crate::bytecode::instruction::operands::IcSlot;
    use crate::bytecode::instruction::operands::Register;
    use crate::optimizer::OptimizationConfiguration;
    use crate::optimizer::OptimizationStatistics;
    use crate::optimizer::passes::fuse_exact_call_window::optimize_chunk;

    fn emit(chunk: &mut Chunk, instruction: Instruction) {
        chunk.emit(instruction, Span::zero());
    }

    #[test]
    fn forwards_a_dead_owned_argument_window_into_an_exact_call() {
        let mut chunk = Chunk::new();
        emit(
            &mut chunk,
            Instruction::MoveOwned {
                destination: Register::new(5),
                source: Register::new(2),
            },
        );
        emit(
            &mut chunk,
            Instruction::MoveOwned {
                destination: Register::new(6),
                source: Register::new(3),
            },
        );
        emit(
            &mut chunk,
            Instruction::CallNamedUnchecked {
                argument_count: Count::new(2),
                destination: Register::new(4),
                first_argument: Register::new(5),
                cache: IcSlot::new(0),
            },
        );
        emit(
            &mut chunk,
            Instruction::ReturnScalarUnchecked {
                source: Register::new(4),
            },
        );

        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.instructions_removed, 2);
        assert!(matches!(
            chunk.code[0],
            Instruction::CallNamedUnchecked {
                first_argument,
                ..
            } if first_argument == Register::new(2)
        ));
    }

    #[test]
    fn keeps_an_owned_argument_window_when_a_source_remains_live() {
        let mut chunk = Chunk::new();
        emit(
            &mut chunk,
            Instruction::MoveOwned {
                destination: Register::new(5),
                source: Register::new(2),
            },
        );
        emit(
            &mut chunk,
            Instruction::MoveOwned {
                destination: Register::new(6),
                source: Register::new(3),
            },
        );
        emit(
            &mut chunk,
            Instruction::CallNamedUnchecked {
                argument_count: Count::new(2),
                destination: Register::new(4),
                first_argument: Register::new(5),
                cache: IcSlot::new(0),
            },
        );
        emit(
            &mut chunk,
            Instruction::ReturnScalarUnchecked {
                source: Register::new(2),
            },
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
}
