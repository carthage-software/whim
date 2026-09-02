//! Specialization of collection operations whose container and key types are proven.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::passes::plan_type_specializations;
use crate::optimizer::passes::specialize_chunk_instructions;
use crate::optimizer::rewrite::plan::RewritePlan;

use crate::optimizer::type_flow::ConstantValue;
use crate::optimizer::type_flow::TypeFlow;

use crate::value::heap::Heap;

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_arrays {
        return;
    }

    statistics.array_operations_specialized += plan_type_specializations(
        plan,
        analysis,
        CandidateSet::COLLECTION,
        specialized_instruction,
    );
}

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    heap: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_arrays || chunk.code.is_empty() {
        return;
    }

    statistics.array_operations_specialized +=
        specialize_chunk_instructions(chunk, heap, specialized_instruction);
}

pub(in crate::optimizer) fn specialized_instruction(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
) -> Option<Instruction> {
    if let Instruction::IndexGet {
        destination,
        container,
        index: subscript,
    } = instruction
        && let Some(index) = tuple_element_index(flow, index, container, subscript)
    {
        return Some(Instruction::ElementGet {
            destination,
            subject: container,
            index,
        });
    }

    let vector = TypeDescriptor::Vector(None);
    let dictionary = TypeDescriptor::Dictionary(None);
    if let Some(replacement) = specialize_with(
        instruction,
        |register| flow.proves(index, register, &TypeDescriptor::String),
        |register| flow.proves(index, register, &TypeDescriptor::Int),
        |register| flow.proves(index, register, &vector),
        |register| flow.proves(index, register, &dictionary),
        |destination, array| array_value_mode(flow, index, destination, array),
    ) {
        return Some(replacement);
    }

    match instruction {
        Instruction::VecIndexGet {
            destination,
            container,
            index: subscript,
            value_mode: ArrayValueMode::Generic,
        } => refined_array_value_mode(flow, index, container).map(|value_mode| {
            Instruction::VecIndexGet {
                destination,
                container,
                index: subscript,
                value_mode,
            }
        }),
        Instruction::DictIndexGetIntKey {
            destination,
            container,
            index: subscript,
            value_mode: ArrayValueMode::Generic,
        } => refined_array_value_mode(flow, index, container).map(|value_mode| {
            Instruction::DictIndexGetIntKey {
                destination,
                container,
                index: subscript,
                value_mode,
            }
        }),
        Instruction::DictIndexGetStringKey {
            destination,
            container,
            index: subscript,
            value_mode: ArrayValueMode::Generic,
        } => refined_array_value_mode(flow, index, container).map(|value_mode| {
            Instruction::DictIndexGetStringKey {
                destination,
                container,
                index: subscript,
                value_mode,
            }
        }),
        Instruction::IndexAddAssign {
            container,
            index: subscript,
            value,
            mode: IndexAddMode::Generic,
        } if flow.proves(index, container, &dictionary)
            && flow.proves(index, value, &TypeDescriptor::Int)
            && flow.proves_array_element(index, container, &TypeDescriptor::Int) =>
        {
            Some(Instruction::IndexAddAssign {
                container,
                index: subscript,
                value,
                mode: if flow.proves(index, subscript, &TypeDescriptor::String) {
                    IndexAddMode::DictStringKeyIntValue
                } else {
                    IndexAddMode::DictAnyKeyIntValue
                },
            })
        }
        Instruction::IndexAddAssign {
            container,
            index: subscript,
            value,
            mode: IndexAddMode::DictAnyKeyIntValue,
        } if flow.proves(index, subscript, &TypeDescriptor::String) => {
            Some(Instruction::IndexAddAssign {
                container,
                index: subscript,
                value,
                mode: IndexAddMode::DictStringKeyIntValue,
            })
        }
        Instruction::ForeachInit {
            iterator,
            subject,
            reserve,
        } if reserve == Register::NONE => {
            let reserve = foreach_reservation_target(flow, index, iterator, subject)?;
            Some(Instruction::ForeachInit {
                iterator,
                subject,
                reserve,
            })
        }
        Instruction::ForeachNext {
            iterator,
            key_destination,
            value_destination,
        } => {
            let (initialization, subject) = foreach_subject(flow.chunk(), index, iterator)?;
            let value_mode =
                array_value_mode(flow, index.saturating_add(1), value_destination, subject);
            if flow.proves(initialization, subject, &vector) {
                Some(Instruction::VecForeachNext {
                    iterator,
                    key_destination,
                    value_destination,
                    value_mode,
                })
            } else if flow.proves(initialization, subject, &dictionary) {
                Some(Instruction::DictForeachNext {
                    iterator,
                    key_destination,
                    value_destination,
                    value_mode,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) fn specialize_with(
    instruction: Instruction,
    is_string: impl Fn(Register) -> bool,
    is_int: impl Fn(Register) -> bool,
    is_vector: impl Fn(Register) -> bool,
    is_dictionary: impl Fn(Register) -> bool,
    value_mode: impl Fn(Register, Register) -> ArrayValueMode,
) -> Option<Instruction> {
    match instruction {
        Instruction::Length {
            destination,
            source,
        } if is_string(source) => Some(Instruction::StringLength {
            destination,
            source,
        }),
        Instruction::IndexGet {
            destination,
            container,
            index,
        } if is_string(container) && is_int(index) => Some(Instruction::StringIndexGet {
            destination,
            container,
            index,
        }),
        Instruction::IndexGet {
            destination,
            container,
            index,
        } if is_vector(container) && is_int(index) => Some(Instruction::VecIndexGet {
            destination,
            container,
            index,
            value_mode: value_mode(destination, container),
        }),
        Instruction::IndexGet {
            destination,
            container,
            index,
        } if is_dictionary(container) && is_int(index) => Some(Instruction::DictIndexGetIntKey {
            destination,
            container,
            index,
            value_mode: value_mode(destination, container),
        }),
        Instruction::IndexGet {
            destination,
            container,
            index,
        } if is_dictionary(container) && is_string(index) => {
            Some(Instruction::DictIndexGetStringKey {
                destination,
                container,
                index,
                value_mode: value_mode(destination, container),
            })
        }
        Instruction::IndexSet {
            container,
            index,
            value,
        } if is_vector(container) && is_int(index) => Some(Instruction::VecIndexSet {
            container,
            index,
            value,
        }),
        Instruction::IndexSet {
            container,
            index,
            value,
        } if is_dictionary(container) && is_int(index) => Some(Instruction::DictIndexSetIntKey {
            container,
            index,
            value,
        }),
        Instruction::IndexSet {
            container,
            index,
            value,
        } if is_dictionary(container) && is_string(index) => {
            Some(Instruction::DictIndexSetStringKey {
                container,
                index,
                value,
            })
        }
        Instruction::IndexSet {
            container,
            index,
            value,
        } if is_dictionary(container) => Some(Instruction::DictIndexSet {
            container,
            index,
            value,
        }),
        Instruction::DictIndexSet {
            container,
            index,
            value,
        } if is_int(index) => Some(Instruction::DictIndexSetIntKey {
            container,
            index,
            value,
        }),
        Instruction::DictIndexSet {
            container,
            index,
            value,
        } if is_string(index) => Some(Instruction::DictIndexSetStringKey {
            container,
            index,
            value,
        }),
        Instruction::Append { container, value } if is_vector(container) => {
            Some(Instruction::VecAppend { container, value })
        }
        _ => None,
    }
}

fn tuple_element_index(
    flow: &TypeFlow<'_>,
    instruction: usize,
    container: Register,
    subscript: Register,
) -> Option<ImmediateInt> {
    let ConstantValue::Int(index) = flow.constant_value(instruction, subscript)? else {
        return None;
    };
    let index = usize::try_from(index).ok()?;
    let required = index.checked_add(1)?;
    if !flow.proves(instruction, container, &TypeDescriptor::TupleAny)
        || !flow.destructure_proven(instruction, container, required, required, true)
    {
        return None;
    }

    Some(ImmediateInt::new(i16::try_from(index).ok()?))
}

fn foreach_reservation_target(
    flow: &TypeFlow<'_>,
    initialization: usize,
    iterator: Register,
    subject: Register,
) -> Option<Register> {
    let chunk = flow.chunk();
    let next = chunk.code.get(initialization + 1).and_then(|instruction| {
        matches!(
            instruction,
            Instruction::ForeachNext {
                iterator: candidate,
                ..
            } if *candidate == iterator
        )
        .then_some(initialization + 1)
    })?;

    for index in next + 2..chunk.code.len() {
        if let Instruction::Jump { offset } = chunk.code[index]
            && relative_target(index, offset.offset()) <= next
        {
            break;
        }

        let target = match chunk.code[index] {
            Instruction::IndexSet { container, .. }
            | Instruction::VecIndexSet { container, .. }
            | Instruction::DictIndexSetIntKey { container, .. }
            | Instruction::DictIndexSetStringKey { container, .. }
            | Instruction::IndexAddAssign { container, .. }
            | Instruction::Append { container, .. }
            | Instruction::VecAppend { container, .. }
            | Instruction::Spread { container, .. } => container,
            _ => continue,
        };
        if target != subject {
            return Some(target);
        }
    }

    None
}

fn array_value_mode(
    flow: &TypeFlow<'_>,
    index: usize,
    destination: Register,
    array: Register,
) -> ArrayValueMode {
    let result = index.saturating_add(1);
    if flow.proves(result, destination, &TypeDescriptor::Int) {
        ArrayValueMode::Int
    } else if flow.proves(result, destination, &TypeDescriptor::Float) {
        ArrayValueMode::Float
    } else {
        refined_array_value_mode(flow, index, array).unwrap_or(ArrayValueMode::Generic)
    }
}

fn refined_array_value_mode(
    flow: &TypeFlow<'_>,
    index: usize,
    array: Register,
) -> Option<ArrayValueMode> {
    if flow.proves_array_element(index, array, &TypeDescriptor::Int) {
        Some(ArrayValueMode::Int)
    } else if flow.proves_array_element(index, array, &TypeDescriptor::Float) {
        Some(ArrayValueMode::Float)
    } else {
        None
    }
}

fn foreach_subject(chunk: &Chunk, next: usize, iterator: Register) -> Option<(usize, Register)> {
    for index in (0..next).rev() {
        if let Instruction::ForeachInit {
            iterator: initialized,
            subject,
            ..
        } = chunk.code[index]
            && initialized == iterator
        {
            return Some((index, subject));
        }

        if !effect_on(chunk, chunk.code[index], iterator).is_none() {
            return None;
        }
    }
    None
}
