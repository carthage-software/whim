//! Constant tracking: which instruction results are known values.

use hashbrown::HashSet;
use whim_base::limits::MAX_TYPE_DEPTH;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayKind;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::Register;
use whim_value::atom::Atom;
use whim_value::heap::Heap;
use whim_value::ops::compare_int_float;
use whim_value::ops::compare_uint_float;

use crate::type_flow::BytecodeComparison;
use crate::type_flow::ConstantValue;
use crate::type_flow::Fact;
use crate::type_flow::NO_ORIGIN;
use crate::type_flow::Ordering;
use crate::type_flow::STRING;
use crate::type_flow::TypeFlow;
use crate::type_flow::append_constant_text;
use crate::type_flow::instruction_index;

fn integer_constant(immediate: ImmediateInteger, kind: IntegerKind) -> ConstantValue {
    match kind {
        IntegerKind::I64 => ConstantValue::Int(i64::from(immediate.as_int())),
        IntegerKind::U64 => ConstantValue::Uint(u64::from(immediate.as_uint())),
    }
}

#[derive(PartialEq, Eq, Hash)]
pub(super) enum ConstantDictionaryKey {
    Bool(bool),
    Int(i64),
    Uint(u64),
    String(Atom),
}

impl ConstantDictionaryKey {
    pub(super) fn from_value(value: ConstantValue) -> Option<Self> {
        match value {
            ConstantValue::Bool(value) => Some(Self::Bool(value)),
            ConstantValue::Int(value) => Some(Self::Int(value)),
            ConstantValue::Uint(value) => Some(Self::Uint(value)),
            ConstantValue::String(value) => Some(Self::String(value)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum ConstantOrdering {
    Ordered(Ordering),
    Unordered,
}

impl ConstantOrdering {
    const fn from_partial(ordering: Option<Ordering>) -> Self {
        match ordering {
            Some(ordering) => Self::Ordered(ordering),
            None => Self::Unordered,
        }
    }

    const fn ordered(self) -> Option<Ordering> {
        match self {
            Self::Ordered(ordering) => Some(ordering),
            Self::Unordered => None,
        }
    }
}

impl TypeFlow<'_> {
    pub(crate) fn constant_result(&self, index: usize) -> Option<(Register, ConstantValue)> {
        self.constant_result_at(index, 0)
    }

    pub(crate) fn constant_value(&self, index: usize, register: Register) -> Option<ConstantValue> {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return None;
        }
        self.constant_value_fact(self.fact(index, register), 0)
    }

    pub(crate) fn constant_result_at(
        &self,
        index: usize,
        depth: usize,
    ) -> Option<(Register, ConstantValue)> {
        if depth > MAX_TYPE_DEPTH || index >= self.chunk.code.len() || !self.reachable[index] {
            return None;
        }

        // The fixpoint asks for constants while its own facts are still
        // moving, so nothing is remembered until they stop.
        if !self.settled.get() || self.constants.borrow().is_empty() {
            return self.constant_result_uncached(index, depth);
        }

        if let Some(remembered) = self.constants.borrow()[index].as_ref() {
            return remembered.clone();
        }

        let result = self.constant_result_uncached(index, depth);
        self.constants.borrow_mut()[index] = Some(result.clone());
        result
    }

    fn constant_result_uncached(
        &self,
        index: usize,
        depth: usize,
    ) -> Option<(Register, ConstantValue)> {
        let value =
            |register: Register| self.constant_value_fact(self.fact(index, register), depth + 1);
        match self.chunk.code[index] {
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } => Some((destination, value(source)?)),
            Instruction::LoadConstant {
                destination,
                constant,
            } => Some((
                destination,
                constant_from_literal(&self.chunk.constants[usize::from(constant.index())]),
            )),
            Instruction::LoadNull { destination } => Some((destination, ConstantValue::Null)),
            Instruction::LoadTrue { destination } => Some((destination, ConstantValue::Bool(true))),
            Instruction::LoadFalse { destination } => {
                Some((destination, ConstantValue::Bool(false)))
            }
            Instruction::LoadInteger {
                kind,
                destination,
                immediate,
            } => Some((destination, integer_constant(immediate, kind))),
            Instruction::Add {
                destination,
                left,
                right,
                ..
            }
            | Instruction::FloatAdd {
                destination,
                left,
                right,
            } => Some((destination, constant_add(value(left)?, value(right)?)?)),
            Instruction::Subtract {
                destination,
                left,
                right,
                ..
            }
            | Instruction::FloatSubtract {
                destination,
                left,
                right,
            } => Some((destination, constant_subtract(value(left)?, value(right)?)?)),
            Instruction::Multiply {
                destination,
                left,
                right,
                ..
            }
            | Instruction::FloatMultiply {
                destination,
                left,
                right,
            } => Some((destination, constant_multiply(value(left)?, value(right)?)?)),
            Instruction::FloatMultiplyConstant {
                destination,
                source,
                constant,
            } => Some((
                destination,
                constant_multiply(
                    value(source)?,
                    constant_from_literal(&self.chunk.constants[usize::from(constant.index())]),
                )?,
            )),
            Instruction::Divide {
                destination,
                left,
                right,
            } => Some((destination, constant_divide(value(left)?, value(right)?)?)),
            Instruction::Modulo {
                destination,
                left,
                right,
                ..
            } => Some((destination, constant_modulo(value(left)?, value(right)?)?)),
            Instruction::Power {
                destination,
                left,
                right,
            } => Some((destination, constant_power(value(left)?, value(right)?)?)),
            Instruction::Negate {
                destination,
                source,
            } => Some((destination, constant_negate(value(source)?)?)),
            Instruction::UnaryPlus {
                destination,
                source,
            } => Some((destination, constant_unary_plus(value(source)?)?)),
            Instruction::Step {
                destination,
                source,
                immediate,
                ..
            } => {
                let step = i64::from(immediate.value());
                let source = value(source)?;
                let result = match source {
                    ConstantValue::Uint(value) => {
                        ConstantValue::Uint(value.checked_add_signed(step)?)
                    }
                    source => constant_add(source, ConstantValue::Int(step))?,
                };
                Some((destination, result))
            }
            Instruction::AddImmediate {
                kind,
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_add(
                    value(source)?,
                    integer_constant(immediate, kind.unwrap_or(IntegerKind::I64)),
                )?,
            )),
            Instruction::SubtractImmediate {
                kind,
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_subtract(
                    value(source)?,
                    integer_constant(immediate, kind.unwrap_or(IntegerKind::I64)),
                )?,
            )),
            Instruction::IntegerMultiplyImmediate {
                kind,
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_multiply(value(source)?, integer_constant(immediate, kind))?,
            )),
            Instruction::IntegerModuloImmediate {
                kind,
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_modulo(value(source)?, integer_constant(immediate, kind))?,
            )),
            Instruction::Concatenate {
                destination,
                left,
                right,
            } => Some((
                destination,
                constant_concatenate(value(left)?, value(right)?, self.allocator)?,
            )),
            Instruction::ConcatenateRightConstant {
                destination,
                source,
                constant,
            } => {
                let Literal::String(right) = &self.chunk.constants[usize::from(constant.index())]
                else {
                    return None;
                };
                Some((
                    destination,
                    constant_concatenate(
                        value(source)?,
                        ConstantValue::String(right.clone()),
                        self.allocator,
                    )?,
                ))
            }
            Instruction::ConcatenateLeftConstant {
                destination,
                source,
                constant,
            } => {
                let Literal::String(left) = &self.chunk.constants[usize::from(constant.index())]
                else {
                    return None;
                };
                Some((
                    destination,
                    constant_concatenate(
                        ConstantValue::String(left.clone()),
                        value(source)?,
                        self.allocator,
                    )?,
                ))
            }
            Instruction::BitwiseAnd {
                destination,
                left,
                right,
                ..
            } => Some((
                destination,
                constant_int_binary(value(left)?, value(right)?, |left, right| left & right)?,
            )),
            Instruction::BitwiseOr {
                destination,
                left,
                right,
                ..
            } => Some((
                destination,
                constant_int_binary(value(left)?, value(right)?, |left, right| left | right)?,
            )),
            Instruction::BitwiseXor {
                destination,
                left,
                right,
                ..
            } => Some((
                destination,
                constant_int_binary(value(left)?, value(right)?, |left, right| left ^ right)?,
            )),
            Instruction::BitwiseNot {
                destination,
                source,
                ..
            } => {
                let result = match value(source)? {
                    ConstantValue::Int(value) => ConstantValue::Int(!value),
                    ConstantValue::Uint(value) => ConstantValue::Uint(!value),
                    _ => return None,
                };
                Some((destination, result))
            }
            Instruction::ShiftLeft {
                destination,
                left,
                right,
                ..
            } => Some((
                destination,
                constant_shift(value(left)?, value(right)?, true)?,
            )),
            Instruction::ShiftRight {
                destination,
                left,
                right,
                ..
            } => Some((
                destination,
                constant_shift(value(left)?, value(right)?, false)?,
            )),
            Instruction::Equal {
                destination,
                left,
                right,
            } => Some((
                destination,
                ConstantValue::Bool(constant_equals(&value(left)?, &value(right)?)),
            )),
            Instruction::NotEqual {
                destination,
                left,
                right,
            } => Some((
                destination,
                ConstantValue::Bool(!constant_equals(&value(left)?, &value(right)?)),
            )),
            Instruction::LessThan {
                destination,
                left,
                right,
            } => Some((
                destination,
                ConstantValue::Bool(matches!(
                    constant_compare(&value(left)?, &value(right)?)?,
                    ConstantOrdering::Ordered(Ordering::Less)
                )),
            )),
            Instruction::LessThanOrEqual {
                destination,
                left,
                right,
            } => Some((
                destination,
                ConstantValue::Bool(matches!(
                    constant_compare(&value(left)?, &value(right)?)?,
                    ConstantOrdering::Ordered(Ordering::Less | Ordering::Equal)
                )),
            )),
            Instruction::GreaterThan {
                destination,
                left,
                right,
            } => Some((
                destination,
                ConstantValue::Bool(matches!(
                    constant_compare(&value(left)?, &value(right)?)?,
                    ConstantOrdering::Ordered(Ordering::Greater)
                )),
            )),
            Instruction::GreaterThanOrEqual {
                destination,
                left,
                right,
            } => Some((
                destination,
                ConstantValue::Bool(matches!(
                    constant_compare(&value(left)?, &value(right)?)?,
                    ConstantOrdering::Ordered(Ordering::Greater | Ordering::Equal)
                )),
            )),
            Instruction::Compare {
                destination,
                left,
                right,
            } => Some((
                destination,
                ConstantValue::Int(
                    match constant_compare(&value(left)?, &value(right)?)?.ordered()? {
                        Ordering::Less => -1,
                        Ordering::Equal => 0,
                        Ordering::Greater => 1,
                    },
                ),
            )),
            Instruction::Not {
                destination,
                source,
            } => {
                let ConstantValue::Bool(value) = value(source)? else {
                    return None;
                };
                Some((destination, ConstantValue::Bool(!value)))
            }
            Instruction::Length {
                destination,
                source,
            }
            | Instruction::StringLength {
                destination,
                source,
            } => Some((
                destination,
                ConstantValue::Uint(
                    self.constant_length_fact(self.fact(index, source), depth + 1)?,
                ),
            )),
            Instruction::IndexGet {
                destination,
                container,
                index: key,
            }
            | Instruction::VecIndexGet {
                destination,
                container,
                index: key,
                ..
            }
            | Instruction::StringIndexGet {
                destination,
                container,
                index: key,
            } => Some((
                destination,
                self.constant_index(self.fact(index, container), value(key)?, depth + 1)?,
            )),
            Instruction::ElementGet {
                destination,
                subject,
                index: element,
            } => Some((
                destination,
                self.constant_index(
                    self.fact(index, subject),
                    ConstantValue::Int(i64::from(element.value())),
                    depth + 1,
                )?,
            )),
            _ => None,
        }
    }

    pub(crate) fn pure_constant_destination(&self, index: usize) -> Option<Register> {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return None;
        }
        if let Some((destination, _)) = self.constant_result(index) {
            return Some(destination);
        }
        match self.chunk.code[index] {
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } if self.fact_is_constant(self.fact(index, source), 0) => Some(destination),
            Instruction::NewArray {
                kind: ArrayKind::Vec | ArrayKind::Tuple,
                count: element_count,
                destination,
                first_element,
            } if (0..usize::from(element_count.value())).all(|offset| {
                self.fact_is_constant(
                    self.fact(index, Register::new(first_element.index() + offset as u16)),
                    0,
                )
            }) =>
            {
                Some(destination)
            }
            Instruction::NewArray {
                kind: ArrayKind::Dict,
                count: pair_count,
                destination,
                first_element: first_pair,
            } if (0..usize::from(pair_count.value())).all(|pair| {
                let key = first_pair.index() + (pair * 2) as u16;
                matches!(
                    self.constant_value_fact(self.fact(index, Register::new(key)), 0),
                    Some(
                        ConstantValue::Int(_)
                            | ConstantValue::Uint(_)
                            | ConstantValue::Bool(_)
                            | ConstantValue::String(_)
                    )
                ) && self.fact_is_constant(self.fact(index, Register::new(key + 1)), 0)
            }) =>
            {
                Some(destination)
            }
            _ => None,
        }
    }

    pub(in crate::type_flow) fn constant_length_fact(
        &self,
        fact: Fact,
        depth: usize,
    ) -> Option<u64> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        if fact.mask == STRING {
            match self.origin_type(fact.origin, depth + 1) {
                Some(TypeDescriptor::StringLiteral(value)) => {
                    return u64::try_from(value.as_bytes().len()).ok();
                }
                Some(TypeDescriptor::StringLength {
                    min,
                    max: Some(max),
                }) if min == max => {
                    return u64::try_from(min).ok();
                }
                _ => {}
            }
        }
        let index = instruction_index(fact.origin)?;
        match self.chunk.code[index] {
            Instruction::LoadConstant { constant, .. } => {
                let Literal::String(value) = &self.chunk.constants[usize::from(constant.index())]
                else {
                    return None;
                };
                u64::try_from(value.as_bytes().len()).ok()
            }
            Instruction::NewArray {
                kind: ArrayKind::Vec | ArrayKind::Tuple,
                count: element_count,
                ..
            } => Some(u64::from(element_count.value())),
            Instruction::NewArray {
                kind: ArrayKind::Dict,
                count: pair_count,
                first_element: first_pair,
                ..
            } => {
                if pair_count.value() <= 1 {
                    return Some(u64::from(pair_count.value()));
                }

                let mut keys = HashSet::with_capacity(usize::from(pair_count.value()));
                for pair in 0..u16::from(pair_count.value()) {
                    let register = Register::new(first_pair.index() + pair * 2);
                    let key = ConstantDictionaryKey::from_value(
                        self.constant_value_fact(self.fact(index, register), depth + 1)?,
                    )?;

                    keys.insert(key);
                }

                u64::try_from(keys.len()).ok()
            }
            Instruction::Concatenate { left, right, .. } => {
                let left = self.constant_length_fact(self.fact(index, left), depth + 1)?;
                let right = self.constant_length_fact(self.fact(index, right), depth + 1)?;
                left.checked_add(right)
            }
            Instruction::ConcatenateRightConstant {
                source, constant, ..
            } => {
                let left = self.constant_length_fact(self.fact(index, source), depth + 1)?;
                let Literal::String(right) = &self.chunk.constants[usize::from(constant.index())]
                else {
                    return None;
                };
                left.checked_add(u64::try_from(right.as_bytes().len()).ok()?)
            }
            Instruction::ConcatenateLeftConstant {
                source, constant, ..
            } => {
                let Literal::String(left) = &self.chunk.constants[usize::from(constant.index())]
                else {
                    return None;
                };
                let right = self.constant_length_fact(self.fact(index, source), depth + 1)?;
                u64::try_from(left.as_bytes().len())
                    .ok()?
                    .checked_add(right)
            }
            _ => None,
        }
    }

    pub(in crate::type_flow) fn constant_value_fact(
        &self,
        fact: Fact,
        depth: usize,
    ) -> Option<ConstantValue> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }

        if let Some(descriptor) = self.origin_descriptor(fact.origin, depth + 1) {
            return match descriptor {
                TypeDescriptor::TrueLiteral => Some(ConstantValue::Bool(true)),
                TypeDescriptor::FalseLiteral => Some(ConstantValue::Bool(false)),
                TypeDescriptor::IntLiteral(value) => Some(ConstantValue::Int(*value)),
                TypeDescriptor::UintLiteral(value) => Some(ConstantValue::Uint(*value)),
                TypeDescriptor::FloatLiteral(value) => Some(ConstantValue::Float(*value)),
                TypeDescriptor::StringLiteral(value) => Some(ConstantValue::String(value.clone())),
                _ => None,
            };
        }

        let index = instruction_index(fact.origin)?;
        self.constant_result_at(index, depth + 1)
            .map(|(_, value)| value)
    }

    pub(in crate::type_flow) fn fact_is_constant(&self, fact: Fact, depth: usize) -> bool {
        if depth > MAX_TYPE_DEPTH || fact.origin == NO_ORIGIN {
            return false;
        }
        if self.constant_value_fact(fact, depth + 1).is_some() {
            return true;
        }
        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        match self.chunk.code[index] {
            Instruction::NewArray {
                kind: ArrayKind::Vec | ArrayKind::Tuple,
                count: element_count,
                first_element,
                ..
            } => (0..usize::from(element_count.value())).all(|offset| {
                self.fact_is_constant(
                    self.fact(index, Register::new(first_element.index() + offset as u16)),
                    depth + 1,
                )
            }),
            Instruction::NewArray {
                kind: ArrayKind::Dict,
                count: pair_count,
                first_element: first_pair,
                ..
            } => (0..usize::from(pair_count.value())).all(|pair| {
                let key = first_pair.index() + (pair * 2) as u16;
                matches!(
                    self.constant_value_fact(self.fact(index, Register::new(key)), depth + 1,),
                    Some(
                        ConstantValue::Int(_)
                            | ConstantValue::Uint(_)
                            | ConstantValue::Bool(_)
                            | ConstantValue::String(_)
                    )
                ) && self.fact_is_constant(self.fact(index, Register::new(key + 1)), depth + 1)
            }),
            _ => false,
        }
    }

    pub(in crate::type_flow) fn constant_index(
        &self,
        container: Fact,
        key: ConstantValue,
        depth: usize,
    ) -> Option<ConstantValue> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        if container.mask == STRING {
            let ConstantValue::String(value) = self.constant_value_fact(container, depth + 1)?
            else {
                return None;
            };
            let byte = *value.as_bytes().get(key.position()?)?;
            return Some(ConstantValue::String(self.allocator.intern(&[byte])));
        }
        let index = instruction_index(container.origin)?;
        match self.chunk.code[index] {
            Instruction::NewArray {
                kind: ArrayKind::Vec | ArrayKind::Tuple,
                count: element_count,
                first_element,
                ..
            } => {
                let key = key.position()?;
                if key >= usize::from(element_count.value()) {
                    return None;
                }
                self.constant_value_fact(
                    self.fact(index, Register::new(first_element.index() + key as u16)),
                    depth + 1,
                )
            }
            Instruction::NewArray {
                kind: ArrayKind::Dict,
                count: pair_count,
                first_element: first_pair,
                ..
            } => {
                for pair in (0..usize::from(pair_count.value())).rev() {
                    let key_register = first_pair.index() + (pair * 2) as u16;
                    let candidate = self.constant_value_fact(
                        self.fact(index, Register::new(key_register)),
                        depth + 1,
                    )?;
                    if constant_equals(&candidate, &key) {
                        return self.constant_value_fact(
                            self.fact(index, Register::new(key_register + 1)),
                            depth + 1,
                        );
                    }
                }
                None
            }
            _ => None,
        }
    }
}

