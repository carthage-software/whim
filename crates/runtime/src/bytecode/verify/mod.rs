//! Structural validation of chunks and compiled units.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::CallDescriptor;
use crate::bytecode::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::bytecode::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::bytecode::chunk::descriptors::IntStepLoopDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::bytecode::chunk::descriptors::PresetSlot;
use crate::bytecode::chunk::descriptors::PropertyInitializationDescriptor;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::CallDescriptorIndex;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::bytecode::instruction::operands::FloatPairUpdateDescriptorIndex;
use crate::bytecode::instruction::operands::FloatSquaresSumBranchDescriptorIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::IntStepLoopDescriptorIndex;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::PreparedIntLoopDescriptorIndex;
use crate::bytecode::instruction::operands::PropertyInitializationDescriptorIndex;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::SwitchTableIndex;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::verify::instruction::verify_instruction;
use crate::unwrap_result_invariant;

mod instruction;

#[cfg(test)]
mod fuzz;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyError {
    LocalRegistersOutOfRange {
        locals: u16,
        registers: u16,
    },
    ParameterRegistersOutOfRange {
        first: u16,
        count: usize,
        locals: u16,
    },
    TraceArgumentRegisterOutOfRange {
        argument: usize,
        register: u16,
        locals: u16,
    },
    /// A non-empty trace map must describe every parameter.
    TraceArgumentCountMismatch {
        parameters: u16,
        trace_arguments: usize,
    },
    SpanCountMismatch {
        code: usize,
        spans: usize,
    },
    RegisterOutOfRange {
        instruction: u32,
        register: u16,
    },
    ConstantOutOfRange {
        instruction: u32,
        constant: u16,
    },
    ConstantKindInvalid {
        instruction: u32,
        constant: u16,
    },
    TypeDescriptorOutOfRange {
        instruction: u32,
        descriptor: u16,
    },
    TypeDescriptorKindInvalid {
        instruction: u32,
        descriptor: u16,
    },
    CallDescriptorOutOfRange {
        instruction: u32,
        descriptor: u16,
    },
    PresetDescriptorOutOfRange {
        instruction: u32,
        descriptor: u16,
    },
    NumericDescriptorOutOfRange {
        instruction: u32,
        descriptor: u16,
    },
    PropertyInitializationDescriptorOutOfRange {
        instruction: u32,
        descriptor: u16,
    },
    PropertyInitializationDescriptorInvalid {
        instruction: u32,
    },
    SwitchTableOutOfRange {
        instruction: u32,
        table: u16,
    },
    SwitchTableInvalid {
        instruction: u32,
    },
    CacheSlotOutOfRange {
        instruction: u32,
        slot: u16,
    },
    JumpOutOfRange {
        instruction: u32,
        target: i64,
    },
    RegisterWindowOutOfRange {
        instruction: u32,
        first: u32,
        count: usize,
    },
    ForeachNextWithoutJump {
        instruction: u32,
    },
    CatchRangeInvalid {
        entry: usize,
        start: u32,
        end: u32,
    },
    CatchHandlerOutOfRange {
        entry: usize,
        handler: u32,
    },
    CatchTypeDescriptorOutOfRange {
        entry: usize,
        descriptor: u16,
    },
    CatchTemporaryFloorOutOfRange {
        entry: usize,
        register: u16,
    },
    CatchBindingOutOfRange {
        entry: usize,
        register: u16,
    },
    /// The chunk is empty, or its last instruction falls through, so the
    /// interpreter's sequential fetch would read past the end. A
    /// well-formed chunk ends in `Return`, `ReturnNull`, `Throw`, `Exit`,
    /// or an unconditional `Jump`.
    MissingTerminator {
        length: usize,
    },
}

