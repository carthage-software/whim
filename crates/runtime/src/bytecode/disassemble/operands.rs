use crate::bytecode::disassemble::Chunk;
use crate::bytecode::disassemble::IndexAddMode;
use crate::bytecode::disassemble::Instruction;
use crate::bytecode::disassemble::Register;
use crate::bytecode::disassemble::render::cache_reference;
use crate::bytecode::disassemble::render::call_reference;
use crate::bytecode::disassemble::render::constant_reference;
use crate::bytecode::disassemble::render::descriptor_reference;
use crate::bytecode::disassemble::render::jump;
use crate::bytecode::disassemble::render::preset_reference;
use crate::bytecode::disassemble::render::register;
use crate::bytecode::disassemble::render::short_jump;
use crate::bytecode::disassemble::render::table_reference;
use crate::bytecode::disassemble::render::window;
use crate::bytecode::instruction::operands::CollectionValueMode;
use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::bytecode::instruction::operands::PropertyReadMode;
use crate::bytecode::instruction::operands::PropertyRemoveMode;

macro_rules! instructions {
    ($($name:ident)|+ ; $fields:tt) => {
        $(Instruction::$name $fields)|+
    };
    ($($name:ident)|+) => {
        $(Instruction::$name)|+
    };
}

#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive match keeps every opcode rendering together"
)]
pub(crate) fn operands(chunk: &Chunk, index: usize, instruction: Instruction) -> String {
    match instruction {
        instructions!(
            Move | MoveOwned | Negate | UnaryPlus | BitwiseNot | IntBitwiseNot | Not | Length
                | StringLength | CloneObject;
            { destination, source }
        ) => format!(" {}, {}", register(destination), register(source)),
        Instruction::IntAddAssign { target, source } => {
            format!(" {}, {}", register(target), register(source))
        }
        Instruction::LoadConstant {
            destination,
            constant,
        } => format!(
            " {}, {}",
            register(destination),
            constant_reference(chunk, constant)
        ),
        instructions!(LoadNull | LoadTrue | LoadFalse; { destination }) => {
            format!(" {}", register(destination))
        }
        Instruction::LoadInt {
            destination,
            immediate,
        } => format!(" {}, {}", register(destination), immediate.value()),
        instructions!(
            Add | Subtract | Multiply | IntAdd | IntSubtract | IntMultiply | IntModulo
                | FloatAdd | FloatSubtract | FloatMultiply | Divide | Modulo | Power
                | Concatenate | BitwiseAnd | IntBitwiseAnd | BitwiseOr | IntBitwiseOr
                | BitwiseXor | IntBitwiseXor | ShiftLeft | IntShiftLeft | ShiftRight
                | IntShiftRight | Equal | NotEqual | LessThan | LessThanOrEqual | GreaterThan
                | GreaterThanOrEqual | Compare;
            { destination, left, right }
        ) => format!(
            " {}, {}, {}",
            register(destination),
            register(left),
            register(right)
        ),
        instructions!(
            AddImmediate | SubtractImmediate | IntMultiplyImmediate | IntModuloImmediate;
            {
                destination,
                source,
                immediate,
            }
        ) => format!(
            " {}, {}, {}",
            register(destination),
            register(source),
            immediate.value()
        ),
        Instruction::FloatMultiplyConstant {
            destination,
            source,
            constant,
        } => format!(
            " {}, {}, {}",
            register(destination),
            register(source),
            constant_reference(chunk, constant)
        ),
        Instruction::FloatDifferenceAdd {
            destination,
            first_operand,
            addend,
        } => format!(
            " {}, {}, {}, {}",
            register(destination),
            register(first_operand),
            register(Register::new(first_operand.index() + 1)),
            register(addend)
        ),
        Instruction::FloatScaleProductAdd {
            destination,
            first_operand,
            constant,
        } => format!(
            " {}, {}, {}",
            register(destination),
            window(first_operand, 3),
            constant_reference(chunk, constant)
        ),
        Instruction::FloatPairUpdate { descriptor } => {
            format!(" descriptor[{}]", descriptor.index())
        }
        instructions!(Jump | NumericRegionJump; { offset }) => {
            format!(" {}", jump(index, offset))
        }
        instructions!(JumpIfFalse | JumpIfTrue; { condition, offset }) => {
            format!(" {} {}", register(condition), jump(index, offset))
        }
        instructions!(JumpIfNull | JumpIfNotNull; { subject, offset }) => {
            format!(" {} {}", register(subject), jump(index, offset))
        }
        instructions!(
            JumpUnless | IntJumpUnless | StringJumpUnless | NumericLoop | IntNumericLoop;
            { comparison, left, right, offset }
        ) => format!(
            " {} {}, {} {}",
            comparison.operator(),
            register(left),
            register(right),
            short_jump(index, offset)
        ),
        Instruction::JumpUnlessConstant {
            comparison,
            source,
            constant,
            offset,
        } => format!(
            " {} {}, {} {}",
            comparison.operator(),
            register(source),
            constant_reference(chunk, constant),
            short_jump(index, offset)
        ),
        instructions!(IntRangeJumpIf | IntRangeJumpUnless; {
            subject,
            descriptor,
            offset,
        }) => format!(
            " {}, {} {}",
            register(subject),
            descriptor_reference(chunk, descriptor),
            short_jump(index, offset)
        ),
        Instruction::BoolPatternBranch {
            subject,
            false_offset,
            default_offset,
        } => format!(
            " {}, false {}, default {}",
            register(subject),
            short_jump(index, false_offset),
            short_jump(index, default_offset)
        ),
        Instruction::StringByteJumpUnlessEqual {
            container,
            index: string_index,
            byte,
            offset,
        } => format!(
            " == {}[{}], '{}' {}",
            register(container),
            register(string_index),
            char::from(byte).escape_default(),
            short_jump(index, offset)
        ),
        Instruction::StringByteJumpUnlessNotEqual {
            container,
            index: string_index,
            byte,
            offset,
        } => format!(
            " != {}[{}], '{}' {}",
            register(container),
            register(string_index),
            char::from(byte).escape_default(),
            short_jump(index, offset)
        ),
        Instruction::StringByteEqual {
            destination,
            container,
            index,
            byte,
        } => string_byte_comparison(destination, container, index, byte, "=="),
        Instruction::StringByteNotEqual {
            destination,
            container,
            index,
            byte,
        } => string_byte_comparison(destination, container, index, byte, "!="),
        Instruction::StringByteLessThan {
            destination,
            container,
            index,
            byte,
        } => string_byte_comparison(destination, container, index, byte, "<"),
        Instruction::StringByteLessThanOrEqual {
            destination,
            container,
            index,
            byte,
        } => string_byte_comparison(destination, container, index, byte, "<="),
        Instruction::StringByteGreaterThan {
            destination,
            container,
            index,
            byte,
        } => string_byte_comparison(destination, container, index, byte, ">"),
        Instruction::StringByteGreaterThanOrEqual {
            destination,
            container,
            index,
            byte,
        } => string_byte_comparison(destination, container, index, byte, ">="),
        Instruction::IntJumpUnlessImmediate {
            comparison,
            source,
            immediate,
            offset,
        } => format!(
            " {} {}, {} {}",
            comparison.operator(),
            register(source),
            immediate.value(),
            short_jump(index, offset)
        ),
        Instruction::IncrementJump {
            target,
            immediate,
            offset,
        } => format!(
            " {}, {} {}",
            register(target),
            immediate.value(),
            short_jump(index, offset)
        ),
        instructions!(Squares | FloatSquares; {
            first_destination,
            first_source,
            second_source,
        }) => format!(
            " {}..{}, {}, {}",
            register(first_destination),
            register(Register::new(first_destination.index().saturating_add(1))),
            register(first_source),
            register(second_source)
        ),
        Instruction::FloatSquaresSum {
            first_destination,
            first_source,
            second_source,
        } => format!(
            " {}..{}, {}, {}",
            register(first_destination),
            register(Register::new(first_destination.index() + 2)),
            register(first_source),
            register(second_source)
        ),
        Instruction::FloatSquaresSumBranch { descriptor, offset } => format!(
            " descriptor[{}] {}",
            descriptor.index(),
            jump(index, offset)
        ),
        Instruction::PreparedIntNumericLoop { descriptor, offset } => format!(
            " descriptor[{}] {}",
            descriptor.index(),
            short_jump(index, offset)
        ),
        Instruction::IntStepLoop { descriptor, offset } => format!(
            " descriptor[{}] {}",
            descriptor.index(),
            short_jump(index, offset)
        ),
        instructions!(CounterLoop | IntCounterLoop; {
            comparison,
            counter,
            limit,
            offset,
        }) => format!(
            " {}, {} {}, {}",
            register(counter),
            comparison.operator(),
            register(limit),
            short_jump(index, offset)
        ),
        instructions!(
            SwitchInt
                | SwitchString
                | SwitchBool
                | SwitchFloat
                | SwitchPattern;
            { subject, table }
        ) => {
            format!(" {}, {}", register(subject), table_reference(table))
        }
        Instruction::SwitchTuplePattern {
            first_element,
            element_count,
            table,
        } => format!(
            " {}, {}",
            window(first_element, u32::from(element_count.value())),
            table_reference(table)
        ),
        Instruction::CheckDefined { subject, name } => format!(
            " {}, {}",
            register(subject),
            constant_reference(chunk, name)
        ),
        instructions!(NewVec | NewTuple; {
            element_count,
            destination,
            first_element,
        }) => format!(
            " {}, {}",
            register(destination),
            window(first_element, u32::from(element_count.value()))
        ),
        Instruction::NewDict {
            pair_count,
            destination,
            first_pair,
        } => format!(
            " {}, {}",
            register(destination),
            window(first_pair, 2 * u32::from(pair_count.value()))
        ),
        instructions!(IndexGet | StringIndexGet; {
            destination,
            container,
            index: subscript,
        }) => format!(
            " {}, {}, {}",
            register(destination),
            register(container),
            register(subscript)
        ),
        Instruction::DictIndexGetStringKey {
            destination,
            container,
            index: subscript,
            value_mode,
        } => format!(
            " {}, {}, {}, {:?}",
            register(destination),
            register(container),
            register(subscript),
            value_mode,
        ),
        instructions!(VecIndexGet | DictIndexGetIntKey; {
            destination,
            container,
            index: subscript,
            value_mode,
        }) => format!(
            " {}, {}, {}{}",
            register(destination),
            register(container),
            register(subscript),
            match value_mode {
                CollectionValueMode::Generic => "",
                CollectionValueMode::Int => ", int",
                CollectionValueMode::Float => ", float",
            }
        ),
        instructions!(
            IndexSet | VecIndexSet | DictIndexSetIntKey | DictIndexSetStringKey | DictIndexSet;
            {
            container,
            index: subscript,
            value,
            }
        ) => format!(
            " {}, {}, {}",
            register(container),
            register(subscript),
            register(value)
        ),
        Instruction::IndexAddAssign {
            container,
            index: subscript,
            value,
            mode,
        } => format!(
            " {}, {}, {}{}",
            register(container),
            register(subscript),
            register(value),
            match mode {
                IndexAddMode::Generic => "",
                IndexAddMode::DictAnyKeyIntValue => ", dict<_, int>",
                IndexAddMode::DictStringKeyIntValue => ", dict<string, int>",
            }
        ),
        instructions!(Append | VecAppend | Spread; { container, value }) => {
            format!(" {}, {}", register(container), register(value))
        }
        Instruction::ReserveCollection {
            container,
            additional,
        } => format!(" {}, {}", register(container), register(additional)),
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
        } => format!(
            " {}, {}, {}",
            register(destination),
            register(array),
            register(value)
        ),
        Instruction::Remove {
            destination,
            container,
            key: index,
        }
        | Instruction::SwapRemove {
            destination,
            container,
            index,
        } => format!(
            " {}, {}, {}",
            register(destination),
            register(container),
            register(index)
        ),
        instructions!(RemoveFirst | RemoveLast; {
            destination,
            container,
        }) => format!(" {}, {}", register(destination), register(container)),
        Instruction::CheckDestructure {
            subject,
            required,
            arity,
            rest,
        } => {
            let bound = if rest {
                format!("at least {}", required.value())
            } else if required == arity {
                format!("exactly {}", arity.value())
            } else {
                format!("between {} and {}", required.value(), arity.value())
            };
            format!(" {}, {bound}", register(subject))
        }
        Instruction::Rest {
            destination,
            subject,
            from,
        } => format!(
            " {}, {}, from {}",
            register(destination),
            register(subject),
            from.value()
        ),
        Instruction::ElementGet {
            destination,
            subject,
            index: element,
        } => format!(
            " {}, {}, {}",
            register(destination),
            register(subject),
            element.value()
        ),
        Instruction::NewDynamic {
            destination,
            class_name,
        } => format!(" {}, {}", register(destination), register(class_name)),
        Instruction::NewTyped {
            destination,
            descriptor,
        } => format!(
            " {}, {}",
            register(destination),
            descriptor_reference(chunk, descriptor)
        ),
        Instruction::PropertyGet {
            destination,
            object,
            cache,
        } => format!(
            " {}, {}, {}",
            register(destination),
            register(object),
            cache_reference(chunk, cache)
        ),
        Instruction::PropertyGetUnchecked {
            destination,
            object,
            slot,
            value_mode,
        } => format!(
            " {}, {}, slot[{}]{}",
            register(destination),
            register(object),
            slot.index(),
            match value_mode {
                PropertyReadMode::Clone => "",
                PropertyReadMode::Take => ", take",
            }
        ),
        instructions!(PropertySet | PropertyInitRaw; {
            object,
            value,
            cache,
        }) => format!(
            " {}, {}, {}",
            register(object),
            register(value),
            cache_reference(chunk, cache)
        ),
        Instruction::PropertySetUnchecked {
            object,
            value,
            slot,
            value_mode,
        } => format!(
            " {}, {}, slot[{}]{}",
            register(object),
            register(value),
            slot.index(),
            match (value_mode.moves(), value_mode.fresh_receiver()) {
                (false, false) => "",
                (true, false) => ", move",
                (false, true) => ", fresh",
                (true, true) => ", fresh, move",
            }
        ),
        Instruction::InitializeProperties {
            object,
            cache,
            descriptor,
        } => format!(
            " {}, {}, property_initializers[{}]",
            register(object),
            cache_reference(chunk, cache),
            descriptor.index()
        ),
        Instruction::PropertyIndexUpdate {
            object,
            operand,
            cache,
            mode,
        } => format!(
            " {}, {}, {}, {}",
            register(object),
            register(operand),
            cache_reference(chunk, cache),
            match mode {
                PropertyIndexUpdateMode::Increment => "increment",
                PropertyIndexUpdateMode::Remove => "remove",
                PropertyIndexUpdateMode::Append => "append",
            }
        ),
        Instruction::PropertyIndexUpdateUnchecked {
            object,
            operand,
            slot,
            mode,
        } => format!(
            " {}, {}, slot[{}], {}",
            register(object),
            register(operand),
            slot.index(),
            match mode {
                PropertyIndexUpdateMode::Increment => "increment",
                PropertyIndexUpdateMode::Remove => "remove",
                PropertyIndexUpdateMode::Append => "append",
            }
        ),
        Instruction::PropertyRemove {
            object,
            destination,
            cache,
            mode,
        } => property_remove(object, destination, &cache_reference(chunk, cache), mode),
        Instruction::PropertyRemoveUnchecked {
            object,
            destination,
            slot,
            mode,
        } => property_remove(
            object,
            destination,
            &format!("slot[{}]", slot.index()),
            mode,
        ),
        Instruction::PropertyIndexSet {
            object,
            first_operand,
            cache,
        } => format!(
            " {}, {}, {}, {}",
            register(object),
            register(first_operand),
            register(Register::new(first_operand.index() + 1)),
            cache_reference(chunk, cache),
        ),
        Instruction::PropertyIndexSetUnchecked {
            object,
            first_operand,
            slot,
        } => format!(
            " {}, {}, {}, slot[{}]",
            register(object),
            register(first_operand),
            register(Register::new(first_operand.index() + 1)),
            slot.index(),
        ),
        Instruction::PropertyFillIntRange {
            object,
            first_operand,
            cache,
        } => format!(
            " {}, {}, {}",
            register(object),
            window(first_operand, 2),
            cache_reference(chunk, cache)
        ),
        Instruction::PropertyStep {
            object,
            cache,
            immediate,
        } => format!(
            " {}, {}, {}",
            register(object),
            cache_reference(chunk, cache),
            immediate.value()
        ),
        Instruction::PropertyStepUnchecked {
            object,
            slot,
            immediate,
        } => format!(
            " {}, slot[{}], {}",
            register(object),
            slot.index(),
            immediate.value()
        ),
        Instruction::PropertyAdd {
            object,
            source,
            cache,
        } => format!(
            " {}, {}, {}",
            register(object),
            register(source),
            cache_reference(chunk, cache)
        ),
        Instruction::PropertyAddUnchecked {
            object,
            source,
            slot,
        } => format!(
            " {}, {}, slot[{}]",
            register(object),
            register(source),
            slot.index()
        ),
        instructions!(
            NewStatic | StaticPropertyGet | ConstantGet | ClassConstantGet;
            { destination, cache }
        ) => format!(
            " {}, {}",
            register(destination),
            cache_reference(chunk, cache)
        ),
        Instruction::StaticPropertySet { cache, value } => {
            format!(" {}, {}", cache_reference(chunk, cache), register(value))
        }
        instructions!(CallValue | CallValueUnchecked | CallValueDiscarded; {
            argument_count,
            destination,
            callee,
            first_argument,
        }) => format!(
            " {}, {}, {}",
            register(destination),
            register(callee),
            window(first_argument, u32::from(argument_count.value()))
        ),
        instructions!(
            CallNamed | CallNamedDiscarded | CallNamedUnchecked | CallMethod
                | CallMethodDiscarded | CallMethodUnchecked | CallMethodDirect | CallStatic
                | CallStaticDiscarded;
            {
            argument_count,
            destination,
            first_argument,
            cache,
            }
        ) => format!(
            " {}, {}, {}",
            register(destination),
            cache_reference(chunk, cache),
            window(first_argument, u32::from(argument_count.value()))
        ),
        Instruction::CallNamedConstantUnchecked {
            destination,
            constant,
            cache,
            borrowed,
        } => format!(
            " {}, {}, {}{}",
            register(destination),
            cache_reference(chunk, cache),
            constant_reference(chunk, constant),
            if borrowed { ", borrowed" } else { "" }
        ),
        Instruction::CallSelfUnchecked {
            argument_count,
            destination,
            first_argument,
        } => format!(
            " {}, self, {}",
            register(destination),
            window(first_argument, u32::from(argument_count.value()))
        ),
        instructions!(CallWithNames | CallWithNamesDiscarded; {
            destination,
            callee,
            descriptor,
        }) => format!(
            " {}, {}, {}",
            register(destination),
            register(callee),
            call_reference(descriptor)
        ),
        Instruction::MakeBound {
            destination,
            callee,
            descriptor,
        } => format!(
            " {}, {}, {}",
            register(destination),
            register(callee),
            preset_reference(chunk, descriptor)
        ),
        Instruction::FillDefault { target, offset } => {
            format!(" {} {}", register(target), jump(index, offset))
        }
        instructions!(
            Return | ReturnUnchecked | ReturnReferenceUnchecked | ReturnScalarUnchecked | Throw;
            { source }
        ) => {
            format!(" {}", register(source))
        }
        Instruction::CheckDiscardedResult { source } => format!(" {}", register(source)),
        Instruction::ReturnPairUnchecked { first, second } => {
            format!(" {}, {}", register(first), register(second))
        }
        Instruction::ReturnIntUnchecked { immediate } => {
            format!(" {}", immediate.value())
        }
        instructions!(ReturnNull | ReturnNullUnchecked | Rethrow | DrainFinalizers) => {
            String::new()
        }
        Instruction::MakeClosure {
            capture_count,
            destination,
            prototype,
            first_capture,
        } => format!(
            " {}, {}, {}",
            register(destination),
            constant_reference(chunk, prototype),
            window(first_capture, u32::from(capture_count.value()))
        ),
        Instruction::Is {
            destination,
            source,
            descriptor,
        }
        | Instruction::AsCheck {
            destination,
            source,
            descriptor,
            ..
        }
        | Instruction::AsOrNull {
            destination,
            source,
            descriptor,
        } => format!(
            " {}, {}, {}",
            register(destination),
            register(source),
            descriptor_reference(chunk, descriptor)
        ),
        Instruction::ThrowUnhandledMatch { subject } => format!(" {}", register(subject)),
        Instruction::ForeachInit {
            iterator,
            subject,
            reserve,
        } => {
            if reserve == Register::NONE {
                format!(" {}, {}", register(iterator), register(subject))
            } else {
                format!(
                    " {}, {}, reserve {}",
                    register(iterator),
                    register(subject),
                    register(reserve),
                )
            }
        }
        Instruction::ForeachNext {
            iterator,
            key_destination,
            value_destination,
        } => format!(
            " {}, {}, {}",
            register(iterator),
            register(key_destination),
            register(value_destination)
        ),
        instructions!(VecForeachNext | DictForeachNext; {
            iterator,
            key_destination,
            value_destination,
            value_mode,
        }) => format!(
            " {}, {}, {}{}",
            register(iterator),
            register(key_destination),
            register(value_destination),
            match value_mode {
                CollectionValueMode::Generic => "",
                CollectionValueMode::Int => ", int",
                CollectionValueMode::Float => ", float",
            }
        ),
        instructions!(Write | WriteLine | WriteError | WriteErrorLine | Debug; {
            value_count,
            first_value,
        }) => format!(" {}", window(first_value, u32::from(value_count.value()))),
        Instruction::Assert {
            operand_count,
            first_value,
            message,
            text,
        } => format!(
            " {}, message {}, text #{}",
            window(first_value, u32::from(operand_count.value()) + 1),
            register(message),
            text.index(),
        ),
        Instruction::Exit { code } => format!(" code {}", register(code)),
        Instruction::Panic { message } => format!(" message #{}", message.index()),
        Instruction::Require {
            once,
            destination,
            path,
        } => {
            let suffix = if once { ", once" } else { "" };
            format!(" {}, {}{suffix}", register(destination), register(path))
        }
        Instruction::Clear { target } => format!(" {}", register(target)),
        Instruction::CheckSoleReference {
            source,
            message,
            chain_previous,
        } => {
            let suffix = if chain_previous {
                ", chain previous"
            } else {
                ""
            };
            format!(
                " {}, message #{}{suffix}",
                register(source),
                message.index()
            )
        }
    }
}

fn property_remove(
    object: Register,
    destination: Register,
    property: &str,
    mode: PropertyRemoveMode,
) -> String {
    match mode {
        PropertyRemoveMode::Key => format!(
            " {}, {}, {}, {}, key",
            register(destination),
            register(object),
            register(Register::new(destination.index() + 1)),
            property,
        ),
        PropertyRemoveMode::Swap => format!(
            " {}, {}, {}, {}, swap",
            register(destination),
            register(object),
            register(Register::new(destination.index() + 1)),
            property,
        ),
        PropertyRemoveMode::First => format!(
            " {}, {}, {}, first",
            register(destination),
            register(object),
            property,
        ),
        PropertyRemoveMode::Last => format!(
            " {}, {}, {}, last",
            register(destination),
            register(object),
            property,
        ),
    }
}

fn string_byte_comparison(
    destination: Register,
    container: Register,
    index: Register,
    byte: u8,
    operator: &str,
) -> String {
    format!(
        " {}, {operator} {}[{}], '{}'",
        register(destination),
        register(container),
        register(index),
        char::from(byte).escape_default(),
    )
}
