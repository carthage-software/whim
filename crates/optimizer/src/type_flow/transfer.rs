//! The per-instruction fact transfer function.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayKind;
use whim_bytecode::instruction::operands::ArrayValueMode;
use whim_bytecode::instruction::operands::Comparison;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::Register;

use crate::type_flow::ALL;
use crate::type_flow::BOOL;
use crate::type_flow::CALLABLE;
use crate::type_flow::Cell;
use crate::type_flow::DICTIONARY;
use crate::type_flow::FLOAT;
use crate::type_flow::Fact;
use crate::type_flow::INT;
use crate::type_flow::NO_ORIGIN;
use crate::type_flow::NULL;
use crate::type_flow::NUMERIC;
use crate::type_flow::OBJECT;
use crate::type_flow::STRING;
use crate::type_flow::THIS_ORIGIN;
use crate::type_flow::TUPLE;
use crate::type_flow::UINT;
use crate::type_flow::VECTOR;
use crate::type_flow::descriptor_mask;
use crate::type_flow::unary_numeric_result;
use crate::type_flow::with_origin;

macro_rules! instructions {
    ($($name:ident)|+ ; $fields:tt) => {
        $(Instruction::$name $fields)|+
    };
}

pub(crate) fn transfer(
    chunk: &Chunk,
    index: usize,
    state: &mut [Fact],
    array_elements: Option<&[u16]>,
    array_keys: Option<&[u16]>,
) {
    let origin = index as u32 + 1;
    let state = Cell::from_mut(state).as_slice_of_cells();
    let read = |register: Register| state[usize::from(register.index())].get();
    let write = |register: Register, fact: Fact| {
        state[usize::from(register.index())].set(fact);
    };
    let clear_window = |first: Register, count: usize| {
        let empty = Fact::UNKNOWN.release_is_unobservable();
        for offset in 0..count {
            write(Register::new(first.index() + offset as u16), empty);
        }
    };

    match chunk.code[index] {
        Instruction::Move {
            destination,
            source,
        } => {
            let mut fact = read(source);
            if destination.index() != 0 && fact.origin == THIS_ORIGIN {
                fact = fact.release_is_unobservable();
            }
            write(destination, fact);
        }
        Instruction::MoveOwned {
            destination,
            source,
        } => write(destination, read(source)),
        Instruction::LoadConstant {
            destination,
            constant,
        } => {
            let literal = &chunk.constants[usize::from(constant.index())];
            let fact = match literal {
                Literal::Int(value) => Fact::integer(*value, origin),
                _ => Fact::with_origin(literal_mask(literal), origin),
            };
            write(destination, fact);
        }
        Instruction::LoadNull { destination } => {
            write(destination, Fact::with_origin(NULL, origin))
        }
        Instruction::LoadTrue { destination }
        | Instruction::LoadFalse { destination }
        | Instruction::Equal { destination, .. }
        | Instruction::NotEqual { destination, .. }
        | Instruction::LessThan { destination, .. }
        | Instruction::LessThanOrEqual { destination, .. }
        | Instruction::GreaterThan { destination, .. }
        | Instruction::GreaterThanOrEqual { destination, .. }
        | Instruction::StringByteEqual { destination, .. }
        | Instruction::StringByteNotEqual { destination, .. }
        | Instruction::StringByteLessThan { destination, .. }
        | Instruction::StringByteLessThanOrEqual { destination, .. }
        | Instruction::StringByteGreaterThan { destination, .. }
        | Instruction::StringByteGreaterThanOrEqual { destination, .. }
        | Instruction::Not { destination, .. }
        | Instruction::Is { destination, .. }
        | Instruction::Contains { destination, .. }
        | Instruction::ContainsKey { destination, .. } => {
            write(destination, Fact::with_origin(BOOL, origin))
        }
        Instruction::LoadInteger {
            kind: IntegerKind::I64,
            destination,
            immediate,
        } => write(
            destination,
            Fact::integer(i64::from(immediate.as_int()), origin),
        ),
        instructions!(LoadInteger | IntegerMultiplyImmediate | IntegerModuloImmediate; { destination, kind: IntegerKind::U64, .. })
        | instructions!(Add | Subtract | Multiply | Modulo | BitwiseAnd | BitwiseOr | BitwiseXor | BitwiseNot | ShiftLeft | ShiftRight | AddImmediate | SubtractImmediate | Step; { destination, kind: Some(IntegerKind::U64), .. }) =>
        {
            write(destination, Fact::with_origin(UINT, origin));
        }
        Instruction::IntegerAddAssign {
            kind: IntegerKind::U64,
            target,
            ..
        } => write(target, Fact::with_origin(UINT, origin)),
        Instruction::UintCounterLoop { counter, .. } => {
            write(counter, Fact::with_origin(UINT, origin))
        }
        Instruction::Add {
            kind: None,
            destination,
            left,
            right,
        }
        | Instruction::Multiply {
            kind: None,
            destination,
            left,
            right,
        } => {
            let left = read(left);
            let right = read(right);
            let mut fact = numeric_result(left, right);
            fact.non_negative = left.non_negative && right.non_negative;
            write(destination, with_origin(fact, origin));
        }
        Instruction::Subtract {
            kind: None,
            destination,
            left,
            right,
        } => write(
            destination,
            with_origin(numeric_result(read(left), read(right)), origin),
        ),
        Instruction::Power {
            destination,
            left,
            right,
        } => {
            let exponent = read(right);
            let mut fact = numeric_result(read(left), exponent);
            if fact.mask == INT && !exponent.non_negative {
                fact = Fact::known(INT | FLOAT);
            }

            write(destination, with_origin(fact, origin));
        }
        Instruction::FloatAdd { destination, .. }
        | Instruction::FloatSubtract { destination, .. }
        | Instruction::FloatMultiply { destination, .. }
        | Instruction::FloatMultiplyConstant { destination, .. }
        | Instruction::FloatDifferenceAdd { destination, .. }
        | Instruction::FloatScaleProductAdd { destination, .. }
        | Instruction::Divide { destination, .. } => {
            write(destination, Fact::with_origin(FLOAT, origin))
        }
        Instruction::Add {
            kind: Some(IntegerKind::I64),
            destination,
            left,
            right,
        }
        | Instruction::Multiply {
            kind: Some(IntegerKind::I64),
            destination,
            left,
            right,
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(left).non_negative && read(right).non_negative;
            write(destination, with_origin(fact, origin));
        }
        Instruction::Modulo {
            kind: Some(IntegerKind::I64),
            destination,
            left,
            ..
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(left).non_negative;
            write(destination, with_origin(fact, origin));
        }
        Instruction::AddImmediate {
            kind: Some(IntegerKind::I64),
            destination,
            source,
            immediate,
        }
        | Instruction::IntegerMultiplyImmediate {
            kind: IntegerKind::I64,
            destination,
            source,
            immediate,
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(source).non_negative && immediate.as_int() >= 0;
            write(destination, with_origin(fact, origin));
        }
        Instruction::Step {
            kind: Some(IntegerKind::I64),
            destination,
            source,
            immediate,
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(source).non_negative && immediate.value() >= 0;
            write(destination, with_origin(fact, origin));
        }
        Instruction::IntegerModuloImmediate {
            kind: IntegerKind::I64,
            destination,
            source,
            ..
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(source).non_negative;
            write(destination, with_origin(fact, origin));
        }
        Instruction::Subtract {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::SubtractImmediate {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::BitwiseAnd {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::BitwiseOr {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::BitwiseXor {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::BitwiseNot {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::ShiftLeft {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::ShiftRight {
            kind: Some(IntegerKind::I64),
            destination,
            ..
        }
        | Instruction::Compare { destination, .. } => {
            write(destination, Fact::with_origin(INT, origin))
        }
        Instruction::Modulo {
            kind: None,
            destination,
            left,
            ..
        }
        | Instruction::BitwiseAnd {
            kind: None,
            destination,
            left,
            ..
        }
        | Instruction::BitwiseOr {
            kind: None,
            destination,
            left,
            ..
        }
        | Instruction::BitwiseXor {
            kind: None,
            destination,
            left,
            ..
        }
        | Instruction::ShiftLeft {
            kind: None,
            destination,
            left,
            ..
        }
        | Instruction::ShiftRight {
            kind: None,
            destination,
            left,
            ..
        } => {
            write(
                destination,
                Fact::with_origin(read(left).mask & (INT | UINT), origin),
            );
        }
        Instruction::BitwiseNot {
            kind: None,
            destination,
            source,
        } => {
            write(
                destination,
                Fact::with_origin(read(source).mask & (INT | UINT), origin),
            );
        }
        Instruction::IntegerAddAssign {
            kind: IntegerKind::I64,
            target,
            ..
        } => write(target, Fact::with_origin(INT, origin)),
        Instruction::Length { destination, .. } | Instruction::StringLength { destination, .. } => {
            let mut fact = Fact::known(UINT);
            fact.non_negative = true;
            write(destination, with_origin(fact, origin));
        }
        Instruction::Negate {
            destination,
            source,
        }
        | Instruction::UnaryPlus {
            destination,
            source,
        }
        | Instruction::SubtractImmediate {
            kind: None,
            destination,
            source,
            ..
        } => write(
            destination,
            with_origin(unary_numeric_result(read(source)), origin),
        ),
        Instruction::AddImmediate {
            kind: None,
            destination,
            source,
            immediate,
        } => {
            let source = read(source);
            let mut fact = unary_numeric_result(source);
            fact.non_negative = source.non_negative && immediate.as_int() >= 0;
            write(destination, with_origin(fact, origin));
        }
        Instruction::Step {
            kind: None,
            destination,
            source,
            immediate,
        } => {
            let source = read(source);
            let mut fact = unary_numeric_result(source);
            fact.non_negative = source.non_negative && immediate.value() >= 0;
            write(destination, with_origin(fact, origin));
        }
        Instruction::Concatenate { destination, .. }
        | Instruction::ConcatenateRightConstant { destination, .. }
        | Instruction::ConcatenateLeftConstant { destination, .. } => {
            write(destination, Fact::with_origin(STRING, origin))
        }
        Instruction::NewArray {
            kind,
            count,
            destination,
            first_element,
        } => {
            let first = usize::from(first_element.index());
            let count = usize::from(count.value());
            let (mask, width) = match kind {
                ArrayKind::Vec => (VECTOR, 1),
                ArrayKind::Dict => (DICTIONARY, 2),
                ArrayKind::Tuple => (TUPLE, 1),
            };
            let observable_release = (0..count).any(|index| {
                state[first + index * width + width - 1]
                    .get()
                    .observable_release
            });
            write(destination, Fact::array(mask, origin, observable_release));
        }
        Instruction::NewFilledVec {
            destination, value, ..
        } => write(
            destination,
            Fact::array(VECTOR, origin, read(value).observable_release),
        ),
        Instruction::Rest {
            destination,
            subject,
            ..
        } => write(
            destination,
            Fact::array(VECTOR, origin, read(subject).observable_release),
        ),
        Instruction::NewStatic { destination, .. }
        | Instruction::NewDynamic { destination, .. }
        | Instruction::NewTyped { destination, .. }
        | Instruction::CloneObject { destination, .. } => {
            write(destination, Fact::with_origin(OBJECT, origin))
        }
        Instruction::InitializeProperties {
            object, descriptor, ..
        } => {
            if chunk
                .property_initialization_descriptor(descriptor)
                .allocates
            {
                write(object, Fact::with_origin(OBJECT, origin));
            }
        }
        Instruction::MakeClosure { destination, .. }
        | Instruction::MakeBound { destination, .. } => {
            write(destination, Fact::with_origin(CALLABLE, origin))
        }
        Instruction::AsCheck {
            destination,
            descriptor,
            ..
        } => write(
            destination,
            Fact::with_origin(
                descriptor_mask(&chunk.type_descriptors[usize::from(descriptor.index())])
                    .unwrap_or(ALL),
                origin,
            ),
        ),
        Instruction::AsOrNull {
            destination,
            descriptor,
            ..
        } => write(
            destination,
            Fact::known(
                descriptor_mask(&chunk.type_descriptors[usize::from(descriptor.index())])
                    .unwrap_or(ALL)
                    | NULL,
            ),
        ),
        Instruction::Coalesce {
            destination,
            source,
            ..
        } => write(destination, read(source)),
        Instruction::IndexGetOrNull { destination, .. }
        | Instruction::VecIndexGetOrNull { destination, .. }
        | Instruction::DictIndexGetIntegerKeyOrNull { destination, .. }
        | Instruction::DictIndexGetStringKeyOrNull { destination, .. }
        | Instruction::StringIndexGetOrNull { destination, .. }
        | Instruction::PropertyGetOrNull { destination, .. }
        | Instruction::PropertyGetOrNullUnchecked { destination, .. }
        | Instruction::StaticPropertyGetOrNull { destination, .. }
        | Instruction::IndexCoalesce { destination, .. }
        | Instruction::VecIndexCoalesce { destination, .. }
        | Instruction::DictIndexCoalesceIntKey { destination, .. }
        | Instruction::DictIndexCoalesceUintKey { destination, .. }
        | Instruction::DictIndexCoalesceStringKey { destination, .. }
        | Instruction::StringIndexCoalesce { destination, .. }
        | Instruction::PropertyCoalesce { destination, .. }
        | Instruction::PropertyCoalesceUnchecked { destination, .. }
        | Instruction::StaticPropertyCoalesce { destination, .. } => {
            write(destination, Fact::UNKNOWN)
        }
        Instruction::IndexGet {
            destination,
            container,
            ..
        } => {
            let container_fact = read(container);
            if container_fact.mask != 0 && container_fact.mask & !STRING == 0 {
                write(destination, Fact::known(STRING));
            } else {
                let array = container_fact.array;
                let mask = array_elements
                    .and_then(|elements| elements.get(array as usize))
                    .copied()
                    .filter(|_| array != NO_ORIGIN)
                    .unwrap_or(ALL);
                write(destination, Fact::with_origin(mask, origin));
            }
        }
        Instruction::StringIndexGet { destination, .. } => write(destination, Fact::known(STRING)),
        Instruction::VecIndexGet {
            destination,
            container,
            ..
        }
        | Instruction::DictIndexGetIntKey {
            destination,
            container,
            ..
        }
        | Instruction::DictIndexGetUintKey {
            destination,
            container,
            ..
        }
        | Instruction::DictIndexGetStringKey {
            destination,
            container,
            ..
        } => {
            let array = read(container).array;
            let mask = array_elements
                .and_then(|elements| elements.get(array as usize))
                .copied()
                .filter(|_| array != NO_ORIGIN)
                .unwrap_or(ALL);
            write(destination, Fact::with_origin(mask, origin));
        }
        Instruction::ElementGet { destination, .. }
        | Instruction::PropertyGet { destination, .. }
        | Instruction::PropertyGetUnchecked { destination, .. }
        | Instruction::CallMethodDirect { destination, .. }
        | Instruction::CallNamedDirect { destination, .. } => {
            write(destination, Fact::with_origin(ALL, origin))
        }
        Instruction::Remove {
            destination,
            container,
            ..
        }
        | Instruction::SwapRemove {
            destination,
            container,
            ..
        }
        | Instruction::RemoveFirst {
            destination,
            container,
        }
        | Instruction::RemoveLast {
            destination,
            container,
        } => {
            let current = read(container);
            write(container, current.without_origin());
            write(destination, Fact::UNKNOWN);
        }
        Instruction::PropertyRemove { destination, .. }
        | Instruction::PropertyRemoveUnchecked { destination, .. } => {
            write(destination, Fact::UNKNOWN);
        }
        instructions!(CallMethod | CallMethodDiscarded | CallMethodUnchecked; {
            argument_count,
            destination,
            first_argument,
            ..
        }) => {
            clear_window(first_argument, usize::from(argument_count.value()));
            write(destination, Fact::with_origin(ALL, origin));
        }
        Instruction::StaticPropertyGet { destination, .. }
        | Instruction::ConstantGet { destination, .. }
        | Instruction::ClassConstantGet { destination, .. }
        | Instruction::CallNamedConstantUnchecked { destination, .. }
        | Instruction::Require { destination, .. } => write(destination, Fact::UNKNOWN),
        instructions!(
            CallValue | CallValueUnchecked | CallValueDiscarded | CallNamed | CallNamedDiscarded
                | CallNamedUnchecked | CallStatic | CallStaticDiscarded;
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
        } => {
            clear_window(first_argument, usize::from(argument_count.value()));
            write(destination, Fact::UNKNOWN);
        }
        Instruction::CallWithNames {
            destination,
            callee,
            descriptor,
        }
        | Instruction::CallWithNamesDiscarded {
            destination,
            callee,
            descriptor,
        } => {
            let descriptor = &chunk.call_descriptors[usize::from(descriptor.index())];
            let count = usize::from(descriptor.positional) + descriptor.named.len();
            clear_window(Register::new(callee.index() + 1), count);
            write(destination, Fact::UNKNOWN);
        }
        Instruction::IndexSet {
            container, value, ..
        }
        | Instruction::VecIndexSet {
            container, value, ..
        }
        | Instruction::DictIndexSetIntegerKey {
            container, value, ..
        }
        | Instruction::DictIndexSetStringKey {
            container, value, ..
        }
        | Instruction::DictIndexSet {
            container, value, ..
        }
        | Instruction::Append { container, value }
        | Instruction::VecAppend { container, value }
        | Instruction::Spread { container, value } => {
            let mut current = read(container);
            current.observable_release |= read(value).observable_release;
            write(container, current.without_origin());
        }
        Instruction::IndexAddAssign { container, .. } => {
            let current = read(container);
            write(container, current.without_origin());
        }
        Instruction::ForeachInit {
            iterator, subject, ..
        } => {
            let subject = read(subject);
            write(
                iterator,
                Fact {
                    mask: ALL,
                    origin: subject.origin,
                    array: subject.array,
                    observable_release: subject.observable_release,
                    non_negative: false,
                    positive: false,
                },
            );
        }
        Instruction::ForeachNext {
            iterator,
            key_destination,
            value_destination,
            ..
        } => {
            let array = read(iterator).array;
            if key_destination != Register::NONE {
                let mask = array_keys
                    .and_then(|keys| keys.get(array as usize))
                    .copied()
                    .filter(|_| array != NO_ORIGIN)
                    .unwrap_or(ALL);
                write(key_destination, Fact::with_origin(mask, origin));
            }

            let mask = array_elements
                .and_then(|elements| elements.get(array as usize))
                .copied()
                .filter(|_| array != NO_ORIGIN)
                .unwrap_or(ALL);
            write(value_destination, Fact::with_origin(mask, origin));
        }
        Instruction::VecForeachNext {
            iterator,
            key_destination,
            value_destination,
            value_mode,
        } => {
            if key_destination != Register::NONE {
                write(key_destination, Fact::with_origin(UINT, origin));
            }

            let array = read(iterator).array;
            let mask = match value_mode {
                ArrayValueMode::Int => INT,
                ArrayValueMode::Uint => UINT,
                ArrayValueMode::Float => FLOAT,
                ArrayValueMode::Generic => array_elements
                    .and_then(|elements| elements.get(array as usize))
                    .copied()
                    .filter(|_| array != NO_ORIGIN)
                    .unwrap_or(ALL),
            };

            write(value_destination, Fact::with_origin(mask, origin));
        }
        Instruction::DictForeachNext {
            iterator,
            key_destination,
            value_destination,
            value_mode,
        } => {
            let array = read(iterator).array;
            if key_destination != Register::NONE {
                let mask = array_keys
                    .and_then(|keys| keys.get(array as usize))
                    .copied()
                    .filter(|_| array != NO_ORIGIN)
                    .unwrap_or(INT | UINT | BOOL | STRING);
                write(key_destination, Fact::with_origin(mask, origin));
            }
            let mask = match value_mode {
                ArrayValueMode::Int => INT,
                ArrayValueMode::Uint => UINT,
                ArrayValueMode::Float => FLOAT,
                ArrayValueMode::Generic => array_elements
                    .and_then(|elements| elements.get(array as usize))
                    .copied()
                    .filter(|_| array != NO_ORIGIN)
                    .unwrap_or(ALL),
            };
            write(value_destination, Fact::with_origin(mask, origin));
        }
        Instruction::IncrementJump {
            target, immediate, ..
        } => {
            let current = read(target);
            let mut next = unary_numeric_result(current);
            next.non_negative = current.non_negative && immediate.value() >= 0;
            write(target, next);
        }
        Instruction::CounterLoop { counter, .. } => {
            let current = read(counter);
            let mut next = unary_numeric_result(current);
            next.non_negative = current.non_negative;
            write(counter, next);
        }
        Instruction::IntCounterLoop { counter, .. } => {
            let current = read(counter);
            let mut next = Fact::known(INT);
            next.non_negative = current.non_negative;
            write(counter, next);
        }
        Instruction::IntStepLoop { descriptor, .. } => {
            let descriptor = chunk.int_step_loop_descriptor(descriptor);
            write(descriptor.counter, Fact::known(INT));
        }
        Instruction::Squares {
            first_destination,
            first_source,
            second_source,
        } => {
            let second_destination = Register::new(first_destination.index() + 1);
            write(first_destination, unary_numeric_result(read(first_source)));
            write(
                second_destination,
                unary_numeric_result(read(second_source)),
            );
        }
        Instruction::FloatSquares {
            first_destination, ..
        } => {
            write(first_destination, Fact::known(FLOAT));
            write(
                Register::new(first_destination.index() + 1),
                Fact::known(FLOAT),
            );
        }
        Instruction::FloatSquaresSum {
            first_destination, ..
        } => {
            for offset in 0..3 {
                write(
                    Register::new(first_destination.index() + offset),
                    Fact::known(FLOAT),
                );
            }
        }
        Instruction::FloatSquaresSumBranch { descriptor, .. } => {
            let descriptor = chunk.float_squares_sum_branch_descriptor(descriptor);

            write(descriptor.sum_destination, Fact::known(FLOAT));
            write(descriptor.first_square_destination, Fact::known(FLOAT));
            write(descriptor.second_square_destination, Fact::known(FLOAT));
        }
        Instruction::FloatPairUpdate { descriptor } => {
            let descriptor = chunk.float_pair_update_descriptor(descriptor);

            write(descriptor.first_destination, Fact::known(FLOAT));
            write(descriptor.second_destination, Fact::known(FLOAT));
        }
        Instruction::NumericLoop { .. }
        | Instruction::IntNumericLoop { .. }
        | Instruction::PreparedIntNumericLoop { .. }
        | Instruction::Jump { .. }
        | Instruction::NumericRegionJump { .. }
        | Instruction::JumpIfFalse { .. }
        | Instruction::JumpIfTrue { .. }
        | Instruction::JumpIfNull { .. }
        | Instruction::JumpIfNotNull { .. }
        | Instruction::SwitchInt { .. }
        | Instruction::SwitchString { .. }
        | Instruction::SwitchBool { .. }
        | Instruction::SwitchFloat { .. }
        | Instruction::SwitchPattern { .. }
        | Instruction::SwitchTuplePattern { .. }
        | Instruction::IntegerRangeJumpIf { .. }
        | Instruction::UintJumpUnless { .. }
        | Instruction::UintJumpUnlessImmediate { .. }
        | Instruction::ReturnIntegerUnchecked { .. }
        | Instruction::IntegerRangeJumpUnless { .. }
        | Instruction::BoolPatternBranch { .. }
        | Instruction::CheckDefined { .. }
        | Instruction::CheckDestructure { .. }
        | Instruction::PropertySet { .. }
        | Instruction::PropertySetUnchecked { .. }
        | Instruction::PropertyInitRaw { .. }
        | Instruction::StaticPropertySet { .. }
        | Instruction::Return { .. }
        | Instruction::ReturnNull
        | Instruction::ReturnUnchecked { .. }
        | Instruction::ReturnReferenceUnchecked { .. }
        | Instruction::ReturnPairUnchecked { .. }
        | Instruction::ReturnScalarUnchecked { .. }
        | Instruction::ReturnNullUnchecked
        | Instruction::Throw { .. }
        | Instruction::Rethrow
        | Instruction::ThrowUnhandledMatch { .. }
        | Instruction::Write { .. }
        | Instruction::Debug { .. }
        | Instruction::Assert { .. }
        | Instruction::Exit { .. }
        | Instruction::Panic { .. }
        | Instruction::FillDefault { .. }
        | Instruction::JumpUnless {
            comparison: Comparison::Equal | Comparison::NotEqual,
            ..
        }
        | Instruction::IntJumpUnless { .. }
        | Instruction::StringJumpUnless { .. }
        | Instruction::StringByteJumpUnlessEqual { .. }
        | Instruction::StringByteJumpUnlessNotEqual { .. }
        | Instruction::IntJumpUnlessImmediate { .. }
        | Instruction::JumpUnlessConstant {
            comparison: Comparison::Equal | Comparison::NotEqual,
            ..
        }
        | Instruction::PropertyIndexUpdate { .. }
        | Instruction::PropertyIndexUpdateUnchecked { .. }
        | Instruction::PropertyIndexSet { .. }
        | Instruction::PropertyIndexSetUnchecked { .. }
        | Instruction::PropertyFillIntRange { .. }
        | Instruction::PropertyStep { .. }
        | Instruction::PropertyStepUnchecked { .. }
        | Instruction::PropertyAdd { .. }
        | Instruction::PropertyAddUnchecked { .. }
        | Instruction::ReserveArray { .. }
        | Instruction::CheckSoleReference { .. }
        | Instruction::CheckDiscardedResult { .. }
        | Instruction::CheckWhereConstraints
        | Instruction::DrainFinalizers => {}
        Instruction::Clear { target } => {
            write(target, Fact::UNKNOWN.release_is_unobservable());
        }
        Instruction::JumpUnless { left, right, .. } => {
            write(left, read(left).release_is_unobservable());
            write(right, read(right).release_is_unobservable());
        }
        Instruction::JumpUnlessConstant { source, .. } => {
            write(source, read(source).release_is_unobservable());
        }
    }
}

pub(crate) fn numeric_result(left: Fact, right: Fact) -> Fact {
    if left.mask & !INT == 0 && right.mask & !INT == 0 {
        Fact::known(INT)
    } else if left.mask & !UINT == 0 && right.mask & !UINT == 0 {
        Fact::known(UINT)
    } else if left.mask & !FLOAT == 0 || right.mask & !FLOAT == 0 {
        Fact::known(FLOAT)
    } else {
        Fact::known(NUMERIC)
    }
}

pub(crate) fn literal_mask(literal: &Literal) -> u16 {
    match literal {
        Literal::Null => NULL,
        Literal::Bool(_) => BOOL,
        Literal::Int(_) => INT,
        Literal::Uint(_) => UINT,
        Literal::Float(_) => FLOAT,
        Literal::String(_) => STRING,
    }
}