pub(crate) fn constant_power(left: ConstantValue, right: ConstantValue) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Uint(base), ConstantValue::Uint(exponent)) => {
            let value = match base {
                0 => u64::from(exponent == 0),
                1 => 1,
                _ => base.checked_pow(u32::try_from(exponent).ok()?)?,
            };
            Some(ConstantValue::Uint(value))
        }
        (ConstantValue::Int(base), ConstantValue::Int(exponent)) if exponent >= 0 => {
            let exponent = u64::try_from(exponent).ok()?;
            let value = match base {
                0 => i64::from(exponent == 0),
                1 => 1,
                -1 => {
                    if exponent.is_multiple_of(2) {
                        1
                    } else {
                        -1
                    }
                }
                _ => base.checked_pow(u32::try_from(exponent).ok()?)?,
            };
            Some(ConstantValue::Int(value))
        }
        (ConstantValue::Int(0), ConstantValue::Int(_)) => None,
        (ConstantValue::Int(base), ConstantValue::Int(exponent)) => {
            Some(ConstantValue::Float((base as f64).powf(exponent as f64)))
        }
        (ConstantValue::Float(base), ConstantValue::Float(exponent)) => {
            Some(ConstantValue::Float(base.powf(exponent)))
        }
        (base, ConstantValue::Float(exponent)) => {
            Some(ConstantValue::Float(constant_numeric(base)?.powf(exponent)))
        }
        (ConstantValue::Float(base), exponent) => {
            Some(ConstantValue::Float(base.powf(constant_numeric(exponent)?)))
        }
        _ => None,
    }
}

