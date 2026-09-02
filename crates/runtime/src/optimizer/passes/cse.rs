//! Common-subexpression elimination for dominating pure operations.

use hashbrown::HashMap;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::rewrite::compact;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::Dominators;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::liveness::effect::overwrites_register;
use crate::optimizer::operands::replace_read_register;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::value::atom::Atom;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.cse {
        return;
    }

    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, configuration, statistics);
    });
}

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.cse || chunk.code.len() < 2 || !chunk.catch_table.is_empty() {
        return;
    }

    let dominators = Dominators::new(chunk);
    let mut candidates: HashMap<ReadKey, Vec<usize>> = HashMap::new();
    if let Some(key) = read_key(chunk, chunk.code[0]) {
        candidates.entry(key).or_default().push(0);
    }
    let mut remove = vec![false; chunk.code.len()];
    for repeated in 1..remove.len() {
        let Some(read) = pure_read(chunk.code[repeated]) else {
            continue;
        };
        let Some(key) = read_key(chunk, chunk.code[repeated]) else {
            continue;
        };

        for original in candidates.get(&key).into_iter().flatten().copied().rev() {
            if remove[original] {
                continue;
            }

            let Some(available) = pure_read(chunk.code[original]) else {
                continue;
            };
            debug_assert!(same_read(chunk, chunk.code[original], chunk.code[repeated]));
            if !dominators.dominates(original, repeated) {
                continue;
            }
            match available_on_all_paths(
                chunk,
                original,
                repeated,
                available.destination,
                available.container,
                available.index,
                &remove,
            ) {
                Availability::ReadInvalidated => break,
                Availability::ValueOverwritten => continue,
                Availability::Available => {}
            }

            if read.destination == available.destination
                || propagate_available_value(
                    chunk,
                    original,
                    repeated,
                    read.destination,
                    available.destination,
                    &remove,
                    &dominators,
                )
            {
                remove[repeated] = true;
                statistics.instructions_removed += 1;
                statistics.common_subexpressions_eliminated += 1;
                break;
            }
        }

        if !remove[repeated] {
            candidates.entry(key).or_default().push(repeated);
        }
    }

    if remove.iter().any(|removed| *removed) {
        compact(chunk, &remove);
    }
}

