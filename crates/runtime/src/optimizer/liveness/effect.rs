//! The per-instruction read/write effect classification behind the liveness
//! queries.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::liveness::Effect;

macro_rules! instructions {
    ($($name:ident)|+ ; $fields:tt) => {
        $(Instruction::$name $fields)|+
    };
    ($($name:ident)|+) => {
        $(Instruction::$name)|+
    };
}

pub(in crate::optimizer) fn effect_on(
    chunk: &Chunk,
    instruction: Instruction,
    register: Register,
) -> Effect {
    let reads = |candidate| candidate == register;
    let writes = |candidate| candidate == register;
    let window = |first: Register, count: usize| {
        let index = register.index();
        let first = first.index();
        index >= first && usize::from(index - first) < count
    };

    let read_then_write = |read: bool, write: bool| match (read, write) {
        (true, true) => Effect::ReadWrite,
        (true, false) => Effect::Read,
        (false, true) => Effect::Write,
        (false, false) => Effect::None,
    };

    match instruction {
        instructions!(
            Move | Negate | UnaryPlus | BitwiseNot | IntBitwiseNot | Not | Length | StringLength
                | CloneObject | AddImmediate | SubtractImmediate | IntMultiplyImmediate
                | IntModuloImmediate | FloatMultiplyConstant | ConcatenateConstant | Is | AsCheck
                | AsOrNull;
            { destination, source, .. }
        ) => read_then_write(reads(source), writes(destination)),
        Instruction::MoveOwned {
            destination,
            source,
        } => read_then_write(reads(source), writes(source) || writes(destination)),
        Instruction::IntAddAssign { target, source } => {
            read_then_write(reads(target) || reads(source), writes(target))
        }
        Instruction::FloatDifferenceAdd {
            destination,
            first_operand,
            addend,
        } => read_then_write(
            window(first_operand, 2) || reads(addend),
            writes(destination),
        ),
        Instruction::FloatScaleProductAdd {
            destination,
            first_operand,
            ..
        } => read_then_write(window(first_operand, 3), writes(destination)),
        instructions!(
            Add | Subtract | Multiply | IntAdd | IntSubtract | IntMultiply | IntModulo
                | IntBitwiseAnd | IntBitwiseOr | IntBitwiseXor | IntShiftLeft | IntShiftRight
                | FloatAdd | FloatSubtract | FloatMultiply | Divide | Modulo | Power
                | Concatenate | BitwiseAnd | BitwiseOr | BitwiseXor | ShiftLeft | ShiftRight
                | Equal | NotEqual | LessThan | LessThanOrEqual | GreaterThan
                | GreaterThanOrEqual | Compare;
            { destination, left, right }
        ) => read_then_write(reads(left) || reads(right), writes(destination)),
        Instruction::LoadConstant { destination, .. }
        | Instruction::LoadNull { destination }
        | Instruction::LoadTrue { destination }
        | Instruction::LoadFalse { destination }
        | Instruction::LoadInt { destination, .. }
        | Instruction::NewStatic { destination, .. }
        | Instruction::NewTyped { destination, .. }
        | Instruction::StaticPropertyGet { destination, .. }
        | Instruction::ConstantGet { destination, .. }
        | Instruction::ClassConstantGet { destination, .. }
        | Instruction::CallNamedConstantUnchecked { destination, .. } => {
            if writes(destination) {
                Effect::Write
            } else {
                Effect::None
            }
        }
        instructions!(Jump | NumericRegionJump | ReturnIntUnchecked; { .. })
        | instructions!(ReturnNull | ReturnNullUnchecked | Rethrow | DrainFinalizers) => {
            Effect::None
        }
        Instruction::JumpIfFalse { condition, .. } | Instruction::JumpIfTrue { condition, .. } => {
            if reads(condition) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::JumpIfNull { subject, .. }
        | Instruction::JumpIfNotNull { subject, .. }
        | Instruction::SwitchInt { subject, .. }
        | Instruction::SwitchString { subject, .. }
        | Instruction::SwitchBool { subject, .. }
        | Instruction::SwitchFloat { subject, .. }
        | Instruction::SwitchPattern { subject, .. }
        | Instruction::IntRangeJumpIf { subject, .. }
        | Instruction::IntRangeJumpUnless { subject, .. }
        | Instruction::BoolPatternBranch { subject, .. }
        | Instruction::CheckDefined { subject, .. }
        | Instruction::CheckDestructure { subject, .. }
        | Instruction::ThrowUnhandledMatch { subject } => {
            if reads(subject) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::SwitchTuplePattern {
            first_element,
            element_count,
            ..
        } => {
            if window(first_element, usize::from(element_count.value())) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(
            JumpUnless | IntJumpUnless | StringJumpUnless | NumericLoop | IntNumericLoop;
            { left, right, .. }
        ) => {
            if reads(left) || reads(right) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::PreparedIntNumericLoop { descriptor, .. } => {
            let descriptor = chunk.prepared_int_loop_descriptor(descriptor);
            let register_mask = 1u64.checked_shl(u32::from(register.index())).unwrap_or(0);
            read_then_write(
                reads(descriptor.counter)
                    || reads(descriptor.limit)
                    || descriptor.float_registers & register_mask != 0,
                writes(descriptor.counter) || descriptor.float_registers & register_mask != 0,
            )
        }
        Instruction::IntStepLoop { descriptor, .. } => {
            let descriptor = chunk.int_step_loop_descriptor(descriptor);
            read_then_write(
                reads(descriptor.counter) || reads(descriptor.limit) || reads(descriptor.step),
                writes(descriptor.counter),
            )
        }
        Instruction::JumpUnlessConstant { source, .. }
        | Instruction::IntJumpUnlessImmediate { source, .. }
        | Instruction::Return { source }
        | Instruction::ReturnUnchecked { source }
        | Instruction::ReturnReferenceUnchecked { source }
        | Instruction::ReturnScalarUnchecked { source }
        | Instruction::Throw { source }
        | Instruction::CheckDiscardedResult { source }
        | Instruction::CheckSoleReference { source, .. } => {
            if reads(source) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(StringByteJumpUnlessEqual | StringByteJumpUnlessNotEqual; {
            container, index, ..
        }) => {
            if reads(container) || reads(index) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::IncrementJump { target, .. } => read_then_write(reads(target), writes(target)),
        instructions!(CounterLoop | IntCounterLoop; { counter, limit, .. }) => {
            read_then_write(reads(counter) || reads(limit), writes(counter))
        }
        instructions!(Squares | FloatSquares; {
            first_destination,
            first_source,
            second_source,
        }) => read_then_write(
            reads(first_source) || reads(second_source),
            writes(first_destination)
                || first_destination
                    .index()
                    .checked_add(1)
                    .is_some_and(|index| writes(Register::new(index))),
        ),
        Instruction::FloatSquaresSum {
            first_destination,
            first_source,
            second_source,
        } => read_then_write(
            reads(first_source) || reads(second_source),
            (0..3).any(|offset| {
                first_destination
                    .index()
                    .checked_add(offset)
                    .is_some_and(|index| writes(Register::new(index)))
            }),
        ),
        Instruction::FloatSquaresSumBranch { descriptor, .. } => {
            let descriptor = chunk.float_squares_sum_branch_descriptor(descriptor);

            read_then_write(
                reads(descriptor.first_source) || reads(descriptor.second_source),
                writes(descriptor.sum_destination)
                    || writes(descriptor.first_square_destination)
                    || writes(descriptor.second_square_destination),
            )
        }
        Instruction::FloatPairUpdate { descriptor } => {
            let descriptor = chunk.float_pair_update_descriptor(descriptor);

            read_then_write(
                window(descriptor.first_operand, 3)
                    || window(descriptor.second_operand, 2)
                    || reads(descriptor.second_addend),
                writes(descriptor.first_destination) || writes(descriptor.second_destination),
            )
        }
        instructions!(NewVec | NewTuple; {
            element_count,
            destination,
            first_element,
        }) => read_then_write(
            window(first_element, usize::from(element_count.value())),
            writes(destination),
        ),
        Instruction::NewDict {
            pair_count,
            destination,
            first_pair,
        } => read_then_write(
            window(first_pair, usize::from(pair_count.value()) * 2),
            writes(destination),
        ),
        instructions!(
            IndexGet | VecIndexGet | DictIndexGetIntKey | DictIndexGetStringKey | StringIndexGet
                | StringByteEqual | StringByteNotEqual | StringByteLessThan
                | StringByteLessThanOrEqual | StringByteGreaterThan | StringByteGreaterThanOrEqual;
            { destination, container, index, .. }
        ) => read_then_write(reads(container) || reads(index), writes(destination)),
        instructions!(IndexSet | VecIndexSet | DictIndexSetIntKey | DictIndexSetStringKey | DictIndexSet; {
            container,
            index,
            value,
        })
        | Instruction::IndexAddAssign {
            container,
            index,
            value,
            ..
        } => {
            if reads(container) || reads(index) || reads(value) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(PropertyIndexUpdate | PropertyIndexUpdateUnchecked; {
            object, operand, ..
        }) => {
            if reads(object) || reads(operand) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(PropertyRemove | PropertyRemoveUnchecked; {
            object,
            destination,
            mode,
            ..
        }) => read_then_write(
            reads(object) || (mode.uses_operand() && reads(Register::new(destination.index() + 1))),
            writes(destination),
        ),
        instructions!(PropertyIndexSet | PropertyIndexSetUnchecked | PropertyFillIntRange; {
            object,
            first_operand,
            ..
        }) => {
            if reads(object) || window(first_operand, 2) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(PropertyStep | PropertyStepUnchecked; { object, .. }) => {
            if reads(object) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(PropertyAdd | PropertyAddUnchecked; { object, source, .. }) => {
            if reads(object) || reads(source) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::Append { container, value }
        | Instruction::VecAppend { container, value }
        | Instruction::Spread { container, value }
        | Instruction::PropertySet {
            object: container,
            value,
            ..
        }
        | Instruction::PropertySetUnchecked {
            object: container,
            value,
            ..
        }
        | Instruction::PropertyInitRaw {
            object: container,
            value,
            ..
        } => {
            if reads(container) || reads(value) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::InitializeProperties {
            object, descriptor, ..
        } => {
            let descriptor = chunk.property_initialization_descriptor(descriptor);
            read_then_write(
                (!descriptor.allocates && reads(object))
                    || descriptor.entries.iter().any(|entry| reads(entry.value)),
                descriptor.allocates && writes(object),
            )
        }
        Instruction::ReserveArray {
            container,
            additional,
        } => {
            if reads(container) || reads(additional) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::Contains {
            destination,
            array,
            value,
        }
        | Instruction::ContainsKey {
            destination,
            array,
            key: value,
        }
        | Instruction::NewFilledVec {
            destination,
            value: array,
            size: value,
        } => read_then_write(reads(array) || reads(value), writes(destination)),
        Instruction::Rest {
            destination,
            subject,
            ..
        }
        | Instruction::ElementGet {
            destination,
            subject,
            ..
        } => read_then_write(reads(subject), writes(destination)),
        Instruction::Remove {
            destination,
            container,
            key: index,
        }
        | Instruction::SwapRemove {
            destination,
            container,
            index,
        } => read_then_write(reads(container) || reads(index), writes(destination)),
        instructions!(RemoveFirst | RemoveLast; {
            destination,
            container,
        }) => read_then_write(reads(container), writes(destination)),
        Instruction::NewDynamic {
            destination,
            class_name,
        } => read_then_write(reads(class_name), writes(destination)),
        instructions!(PropertyGet | PropertyGetUnchecked; {
            destination,
            object,
            ..
        }) => read_then_write(reads(object), writes(destination)),
        Instruction::StaticPropertySet { value, .. } => {
            if reads(value) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(CallValue | CallValueUnchecked | CallValueDiscarded; {
            argument_count,
            destination,
            callee,
            first_argument,
        }) => read_then_write(
            reads(callee) || window(first_argument, usize::from(argument_count.value())),
            writes(destination) || window(first_argument, usize::from(argument_count.value())),
        ),
        instructions!(
            CallNamed | CallNamedDiscarded | CallNamedUnchecked | CallMethod
                | CallMethodDiscarded | CallMethodUnchecked | CallStatic | CallStaticDiscarded;
            {
            argument_count,
            destination,
            first_argument,
            ..
            }
        )
        | Instruction::CallSelfUnchecked {
            argument_count,
            destination,
            first_argument,
        } => read_then_write(
            window(first_argument, usize::from(argument_count.value())),
            writes(destination) || window(first_argument, usize::from(argument_count.value())),
        ),
        Instruction::CallMethodDirect {
            argument_count,
            destination,
            first_argument,
            ..
        } => read_then_write(
            window(first_argument, usize::from(argument_count.value())),
            writes(destination),
        ),
        instructions!(CallWithNames | CallWithNamesDiscarded; {
            destination,
            callee,
            descriptor,
        }) => {
            let descriptor = &chunk.call_descriptors[usize::from(descriptor.index())];
            let count = usize::from(descriptor.positional) + descriptor.named.len();
            read_then_write(
                reads(callee) || window(Register::new(callee.index() + 1), count),
                writes(destination) || window(Register::new(callee.index() + 1), count),
            )
        }
        Instruction::ReturnPairUnchecked { first, second } => {
            if reads(first) || reads(second) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        instructions!(Write | WriteLine | WriteError | WriteErrorLine | Debug; {
            value_count,
            first_value,
        }) => {
            if window(first_value, usize::from(value_count.value())) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::MakeClosure {
            capture_count,
            destination,
            first_capture,
            ..
        } => read_then_write(
            window(first_capture, usize::from(capture_count.value())),
            writes(destination),
        ),
        Instruction::MakeBound {
            destination,
            callee,
            descriptor,
        } => {
            let count = chunk.preset_descriptors[usize::from(descriptor.index())]
                .slots
                .len();
            read_then_write(
                reads(callee) || window(Register::new(callee.index() + 1), count),
                writes(destination),
            )
        }
        Instruction::ForeachInit {
            iterator,
            subject,
            reserve,
        } => read_then_write(
            reads(subject) || (reserve != Register::NONE && reads(reserve)),
            writes(iterator),
        ),
        Instruction::ForeachNext {
            iterator,
            key_destination,
            value_destination,
        }
        | Instruction::VecForeachNext {
            iterator,
            key_destination,
            value_destination,
            ..
        }
        | Instruction::DictForeachNext {
            iterator,
            key_destination,
            value_destination,
            ..
        } => read_then_write(
            reads(iterator),
            writes(key_destination) || writes(value_destination),
        ),
        Instruction::Assert {
            operand_count,
            first_value,
            message,
            ..
        } => {
            if window(first_value, usize::from(operand_count.value()) + 1)
                || (message != Register::NONE && reads(message))
            {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::Exit { code } => {
            if code != Register::NONE && reads(code) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::Panic { .. } => Effect::None,
        Instruction::Require {
            destination, path, ..
        } => read_then_write(reads(path), writes(destination)),
        Instruction::FillDefault { target, .. } => {
            if reads(target) {
                Effect::Read
            } else {
                Effect::None
            }
        }
        Instruction::Clear { target } => {
            if reads(target) {
                Effect::ReadWrite
            } else {
                Effect::None
            }
        }
    }
}

pub(in crate::optimizer) fn overwrites_register(
    chunk: &Chunk,
    instruction: Instruction,
    register: Register,
) -> bool {
    effect_on(chunk, instruction, register).writes()
}