pub(crate) fn constant_negate(value: ConstantValue) -> Option<ConstantValue> {
    match value {
        ConstantValue::Int(value) => value.checked_neg().map(ConstantValue::Int),
        ConstantValue::Uint(0) => Some(ConstantValue::Uint(0)),
        ConstantValue::Float(value) => Some(ConstantValue::Float(-value)),
        _ => None,
    }
}

pub(crate) fn constant_unary_plus(value: ConstantValue) -> Option<ConstantValue> {
    match value {
        ConstantValue::Int(_) | ConstantValue::Uint(_) | ConstantValue::Float(_) => Some(value),
        _ => None,
    }
}

pub(crate) fn constant_int_binary(
    left: ConstantValue,
    right: ConstantValue,
    operation: impl FnOnce(i64, i64) -> i64,
) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            Some(ConstantValue::Int(operation(left, right)))
        }
        (ConstantValue::Uint(left), ConstantValue::Uint(right)) => Some(ConstantValue::Uint(
            operation(left as i64, right as i64) as u64,
        )),
        _ => None,
    }
}

pub(crate) fn constant_shift(
    left: ConstantValue,
    right: ConstantValue,
    shift_left: bool,
) -> Option<ConstantValue> {
    let right = match right {
        ConstantValue::Int(value) => u32::try_from(value).ok()?,
        ConstantValue::Uint(value) => u32::try_from(value).ok()?,
        _ => return None,
    };
    if right > 63 {
        return None;
    }
    match left {
        ConstantValue::Int(left) => Some(ConstantValue::Int(if shift_left {
            left << right
        } else {
            left >> right
        })),
        ConstantValue::Uint(left) => Some(ConstantValue::Uint(if shift_left {
            left << right
        } else {
            left >> right
        })),
        _ => None,
    }
}