#[derive(Clone, Copy)]
struct PureRead {
    destination: Register,
    container: Register,
    index: Option<Register>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ReadKey {
    Property(Register, Atom),
    PropertySlot(Register, u16),
    Index(u8, Register, Register),
}

fn read_key(chunk: &Chunk, instruction: Instruction) -> Option<ReadKey> {
    match instruction {
        Instruction::PropertyGet { object, cache, .. } => {
            let IcDescriptor::Member { name, .. } =
                chunk.ic_descriptors.get(usize::from(cache.index()))?
            else {
                return None;
            };
            Some(ReadKey::Property(object, name.clone()))
        }
        Instruction::PropertyGetUnchecked { object, slot, .. } => {
            Some(ReadKey::PropertySlot(object, slot.index()))
        }
        Instruction::IndexGet {
            container, index, ..
        } => Some(ReadKey::Index(0, container, index)),
        Instruction::VecIndexGet {
            container, index, ..
        } => Some(ReadKey::Index(1, container, index)),
        Instruction::DictIndexGetIntKey {
            container, index, ..
        } => Some(ReadKey::Index(2, container, index)),
        Instruction::DictIndexGetStringKey {
            container, index, ..
        } => Some(ReadKey::Index(3, container, index)),
        Instruction::StringIndexGet {
            container, index, ..
        } => Some(ReadKey::Index(4, container, index)),
        _ => None,
    }
}

fn pure_read(instruction: Instruction) -> Option<PureRead> {
    match instruction {
        Instruction::PropertyGet {
            destination,
            object,
            ..
        }
        | Instruction::PropertyGetUnchecked {
            destination,
            object,
            ..
        } if destination != object => Some(PureRead {
            destination,
            container: object,
            index: None,
        }),
        Instruction::IndexGet {
            destination,
            container,
            index,
        }
        | Instruction::VecIndexGet {
            destination,
            container,
            index,
            ..
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
        }
        | Instruction::StringIndexGet {
            destination,
            container,
            index,
        } if destination != container && destination != index => Some(PureRead {
            destination,
            container,
            index: Some(index),
        }),
        _ => None,
    }
}

fn same_read(chunk: &Chunk, left: Instruction, right: Instruction) -> bool {
    match (left, right) {
        (
            Instruction::PropertyGet {
                object: left_object,
                cache: left_cache,
                ..
            },
            Instruction::PropertyGet {
                object: right_object,
                cache: right_cache,
                ..
            },
        ) => {
            if left_object != right_object {
                return false;
            }

            let Some(IcDescriptor::Member {
                name: left_name, ..
            }) = chunk.ic_descriptors.get(usize::from(left_cache.index()))
            else {
                return false;
            };
            let Some(IcDescriptor::Member {
                name: right_name, ..
            }) = chunk.ic_descriptors.get(usize::from(right_cache.index()))
            else {
                return false;
            };

            left_name == right_name
        }
        (
            Instruction::PropertyGetUnchecked {
                object: left_object,
                slot: left_slot,
                ..
            },
            Instruction::PropertyGetUnchecked {
                object: right_object,
                slot: right_slot,
                ..
            },
        ) => left_object == right_object && left_slot == right_slot,
        (
            Instruction::IndexGet {
                container: left_container,
                index: left_index,
                ..
            },
            Instruction::IndexGet {
                container: right_container,
                index: right_index,
                ..
            },
        )
        | (
            Instruction::VecIndexGet {
                container: left_container,
                index: left_index,
                ..
            },
            Instruction::VecIndexGet {
                container: right_container,
                index: right_index,
                ..
            },
        )
        | (
            Instruction::DictIndexGetIntKey {
                container: left_container,
                index: left_index,
                ..
            },
            Instruction::DictIndexGetIntKey {
                container: right_container,
                index: right_index,
                ..
            },
        )
        | (
            Instruction::DictIndexGetStringKey {
                container: left_container,
                index: left_index,
                ..
            },
            Instruction::DictIndexGetStringKey {
                container: right_container,
                index: right_index,
                ..
            },
        )
        | (
            Instruction::StringIndexGet {
                container: left_container,
                index: left_index,
                ..
            },
            Instruction::StringIndexGet {
                container: right_container,
                index: right_index,
                ..
            },
        ) => left_container == right_container && left_index == right_index,
        _ => false,
    }
}

fn propagate_available_value(
    chunk: &mut Chunk,
    original: usize,
    definition: usize,
    replaced: Register,
    available: Register,
    removed: &[bool],
    dominators: &Dominators,
) -> bool {
    let mut replacements = Vec::new();
    let mut work = Vec::new();
    successors(chunk, definition, &mut work);
    let mut seen = vec![false; chunk.code.len()];
    while let Some(index) = work.pop() {
        if index >= chunk.code.len() || seen[index] {
            continue;
        }
        seen[index] = true;

        if removed[index] {
            let mut edges = Vec::new();
            successors(chunk, index, &mut edges);
            work.extend(edges);
            continue;
        }

        let instruction = chunk.code[index];
        let replaced_effect = effect_on(chunk, instruction, replaced);
        let available_effect = effect_on(chunk, instruction, available);
        if available_effect.writes() {
            if replaced_effect.reads() || !replaced_effect.writes() {
                return false;
            }

            continue;
        }
        if replaced_effect.reads() {
            if !dominators.dominates(definition, index) {
                return false;
            }
            let Some(replacement) = replace_read_register(instruction, replaced, available) else {
                return false;
            };
            replacements.push((index, replacement));
        }
        if replaced_effect.writes() {
            continue;
        }

        let mut edges = Vec::new();
        successors(chunk, index, &mut edges);
        if edges
            .iter()
            .any(|edge| *edge > original && *edge <= definition)
        {
            return false;
        }
        for edge in edges {
            if edge > original {
                work.push(edge);
            }
        }
    }

    for (index, replacement) in replacements {
        chunk.code[index] = replacement;
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Availability {
    Available,
    ValueOverwritten,
    ReadInvalidated,
}

fn available_on_all_paths(
    chunk: &Chunk,
    original: usize,
    repeated: usize,
    available: Register,
    container: Register,
    subscript: Option<Register>,
    removed: &[bool],
) -> Availability {
    let mut edges = Vec::new();
    successors(chunk, original, &mut edges);
    let mut work: Vec<_> = edges
        .into_iter()
        .map(|index| (index, Availability::Available))
        .collect();
    let mut seen = vec![[false; 3]; chunk.code.len()];
    let mut result = Availability::Available;
    while let Some((index, mut availability)) = work.pop() {
        if index >= chunk.code.len() {
            continue;
        }
        if index == repeated {
            if availability == Availability::ReadInvalidated {
                return availability;
            }
            if availability == Availability::ValueOverwritten {
                result = availability;
            }
        } else if index == original {
            availability = Availability::Available;
        } else if !removed[index] {
            let instruction = chunk.code[index];
            if availability != Availability::ReadInvalidated
                && (!transparent(instruction)
                    || overwrites_register(chunk, instruction, container)
                    || subscript.is_some_and(|subscript| {
                        overwrites_register(chunk, instruction, subscript)
                    }))
            {
                availability = Availability::ReadInvalidated;
            } else if availability == Availability::Available
                && overwrites_register(chunk, instruction, available)
            {
                availability = Availability::ValueOverwritten;
            }
        }

        let state = match availability {
            Availability::Available => 0,
            Availability::ValueOverwritten => 1,
            Availability::ReadInvalidated => 2,
        };
        if seen[index][state] {
            continue;
        }
        seen[index][state] = true;

        let mut edges = Vec::new();
        successors(chunk, index, &mut edges);
        work.extend(edges.into_iter().map(|target| (target, availability)));
    }

    result
}

fn transparent(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Move { .. }
            | Instruction::MoveOwned { .. }
            | Instruction::LoadConstant { .. }
            | Instruction::LoadNull { .. }
            | Instruction::LoadTrue { .. }
            | Instruction::LoadFalse { .. }
            | Instruction::LoadInt { .. }
            | Instruction::Add { .. }
            | Instruction::Subtract { .. }
            | Instruction::Multiply { .. }
            | Instruction::Divide { .. }
            | Instruction::Modulo { .. }
            | Instruction::Power { .. }
            | Instruction::Negate { .. }
            | Instruction::UnaryPlus { .. }
            | Instruction::AddImmediate { .. }
            | Instruction::SubtractImmediate { .. }
            | Instruction::IntAdd { .. }
            | Instruction::IntSubtract { .. }
            | Instruction::IntMultiply { .. }
            | Instruction::IntModulo { .. }
            | Instruction::IntMultiplyImmediate { .. }
            | Instruction::IntModuloImmediate { .. }
            | Instruction::Equal { .. }
            | Instruction::NotEqual { .. }
            | Instruction::LessThan { .. }
            | Instruction::LessThanOrEqual { .. }
            | Instruction::GreaterThan { .. }
            | Instruction::GreaterThanOrEqual { .. }
            | Instruction::Compare { .. }
            | Instruction::Not { .. }
            | Instruction::Jump { .. }
            | Instruction::JumpIfFalse { .. }
            | Instruction::JumpIfTrue { .. }
            | Instruction::JumpIfNull { .. }
            | Instruction::JumpIfNotNull { .. }
            | Instruction::JumpUnless { .. }
            | Instruction::IntJumpUnless { .. }
            | Instruction::StringJumpUnless { .. }
            | Instruction::StringByteJumpUnlessEqual { .. }
            | Instruction::StringByteJumpUnlessNotEqual { .. }
            | Instruction::IntJumpUnlessImmediate { .. }
            | Instruction::JumpUnlessConstant { .. }
            | Instruction::PropertyGet { .. }
            | Instruction::PropertyGetUnchecked { .. }
            | Instruction::Return { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::ReturnNull
            | Instruction::ReturnNullUnchecked
    )
}

#[cfg(test)]
mod tests {
    use whim_span::Span;

    use crate::bytecode::instruction::operands::ArrayValueMode;
    use crate::bytecode::instruction::operands::Comparison;
    use crate::bytecode::instruction::operands::ImmediateInt;
    use crate::bytecode::instruction::operands::JumpOffset;
    use crate::bytecode::instruction::operands::PropertyReadMode;
    use crate::bytecode::instruction::operands::PropertySlot;
    use crate::bytecode::instruction::operands::Register;
    use crate::bytecode::instruction::operands::ShortJumpOffset;

    use crate::optimizer::OptimizationConfiguration;
    use crate::optimizer::OptimizationStatistics;
    use crate::optimizer::passes::cse::Chunk;
    use crate::optimizer::passes::cse::Instruction;
    use crate::optimizer::passes::cse::optimize_chunk;

    fn emit(chunk: &mut Chunk, instruction: Instruction) {
        chunk.emit(instruction, Span::zero());
    }

    #[test]
    fn removes_a_dominated_repeated_vector_read() {
        let mut chunk = Chunk::new();
        let value = Register::new(7);
        let vector = Register::new(1);
        let index = Register::new(6);
        emit(
            &mut chunk,
            Instruction::VecIndexGet {
                destination: value,
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            },
        );
        emit(
            &mut chunk,
            Instruction::JumpUnless {
                comparison: Comparison::LessThan,
                left: Register::new(4),
                right: value,
                offset: ShortJumpOffset::new(2),
            },
        );
        emit(
            &mut chunk,
            Instruction::VecIndexGet {
                destination: value,
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            },
        );
        emit(&mut chunk, Instruction::ReturnNull);

        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.common_subexpressions_eliminated, 1);
        assert_eq!(chunk.code.len(), 3);
        assert_eq!(
            chunk.code[0],
            Instruction::VecIndexGet {
                destination: value,
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            }
        );
        assert!(matches!(chunk.code[2], Instruction::ReturnNull));
    }

    #[test]
    fn refreshes_an_available_read_when_a_path_loops_through_its_definition() {
        let mut chunk = Chunk::new();
        let value = Register::new(7);
        let vector = Register::new(1);
        let index = Register::new(6);
        emit(
            &mut chunk,
            Instruction::VecIndexGet {
                destination: value,
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            },
        );
        emit(
            &mut chunk,
            Instruction::JumpUnless {
                comparison: Comparison::LessThan,
                left: Register::new(4),
                right: value,
                offset: ShortJumpOffset::new(2),
            },
        );
        emit(
            &mut chunk,
            Instruction::VecIndexGet {
                destination: value,
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            },
        );
        emit(
            &mut chunk,
            Instruction::Jump {
                offset: JumpOffset::new(-3),
            },
        );

        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.common_subexpressions_eliminated, 1);
        assert_eq!(chunk.code.len(), 3);
    }

    #[test]
    fn never_reuses_a_read_already_scheduled_for_removal() {
        let mut chunk = Chunk::new();
        let vector = Register::new(0);
        let index = Register::new(1);
        emit(
            &mut chunk,
            Instruction::VecIndexGet {
                destination: Register::new(2),
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            },
        );
        emit(
            &mut chunk,
            Instruction::VecIndexGet {
                destination: Register::new(3),
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            },
        );
        emit(
            &mut chunk,
            Instruction::VecIndexGet {
                destination: Register::new(4),
                container: vector,
                index,
                value_mode: ArrayValueMode::Float,
            },
        );
        emit(
            &mut chunk,
            Instruction::Return {
                source: Register::new(4),
            },
        );

        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.common_subexpressions_eliminated, 2);
        assert_eq!(chunk.code.len(), 2);
        assert_eq!(
            chunk.code[1],
            Instruction::Return {
                source: Register::new(2),
            }
        );
    }