pub(crate) fn verify(chunk: &Chunk) -> Result<(), VerifyError> {
    if chunk.local_register_count > chunk.register_count {
        return Err(VerifyError::LocalRegistersOutOfRange {
            locals: chunk.local_register_count,
            registers: chunk.register_count,
        });
    }

    let parameter_end = usize::from(chunk.parameter_register_start)
        .saturating_add(usize::from(chunk.parameter_register_count));
    if parameter_end > usize::from(chunk.local_register_count) {
        return Err(VerifyError::ParameterRegistersOutOfRange {
            first: chunk.parameter_register_start,
            count: usize::from(chunk.parameter_register_count),
            locals: chunk.local_register_count,
        });
    }
    if !chunk.trace_argument_registers.is_empty()
        && chunk.trace_argument_registers.len() != usize::from(chunk.parameter_register_count)
    {
        return Err(VerifyError::TraceArgumentCountMismatch {
            parameters: chunk.parameter_register_count,
            trace_arguments: chunk.trace_argument_registers.len(),
        });
    }
    for (argument, register) in chunk.trace_argument_registers.iter().enumerate() {
        if *register != Register::NONE && register.index() >= chunk.local_register_count {
            return Err(VerifyError::TraceArgumentRegisterOutOfRange {
                argument,
                register: register.index(),
                locals: chunk.local_register_count,
            });
        }
    }

    if chunk.spans.len() != chunk.code.len() {
        return Err(VerifyError::SpanCountMismatch {
            code: chunk.code.len(),
            spans: chunk.spans.len(),
        });
    }

    for (index, instruction) in chunk.code.iter().enumerate() {
        // SAFETY: a chunk's instruction count never exceeds u32::MAX, so the index fits u32.
        let at = unsafe {
            unwrap_result_invariant(
                u32::try_from(index),
                "whim-runtime: a chunk cannot exceed u32::MAX instructions",
            )
        };

        verify_instruction(chunk, at, *instruction)?;
    }

    match chunk.code.last() {
        Some(
            Instruction::Jump { .. }
            | Instruction::NumericRegionJump { .. }
            | Instruction::Return { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnNull
            | Instruction::ReturnNullUnchecked
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::Throw { .. }
            | Instruction::Exit { .. }
            | Instruction::Panic { .. },
        ) => {}
        _ => {
            return Err(VerifyError::MissingTerminator {
                length: chunk.code.len(),
            });
        }
    }

    verify_catch_table(chunk)
}

fn verify_catch_table(chunk: &Chunk) -> Result<(), VerifyError> {
    // SAFETY: a chunk's instruction count never exceeds u32::MAX.
    let length = unsafe {
        unwrap_result_invariant(
            u32::try_from(chunk.code.len()),
            "whim-runtime: a chunk cannot exceed u32::MAX instructions",
        )
    };
    for (entry_index, entry) in chunk.catch_table.iter().enumerate() {
        if entry.start > entry.end || entry.end > length {
            return Err(VerifyError::CatchRangeInvalid {
                entry: entry_index,
                start: entry.start,
                end: entry.end,
            });
        }

        if entry.handler >= length {
            return Err(VerifyError::CatchHandlerOutOfRange {
                entry: entry_index,
                handler: entry.handler,
            });
        }

        let descriptor = entry.type_descriptor.index();
        if usize::from(descriptor) >= chunk.type_descriptors.len() {
            return Err(VerifyError::CatchTypeDescriptorOutOfRange {
                entry: entry_index,
                descriptor,
            });
        }

        if entry.temporary_floor > chunk.register_count {
            return Err(VerifyError::CatchTemporaryFloorOutOfRange {
                entry: entry_index,
                register: entry.temporary_floor,
            });
        }

        if let Some(binding) = entry.binding
            && binding.index() >= chunk.register_count
        {
            return Err(VerifyError::CatchBindingOutOfRange {
                entry: entry_index,
                register: binding.index(),
            });
        }
    }

    Ok(())
}

pub(crate) fn verify_unit(unit: &CompiledUnit) -> Result<(), VerifyError> {
    verify(&unit.main)?;
    for function in &unit.functions {
        verify_function(function)?;
    }

    for class in &unit.classes {
        verify_class(class)?;
    }

    for constant in &unit.constants {
        verify_initializer(&constant.initializer)?;
    }

    Ok(())
}

fn verify_function(function: &CompiledFunction) -> Result<(), VerifyError> {
    verify(&function.chunk)
}

fn verify_class(class: &CompiledClassLike) -> Result<(), VerifyError> {
    for constant in &class.constants {
        verify_initializer(&constant.initializer)?;
    }

    for property in &class.properties {
        if let Some(default) = &property.default {
            verify_initializer(default)?;
        }
    }

    for method in &class.methods {
        verify_function(&method.function)?;
    }

    for case in &class.cases {
        if let Some(value) = &case.value {
            verify_initializer(value)?;
        }
    }

    for attribute in &class.attributes {
        for argument in &attribute.arguments {
            verify_initializer(argument)?;
        }
        for (_, argument) in &attribute.named_arguments {
            verify_initializer(argument)?;
        }
    }

    Ok(())
}

fn verify_initializer(initializer: &ConstantInitializer) -> Result<(), VerifyError> {
    match initializer {
        ConstantInitializer::Literal(_) => Ok(()),
        ConstantInitializer::Thunk(chunk) => verify(chunk),
    }
}

const fn check_register(chunk: &Chunk, at: u32, register: Register) -> Result<(), VerifyError> {
    if register.index() < chunk.register_count {
        Ok(())
    } else {
        Err(VerifyError::RegisterOutOfRange {
            instruction: at,
            register: register.index(),
        })
    }
}

/// Checks an optional register; [`Register::NONE`] is always valid.
fn check_optional_register(chunk: &Chunk, at: u32, register: Register) -> Result<(), VerifyError> {
    if register == Register::NONE {
        Ok(())
    } else {
        check_register(chunk, at, register)
    }
}

fn check_constant(chunk: &Chunk, at: u32, constant: ConstantIndex) -> Result<(), VerifyError> {
    if usize::from(constant.index()) < chunk.constants.len() {
        Ok(())
    } else {
        Err(VerifyError::ConstantOutOfRange {
            instruction: at,
            constant: constant.index(),
        })
    }
}

fn check_float_constant(
    chunk: &Chunk,
    at: u32,
    constant: ConstantIndex,
) -> Result<(), VerifyError> {
    check_constant(chunk, at, constant)?;
    if matches!(
        chunk.constants[usize::from(constant.index())],
        Literal::Float(_)
    ) {
        Ok(())
    } else {
        Err(VerifyError::ConstantKindInvalid {
            instruction: at,
            constant: constant.index(),
        })
    }
}

fn check_string_constant(
    chunk: &Chunk,
    at: u32,
    constant: ConstantIndex,
) -> Result<(), VerifyError> {
    check_constant(chunk, at, constant)?;
    if matches!(
        chunk.constants[usize::from(constant.index())],
        Literal::String(_)
    ) {
        Ok(())
    } else {
        Err(VerifyError::ConstantKindInvalid {
            instruction: at,
            constant: constant.index(),
        })
    }
}

fn check_type_descriptor(
    chunk: &Chunk,
    at: u32,
    descriptor: DescriptorIndex,
) -> Result<(), VerifyError> {
    if usize::from(descriptor.index()) < chunk.type_descriptors.len() {
        Ok(())
    } else {
        Err(VerifyError::TypeDescriptorOutOfRange {
            instruction: at,
            descriptor: descriptor.index(),
        })
    }
}

fn check_call_descriptor(
    chunk: &Chunk,
    at: u32,
    descriptor: CallDescriptorIndex,
) -> Result<&CallDescriptor, VerifyError> {
    chunk
        .call_descriptors
        .get(usize::from(descriptor.index()))
        .ok_or_else(|| VerifyError::CallDescriptorOutOfRange {
            instruction: at,
            descriptor: descriptor.index(),
        })
}

fn check_prepared_int_loop_descriptor(
    chunk: &Chunk,
    at: u32,
    descriptor: PreparedIntLoopDescriptorIndex,
) -> Result<&PreparedIntLoopDescriptor, VerifyError> {
    chunk
        .prepared_int_loop_descriptors
        .get(usize::from(descriptor.index()))
        .ok_or_else(|| VerifyError::NumericDescriptorOutOfRange {
            instruction: at,
            descriptor: descriptor.index(),
        })
}

fn check_int_step_loop_descriptor(
    chunk: &Chunk,
    at: u32,
    descriptor: IntStepLoopDescriptorIndex,
) -> Result<&IntStepLoopDescriptor, VerifyError> {
    chunk
        .int_step_loop_descriptors
        .get(usize::from(descriptor.index()))
        .ok_or_else(|| VerifyError::NumericDescriptorOutOfRange {
            instruction: at,
            descriptor: descriptor.index(),
        })
}

fn check_float_squares_sum_branch_descriptor(
    chunk: &Chunk,
    at: u32,
    descriptor: FloatSquaresSumBranchDescriptorIndex,
) -> Result<&FloatSquaresSumBranchDescriptor, VerifyError> {
    chunk
        .float_squares_sum_branch_descriptors
        .get(usize::from(descriptor.index()))
        .ok_or_else(|| VerifyError::NumericDescriptorOutOfRange {
            instruction: at,
            descriptor: descriptor.index(),
        })
}

fn check_float_pair_update_descriptor(
    chunk: &Chunk,
    at: u32,
    descriptor: FloatPairUpdateDescriptorIndex,
) -> Result<&FloatPairUpdateDescriptor, VerifyError> {
    chunk
        .float_pair_update_descriptors
        .get(usize::from(descriptor.index()))
        .ok_or_else(|| VerifyError::NumericDescriptorOutOfRange {
            instruction: at,
            descriptor: descriptor.index(),
        })
}

fn check_property_initialization_descriptor(
    chunk: &Chunk,
    at: u32,
    descriptor: PropertyInitializationDescriptorIndex,
) -> Result<&PropertyInitializationDescriptor, VerifyError> {
    chunk
        .property_initialization_descriptors
        .get(usize::from(descriptor.index()))
        .ok_or_else(|| VerifyError::PropertyInitializationDescriptorOutOfRange {
            instruction: at,
            descriptor: descriptor.index(),
        })
}

fn check_switch_table(
    chunk: &Chunk,
    at: u32,
    table: SwitchTableIndex,
) -> Result<&SwitchTable, VerifyError> {
    chunk
        .switch_tables
        .get(usize::from(table.index()))
        .ok_or_else(|| VerifyError::SwitchTableOutOfRange {
            instruction: at,
            table: table.index(),
        })
}

/// Checks an inline-cache index against the descriptor table.
fn check_cache(chunk: &Chunk, at: u32, slot: IcSlot) -> Result<(), VerifyError> {
    if usize::from(slot.index()) < chunk.ic_descriptors.len() {
        Ok(())
    } else {
        Err(VerifyError::CacheSlotOutOfRange {
            instruction: at,
            slot: slot.index(),
        })
    }
}

/// Checks that `at + relative` lands inside the code.
fn check_relative_target(chunk: &Chunk, at: u32, relative: i32) -> Result<(), VerifyError> {
    let target = i64::from(at) + i64::from(relative);
    if usize::try_from(target).is_ok_and(|target| target < chunk.code.len()) {
        Ok(())
    } else {
        Err(VerifyError::JumpOutOfRange {
            instruction: at,
            target,
        })
    }
}

fn check_jump(chunk: &Chunk, at: u32, offset: JumpOffset) -> Result<(), VerifyError> {
    check_relative_target(chunk, at, offset.offset())
}

fn check_window(chunk: &Chunk, at: u32, first: u32, count: usize) -> Result<(), VerifyError> {
    if u64::from(first) + count as u64 <= u64::from(chunk.register_count) {
        Ok(())
    } else {
        Err(VerifyError::RegisterWindowOutOfRange {
            instruction: at,
            first,
            count,
        })
    }
}