pub(crate) fn constant_numeric(value: ConstantValue) -> Option<f64> {
    match value {
        ConstantValue::Int(value) => Some(value as f64),
        ConstantValue::Uint(value) => Some(value as f64),
        ConstantValue::Float(value) => Some(value),
        _ => None,
    }
}

pub(crate) fn constant_equals(left: &ConstantValue, right: &ConstantValue) -> bool {
    match (left, right) {
        (ConstantValue::Null, ConstantValue::Null) => true,
        (ConstantValue::Bool(left), ConstantValue::Bool(right)) => left == right,
        (ConstantValue::Int(left), ConstantValue::Int(right)) => left == right,
        (ConstantValue::Uint(left), ConstantValue::Uint(right)) => left == right,
        (ConstantValue::Float(left), ConstantValue::Float(right)) => left == right,
        (ConstantValue::String(left), ConstantValue::String(right)) => {
            left.as_bytes() == right.as_bytes()
        }
        _ => false,
    }
}

pub(super) fn constant_comparison(
    comparison: BytecodeComparison,
    left: &ConstantValue,
    right: &ConstantValue,
) -> Option<bool> {
    Some(match comparison {
        BytecodeComparison::Equal => constant_equals(left, right),
        BytecodeComparison::NotEqual => !constant_equals(left, right),
        ordered => matches!(
            (ordered, constant_compare(left, right)?),
            (
                BytecodeComparison::LessThan,
                ConstantOrdering::Ordered(Ordering::Less)
            ) | (
                BytecodeComparison::LessThanOrEqual,
                ConstantOrdering::Ordered(Ordering::Less | Ordering::Equal),
            ) | (
                BytecodeComparison::GreaterThan,
                ConstantOrdering::Ordered(Ordering::Greater)
            ) | (
                BytecodeComparison::GreaterThanOrEqual,
                ConstantOrdering::Ordered(Ordering::Greater | Ordering::Equal),
            )
        ),
    })
}

