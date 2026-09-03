//! The per-instruction fact transfer function.

use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::Comparison;
use crate::optimizer::type_flow::ALL;
use crate::optimizer::type_flow::BOOL;
use crate::optimizer::type_flow::CALLABLE;
use crate::optimizer::type_flow::Cell;
use crate::optimizer::type_flow::Chunk;
use crate::optimizer::type_flow::DICTIONARY;
use crate::optimizer::type_flow::FLOAT;
use crate::optimizer::type_flow::Fact;
use crate::optimizer::type_flow::INT;
use crate::optimizer::type_flow::Instruction;
use crate::optimizer::type_flow::Literal;
use crate::optimizer::type_flow::NO_ORIGIN;
use crate::optimizer::type_flow::NULL;
use crate::optimizer::type_flow::NUMERIC;
use crate::optimizer::type_flow::OBJECT;
use crate::optimizer::type_flow::Register;
use crate::optimizer::type_flow::STRING;
use crate::optimizer::type_flow::THIS_ORIGIN;
use crate::optimizer::type_flow::TUPLE;
use crate::optimizer::type_flow::VECTOR;
use crate::optimizer::type_flow::descriptor_mask;
use crate::optimizer::type_flow::unary_numeric_result;
use crate::optimizer::type_flow::with_origin;

macro_rules! instructions {
    ($($name:ident)|+ ; $fields:tt) => {
        $(Instruction::$name $fields)|+
    };
}

