//! Per-instruction structural verification beyond encoded operand bounds.

use crate::bytecode::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::bytecode::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::bytecode::chunk::descriptors::IntStepLoopDescriptor;
use crate::bytecode::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::chunk::descriptors::descriptor_is_trivial;
use crate::bytecode::chunk::descriptors::string_switch_buckets;
use crate::bytecode::instruction::InstructionOperand;
use crate::bytecode::verify::Chunk;
use crate::bytecode::verify::Instruction;
use crate::bytecode::verify::PresetSlot;
use crate::bytecode::verify::Register;
use crate::bytecode::verify::SwitchTable;
use crate::bytecode::verify::VerifyError;
use crate::bytecode::verify::check_cache;
use crate::bytecode::verify::check_call_descriptor;
use crate::bytecode::verify::check_constant;
use crate::bytecode::verify::check_float_constant;
use crate::bytecode::verify::check_float_pair_update_descriptor;
use crate::bytecode::verify::check_float_squares_sum_branch_descriptor;
use crate::bytecode::verify::check_int_step_loop_descriptor;
use crate::bytecode::verify::check_jump;
use crate::bytecode::verify::check_optional_register;
use crate::bytecode::verify::check_prepared_int_loop_descriptor;
use crate::bytecode::verify::check_property_initialization_descriptor;
use crate::bytecode::verify::check_register;
use crate::bytecode::verify::check_relative_target;
use crate::bytecode::verify::check_string_constant;
use crate::bytecode::verify::check_switch_table;
use crate::bytecode::verify::check_type_descriptor;
use crate::bytecode::verify::check_window;
use crate::unwrap_result_invariant;