fn constant_compare(left: &ConstantValue, right: &ConstantValue) -> Option<ConstantOrdering> {
    match (left, right) {
        (ConstantValue::Uint(left), ConstantValue::Uint(right)) => {
            Some(ConstantOrdering::Ordered(left.cmp(right)))
        }
        (ConstantValue::Int(left), ConstantValue::Uint(right)) => Some(ConstantOrdering::Ordered(
            i128::from(*left).cmp(&i128::from(*right)),
        )),
        (ConstantValue::Uint(left), ConstantValue::Int(right)) => Some(ConstantOrdering::Ordered(
            i128::from(*left).cmp(&i128::from(*right)),
        )),
        (ConstantValue::Uint(left), ConstantValue::Float(right)) => Some(
            ConstantOrdering::from_partial(compare_uint_float(*left, *right)),
        ),
        (ConstantValue::Float(left), ConstantValue::Uint(right)) => {
            Some(ConstantOrdering::from_partial(
                compare_uint_float(*right, *left).map(Ordering::reverse),
            ))
        }
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            Some(ConstantOrdering::Ordered(left.cmp(right)))
        }
        (ConstantValue::Float(left), ConstantValue::Float(right)) => {
            Some(ConstantOrdering::from_partial(left.partial_cmp(right)))
        }
        (ConstantValue::Int(left), ConstantValue::Float(right)) => Some(
            ConstantOrdering::from_partial(compare_int_float(*left, *right)),
        ),
        (ConstantValue::Float(left), ConstantValue::Int(right)) => Some(
            ConstantOrdering::from_partial(compare_int_float(*right, *left).map(Ordering::reverse)),
        ),
        (ConstantValue::String(left), ConstantValue::String(right)) => Some(
            ConstantOrdering::Ordered(left.as_bytes().cmp(right.as_bytes())),
        ),
        _ => None,
    }
}