pub(in crate::optimizer) fn transfer(
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
        Instruction::LoadInt {
            destination,
            immediate,
        } => write(
            destination,
            Fact::integer(i64::from(immediate.value()), origin),
        ),
        Instruction::Add {
            destination,
            left,
            right,
        }
        | Instruction::Multiply {
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
            destination,
            left,
            right,
        }
        | Instruction::Power {
            destination,
            left,
            right,
        } => write(
            destination,
            with_origin(numeric_result(read(left), read(right)), origin),
        ),
        Instruction::FloatAdd { destination, .. }
        | Instruction::FloatSubtract { destination, .. }
        | Instruction::FloatMultiply { destination, .. }
        | Instruction::FloatMultiplyConstant { destination, .. }
        | Instruction::FloatDifferenceAdd { destination, .. }
        | Instruction::FloatScaleProductAdd { destination, .. }
        | Instruction::Divide { destination, .. } => {
            write(destination, Fact::with_origin(FLOAT, origin))
        }
        Instruction::IntAdd {
            destination,
            left,
            right,
        }
        | Instruction::IntMultiply {
            destination,
            left,
            right,
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(left).non_negative && read(right).non_negative;
            write(destination, with_origin(fact, origin));
        }
        Instruction::IntModulo {
            destination, left, ..
        }
        | Instruction::Modulo {
            destination, left, ..
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(left).non_negative;
            write(destination, with_origin(fact, origin));
        }
        Instruction::IntMultiplyImmediate {
            destination,
            source,
            immediate,
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(source).non_negative && immediate.value() >= 0;
            write(destination, with_origin(fact, origin));
        }
        Instruction::IntModuloImmediate {
            destination,
            source,
            ..
        } => {
            let mut fact = Fact::known(INT);
            fact.non_negative = read(source).non_negative;
            write(destination, with_origin(fact, origin));
        }
        Instruction::IntSubtract { destination, .. }
        | Instruction::IntBitwiseAnd { destination, .. }
        | Instruction::IntBitwiseOr { destination, .. }
        | Instruction::IntBitwiseXor { destination, .. }
        | Instruction::IntBitwiseNot { destination, .. }
        | Instruction::IntShiftLeft { destination, .. }
        | Instruction::IntShiftRight { destination, .. }
        | Instruction::BitwiseAnd { destination, .. }
        | Instruction::BitwiseOr { destination, .. }
        | Instruction::BitwiseXor { destination, .. }
        | Instruction::BitwiseNot { destination, .. }
        | Instruction::ShiftLeft { destination, .. }
        | Instruction::ShiftRight { destination, .. }
        | Instruction::Compare { destination, .. } => {
            write(destination, Fact::with_origin(INT, origin))
        }
        Instruction::IntAddAssign { target, .. } => write(target, Fact::with_origin(INT, origin)),
        Instruction::Length { destination, .. } | Instruction::StringLength { destination, .. } => {
            let mut fact = Fact::known(INT);
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
            destination,
            source,
            ..
        } => write(
            destination,
            with_origin(unary_numeric_result(read(source)), origin),
        ),
        Instruction::AddImmediate {
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
        | Instruction::ConcatenateConstant { destination, .. } => {
            write(destination, Fact::with_origin(STRING, origin))
        }
        instruction @ (Instruction::NewVec {
            element_count,
            destination,
            first_element,
        }
        | Instruction::NewTuple {
            element_count,
            destination,
            first_element,
        }) => {
            let first = usize::from(first_element.index());
            let count = usize::from(element_count.value());
            let observable_release =
                (first..first + count).any(|register| state[register].get().observable_release);
            let mask = if matches!(instruction, Instruction::NewVec { .. }) {
                VECTOR
            } else {
                TUPLE
            };
            write(destination, Fact::array(mask, origin, observable_release));
        }
        Instruction::NewFilledVec {
            destination, value, ..
        } => write(
            destination,
            Fact::array(VECTOR, origin, read(value).observable_release),
        ),
        Instruction::NewDict {
            pair_count,
            destination,
            first_pair,
        } => {
            let first = usize::from(first_pair.index());
            let count = usize::from(pair_count.value());
            let observable_release =
                (0..count).any(|pair| state[first + pair * 2 + 1].get().observable_release);
            write(
                destination,
                Fact::array(DICTIONARY, origin, observable_release),
            );
        }
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
        | Instruction::CallMethodDirect { destination, .. } => {
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
        | Instruction::DictIndexSetIntKey {
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
                write(key_destination, Fact::with_origin(INT, origin));
            }

            let array = read(iterator).array;
            let mask = match value_mode {
                ArrayValueMode::Int => INT,
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
                    .unwrap_or(INT | STRING);
                write(key_destination, Fact::with_origin(mask, origin));
            }
            let mask = match value_mode {
                ArrayValueMode::Int => INT,
                ArrayValueMode::Float => FLOAT,
                ArrayValueMode::Generic => array_elements
                    .and_then(|elements| elements.get(array as usize))
                    .copied()
                    .filter(|_| array != NO_ORIGIN)
                    .unwrap_or(ALL),
            };
            write(value_destination, Fact::with_origin(mask, origin));
        }
        Instruction::IncrementJump { target, .. } => {
            write(target, unary_numeric_result(read(target)))
        }
        Instruction::CounterLoop { counter, .. } => {
            write(counter, unary_numeric_result(read(counter)))
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
        | Instruction::IntRangeJumpIf { .. }
        | Instruction::IntRangeJumpUnless { .. }
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
        | Instruction::ReturnIntUnchecked { .. }
        | Instruction::Throw { .. }
        | Instruction::Rethrow
        | Instruction::ThrowUnhandledMatch { .. }
        | Instruction::Write { .. }
        | Instruction::WriteLine { .. }
        | Instruction::WriteError { .. }
        | Instruction::WriteErrorLine { .. }
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

pub(in crate::optimizer) fn numeric_result(left: Fact, right: Fact) -> Fact {
    if left.mask & !INT == 0 && right.mask & !INT == 0 {
        Fact::known(INT)
    } else if left.mask & !FLOAT == 0 || right.mask & !FLOAT == 0 {
        Fact::known(FLOAT)
    } else {
        Fact::known(NUMERIC)
    }
}

pub(in crate::optimizer) fn literal_mask(literal: &Literal) -> u16 {
    match literal {
        Literal::Null => NULL,
        Literal::Bool(_) => BOOL,
        Literal::Int(_) => INT,
        Literal::Float(_) => FLOAT,
        Literal::String(_) => STRING,
    }
}
