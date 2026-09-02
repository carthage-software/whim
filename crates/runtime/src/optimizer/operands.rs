//! Per-instruction register operands shared by optimizer analyses and rewrites.

use std::mem;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::word::InstructionKind;
use crate::unwrap_option_invariant;

macro_rules! instructions {
    ($($name:ident)|+ ; $fields:tt) => {
        $(Instruction::$name $fields)|+
    };
}

macro_rules! instruction_kinds {
    ($($name:ident)|+) => {
        $(InstructionKind::$name)|+
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::optimizer) enum Access {
    Read,
    Write,
}

#[derive(Clone, Copy)]
pub(in crate::optimizer) struct Operand {
    pub(in crate::optimizer) offset: usize,
    pub(in crate::optimizer) access: Access,
}

const fn operand(offset: usize, access: Access) -> Operand {
    Operand { offset, access }
}

const R1: Operand = operand(1, Access::Read);
const R2: Operand = operand(2, Access::Read);
const R3: Operand = operand(3, Access::Read);
const R4: Operand = operand(4, Access::Read);
const R5: Operand = operand(5, Access::Read);
const W1: Operand = operand(1, Access::Write);
const W2: Operand = operand(2, Access::Write);
const W3: Operand = operand(3, Access::Write);
const W5: Operand = operand(5, Access::Write);

pub(in crate::optimizer) fn write_may_alias_inputs(kind: InstructionKind) -> bool {
    !matches!(
        kind,
        InstructionKind::CallNamed
            | InstructionKind::CallNamedDiscarded
            | InstructionKind::CallMethod
            | InstructionKind::CallMethodDiscarded
            | InstructionKind::CallStatic
            | InstructionKind::CallStaticDiscarded
            | InstructionKind::CallValue
            | InstructionKind::CallValueDiscarded
            | InstructionKind::ForeachInit
            | InstructionKind::ForeachNext
            | InstructionKind::VecForeachNext
            | InstructionKind::DictForeachNext
    )
}

pub(in crate::optimizer) fn operands(kind: InstructionKind) -> Option<&'static [Operand]> {
    match kind {
        instruction_kinds!(
            LoadConstant
                | LoadNull
                | LoadTrue
                | LoadFalse
                | LoadInt
                | NewStatic
                | NewTyped
                | StaticPropertyGet
                | ConstantGet
                | ClassConstantGet
                | CallNamedConstantUnchecked
                | Clear
        ) => Some(&[W1]),
        instruction_kinds!(
            NewVec
                | NewDict
                | NewTuple
                | MakeClosure
                | CallNamed
                | CallNamedDiscarded
                | CallMethod
                | CallMethodDiscarded
                | CallMethodUnchecked
                | CallMethodDirect
                | CallStatic
                | CallStaticDiscarded
        ) => Some(&[W2]),
        InstructionKind::CallNamedUnchecked
        | InstructionKind::CallSelfUnchecked
        | InstructionKind::Require => Some(&[R4, W2]),
        InstructionKind::CallValue | InstructionKind::CallValueDiscarded => Some(&[W2, R4]),
        instruction_kinds!(
            Jump | Write
                | WriteLine
                | WriteError
                | WriteErrorLine
                | Debug
                | DrainFinalizers
                | ReturnNull
                | ReturnNullUnchecked
                | ReturnIntUnchecked
                | Rethrow
        ) => Some(&[]),
        instruction_kinds!(
            Move | Negate
                | UnaryPlus
                | BitwiseNot
                | Not
                | Rest
                | Length
                | RemoveFirst
                | RemoveLast
                | ElementGet
                | NewDynamic
                | PropertyGet
                | PropertyGetUnchecked
                | CloneObject
                | Is
                | AsCheck
                | AsOrNull
                | StringLength
                | FloatMultiplyConstant
                | IntBitwiseNot
                | AddImmediate
                | SubtractImmediate
                | IntMultiplyImmediate
                | IntModuloImmediate
        ) => Some(&[R3, W1]),
        InstructionKind::MoveOwned => Some(&[R3, W3, W1]),
        InstructionKind::IntAddAssign => Some(&[R1, R3, W1]),
        instruction_kinds!(
            Add | Subtract
                | Multiply
                | IntAdd
                | IntSubtract
                | IntMultiply
                | IntModulo
                | IntBitwiseAnd
                | IntBitwiseOr
                | IntBitwiseXor
                | IntShiftLeft
                | IntShiftRight
                | Divide
                | Modulo
                | Power
                | Concatenate
                | BitwiseAnd
                | BitwiseOr
                | BitwiseXor
                | ShiftLeft
                | ShiftRight
                | Equal
                | NotEqual
                | LessThan
                | LessThanOrEqual
                | GreaterThan
                | GreaterThanOrEqual
                | Compare
                | IndexGet
                | StringIndexGet
                | Remove
                | SwapRemove
                | Contains
                | ContainsKey
                | NewFilledVec
                | FloatAdd
                | FloatSubtract
                | FloatMultiply
                | ForeachInit
        ) => Some(&[R3, R5, W1]),
        instruction_kinds!(
            StringByteEqual
                | StringByteNotEqual
                | StringByteLessThan
                | StringByteLessThanOrEqual
                | StringByteGreaterThan
                | StringByteGreaterThanOrEqual
        ) => Some(&[W1, R3, R5]),
        instruction_kinds!(
            JumpIfFalse
                | JumpIfTrue
                | JumpIfNull
                | JumpIfNotNull
                | SwitchInt
                | SwitchString
                | SwitchBool
                | SwitchFloat
                | SwitchPattern
                | IntRangeJumpIf
                | IntRangeJumpUnless
                | BoolPatternBranch
                | SwitchTuplePattern
                | FillDefault
                | CheckDefined
                | CheckDestructure
                | ThrowUnhandledMatch
                | Throw
                | Return
                | ReturnUnchecked
                | ReturnReferenceUnchecked
                | ReturnScalarUnchecked
                | PropertyStep
                | PropertyStepUnchecked
                | CheckSoleReference
                | CheckDiscardedResult
                | Exit
        ) => Some(&[R1]),
        instruction_kinds!(
            JumpUnless | IntJumpUnless | StringJumpUnless | NumericLoop | IntNumericLoop | Assert
        ) => Some(&[R2, R4]),
        InstructionKind::CounterLoop | InstructionKind::IntCounterLoop => Some(&[R2, R4, W2]),
        instruction_kinds!(
            StringByteJumpUnlessEqual
                | StringByteJumpUnlessNotEqual
                | ReserveArray
                | Append
                | VecAppend
                | Spread
                | PropertySet
                | PropertySetUnchecked
                | PropertyInitRaw
                | PropertyIndexUpdate
                | PropertyIndexUpdateUnchecked
                | PropertyIndexSet
                | PropertyIndexSetUnchecked
                | PropertyAdd
                | PropertyAddUnchecked
                | ReturnPairUnchecked
        ) => Some(&[R1, R3]),
        InstructionKind::JumpUnlessConstant | InstructionKind::IntJumpUnlessImmediate => {
            Some(&[R2])
        }
        InstructionKind::IncrementJump => Some(&[R1, W1]),
        instruction_kinds!(
            IndexSet | VecIndexSet | DictIndexSetIntKey | DictIndexSetStringKey | DictIndexSet
        ) => Some(&[R1, R3, R5]),
        InstructionKind::PropertyRemove | InstructionKind::PropertyRemoveUnchecked => {
            Some(&[R1, W3])
        }
        InstructionKind::StaticPropertySet => Some(&[R3]),
        InstructionKind::ForeachNext
        | InstructionKind::VecForeachNext
        | InstructionKind::DictForeachNext => Some(&[R1, W3, W5]),
        _ => None,
    }
}