pub(crate) fn constant_from_literal(literal: &Literal) -> ConstantValue {
    match literal {
        Literal::Null => ConstantValue::Null,
        Literal::Bool(value) => ConstantValue::Bool(*value),
        Literal::Int(value) => ConstantValue::Int(*value),
        Literal::Uint(value) => ConstantValue::Uint(*value),
        Literal::Float(value) => ConstantValue::Float(*value),
        Literal::String(value) => ConstantValue::String(value.clone()),
    }
}

pub(crate) fn constant_add(left: ConstantValue, right: ConstantValue) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Uint(left), ConstantValue::Uint(right)) => {
            left.checked_add(right).map(ConstantValue::Uint)
        }
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            left.checked_add(right).map(ConstantValue::Int)
        }
        (ConstantValue::Float(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left + right))
        }
        (left, ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(constant_numeric(left)? + right))
        }
        (ConstantValue::Float(left), right) => {
            Some(ConstantValue::Float(left + constant_numeric(right)?))
        }
        _ => None,
    }
}

pub(crate) fn constant_subtract(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Uint(left), ConstantValue::Uint(right)) => {
            left.checked_sub(right).map(ConstantValue::Uint)
        }
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            left.checked_sub(right).map(ConstantValue::Int)
        }
        (ConstantValue::Float(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left - right))
        }
        (left, ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(constant_numeric(left)? - right))
        }
        (ConstantValue::Float(left), right) => {
            Some(ConstantValue::Float(left - constant_numeric(right)?))
        }
        _ => None,
    }
}