    #[test]
    fn property_reads_from_different_receivers_are_distinct() {
        let mut chunk = Chunk::new();
        emit(
            &mut chunk,
            Instruction::PropertyGetUnchecked {
                destination: Register::new(2),
                object: Register::new(0),
                slot: PropertySlot::new(0),
                value_mode: PropertyReadMode::Clone,
            },
        );
        emit(
            &mut chunk,
            Instruction::PropertyGetUnchecked {
                destination: Register::new(3),
                object: Register::new(1),
                slot: PropertySlot::new(0),
                value_mode: PropertyReadMode::Clone,
            },
        );
        emit(
            &mut chunk,
            Instruction::Return {
                source: Register::new(3),
            },
        );

        let before = chunk.code.clone();
        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.common_subexpressions_eliminated, 0);
        assert_eq!(chunk.code, before);
    }

    #[test]
    fn keeps_a_read_refreshed_by_its_own_loop() {
        let mut chunk = Chunk::new();
        let string = Register::new(0);
        let index = Register::new(1);
        let character = Register::new(2);
        emit(
            &mut chunk,
            Instruction::StringIndexGet {
                destination: character,
                container: string,
                index,
            },
        );
        emit(
            &mut chunk,
            Instruction::StringIndexGet {
                destination: character,
                container: string,
                index,
            },
        );
        emit(
            &mut chunk,
            Instruction::IncrementJump {
                target: index,
                immediate: ImmediateInt::new(1),
                offset: ShortJumpOffset::new(-1),
            },
        );

        let before = chunk.code.clone();
        let mut statistics = OptimizationStatistics::default();
        optimize_chunk(
            &mut chunk,
            OptimizationConfiguration::default(),
            &mut statistics,
        );

        assert_eq!(statistics.common_subexpressions_eliminated, 0);
        assert_eq!(chunk.code, before);
    }
}