/// Registers read through a consecutive window whose tail is not encoded as
/// an explicit instruction field. The first register is already present in
/// [`operands`]; this returns only its implicit successors.
pub(in crate::optimizer) fn implicit_reads(instruction: Instruction) -> Option<(Register, usize)> {
    match instruction {
        Instruction::Assert {
            operand_count,
            first_value,
            ..
        } if operand_count.value() != 0 => Some((
            Register::new(first_value.index() + 1),
            usize::from(operand_count.value()),
        )),
        instructions!(
            CallNamed | CallNamedDiscarded | CallMethod | CallMethodDiscarded
                | CallMethodUnchecked | CallMethodDirect | CallStatic | CallStaticDiscarded
                | CallValue | CallValueDiscarded;
            {
            argument_count,
            first_argument,
            ..
            }
        ) if argument_count.value() != 0 => {
            Some((first_argument, usize::from(argument_count.value())))
        }
        instructions!(CallNamedUnchecked | CallSelfUnchecked; {
            argument_count,
            first_argument,
            ..
        }) if argument_count.value() > 1 => Some((
            Register::new(first_argument.index() + 1),
            usize::from(argument_count.value() - 1),
        )),
        instructions!(NewVec | NewTuple; {
            element_count,
            first_element,
            ..
        }) if element_count.value() != 0 => {
            Some((first_element, usize::from(element_count.value())))
        }
        Instruction::SwitchTuplePattern {
            first_element,
            element_count,
            ..
        } if element_count.value() != 0 => {
            Some((first_element, usize::from(element_count.value())))
        }
        Instruction::NewDict {
            pair_count,
            first_pair,
            ..
        } if pair_count.value() != 0 => Some((first_pair, usize::from(pair_count.value()) * 2)),
        Instruction::MakeClosure {
            capture_count,
            first_capture,
            ..
        } if capture_count.value() != 0 => {
            Some((first_capture, usize::from(capture_count.value())))
        }
        instructions!(Write | WriteLine | WriteError | WriteErrorLine | Debug; {
            value_count,
            first_value,
        }) if value_count.value() != 0 => Some((first_value, usize::from(value_count.value()))),
        instructions!(PropertyIndexSet | PropertyIndexSetUnchecked; { first_operand, .. }) => {
            Some((Register::new(first_operand.index() + 1), 1))
        }
        instructions!(PropertyRemove | PropertyRemoveUnchecked; {
            destination,
            mode,
            ..
        }) if mode.uses_operand() => Some((Register::new(destination.index() + 1), 1)),
        _ => None,
    }
}