pub(crate) fn constant_multiply(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Uint(left), ConstantValue::Uint(right)) => {
            left.checked_mul(right).map(ConstantValue::Uint)
        }
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            left.checked_mul(right).map(ConstantValue::Int)
        }
        (ConstantValue::Float(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left * right))
        }
        (left, ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(constant_numeric(left)? * right))
        }
        (ConstantValue::Float(left), right) => {
            Some(ConstantValue::Float(left * constant_numeric(right)?))
        }
        _ => None,
    }
}

pub(crate) fn constant_concatenate(
    left: ConstantValue,
    right: ConstantValue,
    allocator: &Heap,
) -> Option<ConstantValue> {
    let mut bytes = Vec::new();
    append_constant_text(&mut bytes, left)?;
    append_constant_text(&mut bytes, right)?;
    Some(ConstantValue::String(allocator.intern(&bytes)))
}

pub(crate) fn constant_divide(left: ConstantValue, right: ConstantValue) -> Option<ConstantValue> {
    let left = constant_numeric(left)?;
    let right = constant_numeric(right)?;
    (right != 0.0).then_some(ConstantValue::Float(left / right))
}

pub(crate) fn constant_modulo(left: ConstantValue, right: ConstantValue) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Int(left), ConstantValue::Int(right)) if right != 0 => {
            Some(ConstantValue::Int(left.checked_rem(right).unwrap_or(0)))
        }
        (ConstantValue::Uint(left), ConstantValue::Uint(right)) => {
            left.checked_rem(right).map(ConstantValue::Uint)
        }
        _ => None,
    }
}