fn tuple_window_descriptor(descriptor: &TypeDescriptor, element_count: usize) -> bool {
    match descriptor {
        TypeDescriptor::Tuple(elements) => {
            elements.len() == element_count && elements.iter().all(descriptor_is_trivial)
        }
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.len() <= element_count
                && elements.iter().all(descriptor_is_trivial)
                && descriptor_is_trivial(rest)
        }
        TypeDescriptor::Union(members) | TypeDescriptor::Intersection(members) => members
            .iter()
            .all(|member| tuple_window_descriptor(member, element_count)),
        _ => false,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive match keeps instruction invariants together"
)]
pub(in crate::bytecode::verify) fn verify_instruction(
    chunk: &Chunk,
    at: u32,
    instruction: Instruction,
) -> Result<(), VerifyError> {
    instruction.try_visit_operands(|operand| verify_operand(chunk, at, operand))?;

    match instruction {
        Instruction::Panic { message } => check_string_constant(chunk, at, message),
        Instruction::FloatDifferenceAdd { first_operand, .. }
        | Instruction::PropertyFillIntRange { first_operand, .. }
        | Instruction::PropertyIndexSet { first_operand, .. }
        | Instruction::PropertyIndexSetUnchecked { first_operand, .. } => {
            check_window(chunk, at, u32::from(first_operand.index()), 2)
        }
        Instruction::PropertyRemove {
            destination, mode, ..
        }
        | Instruction::PropertyRemoveUnchecked {
            destination, mode, ..
        } if mode.uses_operand() => check_window(chunk, at, u32::from(destination.index()), 2),
        Instruction::FloatScaleProductAdd { first_operand, .. } => {
            check_window(chunk, at, u32::from(first_operand.index()), 3)
        }
        Instruction::Squares {
            first_destination, ..
        }
        | Instruction::FloatSquares {
            first_destination, ..
        } => check_window(chunk, at, u32::from(first_destination.index()), 2),
        Instruction::FloatSquaresSum {
            first_destination, ..
        } => check_window(chunk, at, u32::from(first_destination.index()), 3),
        Instruction::FloatSquaresSumBranch { descriptor, .. } => {
            let FloatSquaresSumBranchDescriptor {
                sum_destination,
                first_square_destination,
                second_square_destination,
                first_source,
                second_source,
                constant,
                ..
            } = *check_float_squares_sum_branch_descriptor(chunk, at, descriptor)?;
            check_register(chunk, at, sum_destination)?;
            check_register(chunk, at, first_square_destination)?;
            check_register(chunk, at, second_square_destination)?;
            check_register(chunk, at, first_source)?;
            check_register(chunk, at, second_source)?;
            check_float_constant(chunk, at, constant)
        }
        Instruction::FloatPairUpdate { descriptor } => {
            let FloatPairUpdateDescriptor {
                first_destination,
                first_operand,
                constant,
                second_destination,
                second_operand,
                second_addend,
            } = *check_float_pair_update_descriptor(chunk, at, descriptor)?;
            check_register(chunk, at, first_destination)?;
            check_window(chunk, at, u32::from(first_operand.index()), 3)?;
            check_float_constant(chunk, at, constant)?;
            check_register(chunk, at, second_destination)?;
            check_window(chunk, at, u32::from(second_operand.index()), 2)?;
            check_register(chunk, at, second_addend)
        }
        Instruction::PreparedIntNumericLoop { descriptor, .. } => {
            let PreparedIntLoopDescriptor {
                counter,
                limit,
                float_registers,
                ..
            } = *check_prepared_int_loop_descriptor(chunk, at, descriptor)?;
            check_register(chunk, at, counter)?;
            check_register(chunk, at, limit)?;
            let mut remaining = float_registers;
            while remaining != 0 {
                // SAFETY: a u64 has at most 64 trailing zeroes.
                let register = unsafe {
                    unwrap_result_invariant(
                        u16::try_from(remaining.trailing_zeros()),
                        "whim-runtime: a u64 bit index fits u16",
                    )
                };
                check_register(chunk, at, Register::new(register))?;
                remaining &= remaining - 1;
            }
            Ok(())
        }
        Instruction::IntStepLoop { descriptor, .. } => {
            let IntStepLoopDescriptor {
                counter,
                limit,
                step,
                ..
            } = *check_int_step_loop_descriptor(chunk, at, descriptor)?;
            check_register(chunk, at, counter)?;
            check_register(chunk, at, limit)?;
            check_register(chunk, at, step)
        }
        Instruction::InitializeProperties {
            object,
            cache,
            descriptor,
        } => {
            check_register(chunk, at, object)?;
            check_cache(chunk, at, cache)?;
            let descriptor = check_property_initialization_descriptor(chunk, at, descriptor)?;
            if descriptor.entries.len() < 2
                || descriptor
                    .entries
                    .iter()
                    .any(|entry| !entry.value_mode.fresh_receiver())
                || (descriptor.allocates
                    && descriptor.entries.iter().enumerate().any(|(index, entry)| {
                        usize::from(entry.slot.index()) != index || entry.value == object
                    }))
            {
                return Err(VerifyError::PropertyInitializationDescriptorInvalid {
                    instruction: at,
                });
            }
            for entry in &descriptor.entries {
                check_register(chunk, at, entry.value)?;
            }
            Ok(())
        }
        Instruction::IntRangeJumpIf { descriptor, .. }
        | Instruction::IntRangeJumpUnless { descriptor, .. }
            if !matches!(
                chunk.type_descriptors[usize::from(descriptor.index())],
                TypeDescriptor::IntRange { .. }
            ) =>
        {
            Err(VerifyError::TypeDescriptorKindInvalid {
                instruction: at,
                descriptor: descriptor.index(),
            })
        }
        instruction @ (Instruction::SwitchInt { table, .. }
        | Instruction::SwitchString { table, .. }
        | Instruction::SwitchBool { table, .. }
        | Instruction::SwitchFloat { table, .. }
        | Instruction::SwitchPattern { table, .. }
        | Instruction::SwitchTuplePattern { table, .. }) => {
            let table = check_switch_table(chunk, at, table)?;
            let valid_kind = matches!(
                (instruction, table),
                (Instruction::SwitchInt { .. }, SwitchTable::Int { .. })
                    | (
                        Instruction::SwitchString { .. },
                        SwitchTable::String { .. } | SwitchTable::StringByte { .. }
                    )
                    | (Instruction::SwitchBool { .. }, SwitchTable::Bool { .. })
                    | (Instruction::SwitchFloat { .. }, SwitchTable::Float { .. })
                    | (
                        Instruction::SwitchPattern { .. },
                        SwitchTable::Pattern { .. } | SwitchTable::DictionaryShape { .. }
                    )
                    | (
                        Instruction::SwitchTuplePattern { .. },
                        SwitchTable::Pattern { .. }
                    )
            );
            if !valid_kind {
                return Err(VerifyError::SwitchTableInvalid { instruction: at });
            }

            match table {
                SwitchTable::Int {
                    targets, default, ..
                }
                | SwitchTable::StringByte {
                    targets, default, ..
                } => {
                    for &target in targets {
                        check_relative_target(chunk, at, target)?;
                    }
                    check_relative_target(chunk, at, *default)
                }
                SwitchTable::String {
                    arms,
                    buckets,
                    default,
                } => {
                    if string_switch_buckets(arms) != *buckets {
                        return Err(VerifyError::SwitchTableInvalid { instruction: at });
                    }
                    for (_, target) in arms {
                        check_relative_target(chunk, at, *target)?;
                    }
                    check_relative_target(chunk, at, *default)
                }
                SwitchTable::Pattern {
                    descriptors,
                    targets,
                    default,
                } => {
                    let valid_descriptors = match instruction {
                        Instruction::SwitchTuplePattern {
                            first_element,
                            element_count,
                            ..
                        } => {
                            check_window(
                                chunk,
                                at,
                                u32::from(first_element.index()),
                                usize::from(element_count.value()),
                            )?;
                            descriptors.iter().all(|descriptor| {
                                tuple_window_descriptor(
                                    descriptor,
                                    usize::from(element_count.value()),
                                )
                            })
                        }
                        _ => descriptors.iter().all(descriptor_is_trivial),
                    };
                    if descriptors.len() != targets.len() || !valid_descriptors {
                        return Err(VerifyError::SwitchTableInvalid { instruction: at });
                    }
                    for &target in targets {
                        check_relative_target(chunk, at, target)?;
                    }
                    check_relative_target(chunk, at, *default)
                }
                SwitchTable::DictionaryShape {
                    keys,
                    patterns,
                    targets,
                    default,
                } => {
                    if keys.is_empty()
                        || keys.len() > 8
                        || patterns.len() != targets.len()
                        || patterns.iter().any(|pattern| {
                            pattern.len() != keys.len()
                                || !pattern.iter().all(descriptor_is_trivial)
                        })
                    {
                        return Err(VerifyError::SwitchTableInvalid { instruction: at });
                    }
                    for &target in targets {
                        check_relative_target(chunk, at, target)?;
                    }
                    check_relative_target(chunk, at, *default)
                }
                SwitchTable::Bool { targets, default } => {
                    if targets.len() != 2 {
                        return Err(VerifyError::SwitchTableInvalid { instruction: at });
                    }
                    for &target in targets {
                        check_relative_target(chunk, at, target)?;
                    }
                    check_relative_target(chunk, at, *default)
                }
                SwitchTable::Float {
                    values,
                    targets,
                    default,
                } => {
                    if values.len() != targets.len() {
                        return Err(VerifyError::SwitchTableInvalid { instruction: at });
                    }
                    for &target in targets {
                        check_relative_target(chunk, at, target)?;
                    }
                    check_relative_target(chunk, at, *default)
                }
            }
        }
        Instruction::NewVec {
            element_count,
            first_element,
            ..
        }
        | Instruction::NewTuple {
            element_count,
            first_element,
            ..
        } => check_window(
            chunk,
            at,
            u32::from(first_element.index()),
            usize::from(element_count.value()),
        ),
        Instruction::NewDict {
            pair_count,
            first_pair,
            ..
        } => check_window(
            chunk,
            at,
            u32::from(first_pair.index()),
            2 * usize::from(pair_count.value()),
        ),
        Instruction::CallValue {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallValueUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallValueDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamed {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamedDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallNamedUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethod {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethodDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethodUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallMethodDirect {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallStatic {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallStaticDiscarded {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallSelfUnchecked {
            argument_count,
            first_argument,
            ..
        } => check_window(
            chunk,
            at,
            u32::from(first_argument.index()),
            usize::from(argument_count.value()),
        ),
        Instruction::CallWithNames {
            callee, descriptor, ..
        }
        | Instruction::CallWithNamesDiscarded {
            callee, descriptor, ..
        } => {
            let shape = check_call_descriptor(chunk, at, descriptor)?;
            let count = usize::from(shape.positional) + shape.named.len();
            check_window(chunk, at, u32::from(callee.index()) + 1, count)
        }
        Instruction::MakeBound {
            callee, descriptor, ..
        } => {
            let Some(shape) = chunk
                .preset_descriptors
                .get(usize::from(descriptor.index()))
            else {
                return Err(VerifyError::PresetDescriptorOutOfRange {
                    instruction: at,
                    descriptor: descriptor.index(),
                });
            };
            let given = shape
                .slots
                .iter()
                .filter(|slot| {
                    matches!(
                        slot,
                        PresetSlot::GivenPositional | PresetSlot::GivenNamed(_)
                    )
                })
                .count();
            check_window(chunk, at, u32::from(callee.index()) + 1, given)
        }
        Instruction::MakeClosure {
            capture_count,
            first_capture,
            ..
        } => check_window(
            chunk,
            at,
            u32::from(first_capture.index()),
            usize::from(capture_count.value()),
        ),
        Instruction::ForeachNext { .. }
        | Instruction::VecForeachNext { .. }
        | Instruction::DictForeachNext { .. } => verify_foreach_next(chunk, at),
        Instruction::Write {
            value_count,
            first_value,
        }
        | Instruction::WriteLine {
            value_count,
            first_value,
        }
        | Instruction::WriteError {
            value_count,
            first_value,
        }
        | Instruction::WriteErrorLine {
            value_count,
            first_value,
        }
        | Instruction::Debug {
            value_count,
            first_value,
        } => check_window(
            chunk,
            at,
            u32::from(first_value.index()),
            usize::from(value_count.value()),
        ),
        Instruction::Assert {
            operand_count,
            first_value,
            ..
        } => check_window(
            chunk,
            at,
            u32::from(first_value.index()),
            usize::from(operand_count.value()) + 1,
        ),
        _ => Ok(()),
    }
}

fn verify_operand(chunk: &Chunk, at: u32, operand: InstructionOperand) -> Result<(), VerifyError> {
    match operand {
        InstructionOperand::Register(register) => check_register(chunk, at, register),
        InstructionOperand::OptionalRegister(register) => {
            check_optional_register(chunk, at, register)
        }
        InstructionOperand::Constant(constant) => check_constant(chunk, at, constant),
        InstructionOperand::FloatConstant(constant) => check_float_constant(chunk, at, constant),
        InstructionOperand::Cache(cache) => check_cache(chunk, at, cache),
        InstructionOperand::Jump(offset) => check_jump(chunk, at, offset),
        InstructionOperand::RelativeTarget(offset) => {
            check_relative_target(chunk, at, i32::from(offset.offset()))
        }
        InstructionOperand::SwitchTable(table) => check_switch_table(chunk, at, table).map(|_| ()),
        InstructionOperand::TypeDescriptor(descriptor) => {
            check_type_descriptor(chunk, at, descriptor)
        }
        InstructionOperand::CallDescriptor(descriptor) => {
            check_call_descriptor(chunk, at, descriptor).map(|_| ())
        }
        InstructionOperand::PresetDescriptor(descriptor) => chunk
            .preset_descriptors
            .get(usize::from(descriptor.index()))
            .map(|_| ())
            .ok_or_else(|| VerifyError::PresetDescriptorOutOfRange {
                instruction: at,
                descriptor: descriptor.index(),
            }),
        InstructionOperand::FloatPairUpdateDescriptor(descriptor) => {
            check_float_pair_update_descriptor(chunk, at, descriptor).map(|_| ())
        }
        InstructionOperand::FloatSquaresSumBranchDescriptor(descriptor) => {
            check_float_squares_sum_branch_descriptor(chunk, at, descriptor).map(|_| ())
        }
        InstructionOperand::IntStepLoopDescriptor(descriptor) => {
            check_int_step_loop_descriptor(chunk, at, descriptor).map(|_| ())
        }
        InstructionOperand::PreparedIntLoopDescriptor(descriptor) => {
            check_prepared_int_loop_descriptor(chunk, at, descriptor).map(|_| ())
        }
        InstructionOperand::PropertyInitializationDescriptor(descriptor) => {
            check_property_initialization_descriptor(chunk, at, descriptor).map(|_| ())
        }
    }
}

fn verify_foreach_next(chunk: &Chunk, at: u32) -> Result<(), VerifyError> {
    let next = if matches!(
        chunk.code.get(at as usize + 1),
        Some(Instruction::DrainFinalizers)
    ) {
        at as usize + 2
    } else {
        at as usize + 1
    };
    match chunk.code.get(next) {
        Some(Instruction::Jump { .. }) => Ok(()),
        _ => Err(VerifyError::ForeachNextWithoutJump { instruction: at }),
    }
}