pub(in crate::optimizer) fn instruction_bytes(instruction: Instruction) -> [u8; 8] {
    // SAFETY: verified bytecode and this pass's guards prove the index.
    unsafe { mem::transmute::<Instruction, [u8; 8]>(instruction) }
}

pub(in crate::optimizer) fn register_at(bytes: [u8; 8], offset: usize) -> Register {
    Register::new(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

#[inline]
fn for_each_explicit_register(
    instruction: Instruction,
    access: Option<Access>,
    mut visit: impl FnMut(Register),
) -> bool {
    let Some(operands) = operands(instruction.kind()) else {
        return false;
    };
    let bytes = instruction_bytes(instruction);
    for (position, operand) in operands.iter().enumerate() {
        let duplicate = operands[..position].iter().any(|previous| {
            previous.offset == operand.offset
                && match access {
                    Some(Access::Read) => previous.access == Access::Read,
                    Some(Access::Write) | None => true,
                }
        });
        if access.is_some_and(|access| operand.access != access)
            || duplicate
            || (operand.offset == 4
                && matches!(
                    instruction,
                    Instruction::CallNamedUnchecked { argument_count, .. }
                        | Instruction::CallSelfUnchecked { argument_count, .. }
                        if argument_count.value() == 0
                ))
        {
            continue;
        }

        let register = register_at(bytes, operand.offset);
        if register != Register::NONE {
            visit(register);
        }
    }

    true
}

/// Visits every register whose current value can affect an instruction or
/// whose previous value may be replaced by it.
pub(in crate::optimizer) fn for_each_register(
    instruction: Instruction,
    mut visit: impl FnMut(Register),
) -> bool {
    if !for_each_explicit_register(instruction, None, &mut visit) {
        return false;
    }

    if let Some((first, count)) = implicit_reads(instruction) {
        for offset in 0..count {
            visit(Register::new(first.index() + offset as u16));
        }
    }

    true
}

/// Visits every register whose previous value an instruction replaces.
pub(in crate::optimizer) fn for_each_write_register(
    instruction: Instruction,
    visit: impl FnMut(Register),
) -> bool {
    for_each_explicit_register(instruction, Some(Access::Write), visit)
}

/// Visits every register whose current value an instruction reads.
pub(in crate::optimizer) fn for_each_read_register(
    instruction: Instruction,
    mut visit: impl FnMut(Register),
) -> bool {
    if !for_each_explicit_register(instruction, Some(Access::Read), &mut visit) {
        return false;
    }

    if let Some((first, count)) = implicit_reads(instruction) {
        for offset in 0..count {
            visit(Register::new(first.index() + offset as u16));
        }
    }

    true
}

pub(in crate::optimizer) fn replace_read_register(
    instruction: Instruction,
    from: Register,
    to: Register,
) -> Option<Instruction> {
    if matches!(
        instruction,
        Instruction::CallNamedUnchecked {
            argument_count,
            first_argument,
            ..
        }
        | Instruction::CallSelfUnchecked {
            argument_count,
            first_argument,
            ..
        } if {
            let index = from.index();
            let first = first_argument.index();
            index >= first && usize::from(index - first) < usize::from(argument_count.value())
        }
    ) {
        return None;
    }

    if matches!(
        instruction,
        Instruction::PropertyIndexSet { first_operand, .. }
            | Instruction::PropertyIndexSetUnchecked { first_operand, .. }
            if from == first_operand
                || from.index() == first_operand.index().saturating_add(1)
    ) {
        return None;
    }

    if implicit_reads(instruction).is_some_and(|(first, count)| {
        let index = from.index();
        let first = first.index();
        index >= first && usize::from(index - first) < count
    }) {
        return None;
    }

    let mut bytes = instruction_bytes(instruction);
    for operand in operands(instruction.kind())? {
        if operand.access == Access::Read && register_at(bytes, operand.offset) == from {
            let encoded = to.index().to_le_bytes();
            bytes[operand.offset] = encoded[0];
            bytes[operand.offset + 1] = encoded[1];
        }
    }

    // SAFETY: verified bytecode and this pass's guards prove the index.
    Some(unsafe { mem::transmute::<[u8; 8], Instruction>(bytes) })
}

pub(in crate::optimizer) fn remap(instruction: Instruction, mapping: &[Register]) -> Instruction {
    // SAFETY: the chunk was classified before rewriting, so every instruction kind has operands.
    let operands = unsafe {
        unwrap_option_invariant(
            operands(instruction.kind()),
            "the chunk was classified before rewriting",
        )
    };
    let mut bytes = instruction_bytes(instruction);
    for (position, operand) in operands.iter().enumerate() {
        if operand.offset == 4
            && matches!(
                instruction,
                Instruction::CallNamedUnchecked { argument_count, .. }
                    | Instruction::CallSelfUnchecked { argument_count, .. }
                    if argument_count.value() == 0
            )
        {
            continue;
        }
        if operands[..position]
            .iter()
            .any(|previous| previous.offset == operand.offset)
        {
            continue;
        }

        let register = register_at(bytes, operand.offset);
        if register == Register::NONE {
            continue;
        }

        let remapped = mapping[usize::from(register.index())].index().to_le_bytes();
        bytes[operand.offset] = remapped[0];
        bytes[operand.offset + 1] = remapped[1];
    }

    // SAFETY: verified bytecode and this pass's guards prove the index.
    unsafe { mem::transmute::<[u8; 8], Instruction>(bytes) }
}
