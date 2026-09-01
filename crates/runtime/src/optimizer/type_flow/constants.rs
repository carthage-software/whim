//! Constant tracking: which instruction results are known values.

use crate::limits::MAX_TYPE_DEPTH;
use crate::optimizer::type_flow::ConstantValue;
use crate::optimizer::type_flow::Fact;
use crate::optimizer::type_flow::Heap;
use crate::optimizer::type_flow::Instruction;
use crate::optimizer::type_flow::Literal;
use crate::optimizer::type_flow::NO_ORIGIN;
use crate::optimizer::type_flow::Ordering;
use crate::optimizer::type_flow::Register;
use crate::optimizer::type_flow::TypeDescriptor;
use crate::optimizer::type_flow::TypeFlow;
use crate::optimizer::type_flow::append_constant_text;
use crate::optimizer::type_flow::instruction_index;
use crate::value::ops::compare_int_float;

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
    pub(in crate::optimizer) fn constant_result(
        &self,
        index: usize,
    ) -> Option<(Register, ConstantValue)> {
        self.constant_result_at(index, 0)
    }

    pub(in crate::optimizer) fn constant_value(
        &self,
        index: usize,
        register: Register,
    ) -> Option<ConstantValue> {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return None;
        }
        self.constant_value_fact(self.fact(index, register), 0)
    }

    pub(in crate::optimizer) fn constant_result_at(
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
            Instruction::LoadInt {
                destination,
                immediate,
            } => Some((
                destination,
                ConstantValue::Int(i64::from(immediate.value())),
            )),
            Instruction::Add {
                destination,
                left,
                right,
            }
            | Instruction::IntAdd {
                destination,
                left,
                right,
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
            }
            | Instruction::IntSubtract {
                destination,
                left,
                right,
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
            }
            | Instruction::IntMultiply {
                destination,
                left,
                right,
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
            }
            | Instruction::IntModulo {
                destination,
                left,
                right,
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
            Instruction::AddImmediate {
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_add(
                    value(source)?,
                    ConstantValue::Int(i64::from(immediate.value())),
                )?,
            )),
            Instruction::SubtractImmediate {
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_subtract(
                    value(source)?,
                    ConstantValue::Int(i64::from(immediate.value())),
                )?,
            )),
            Instruction::IntMultiplyImmediate {
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_multiply(
                    value(source)?,
                    ConstantValue::Int(i64::from(immediate.value())),
                )?,
            )),
            Instruction::IntModuloImmediate {
                destination,
                source,
                immediate,
            } => Some((
                destination,
                constant_modulo(
                    value(source)?,
                    ConstantValue::Int(i64::from(immediate.value())),
                )?,
            )),
            Instruction::Concatenate {
                destination,
                left,
                right,
            } => Some((
                destination,
                constant_concatenate(value(left)?, value(right)?, self.allocator)?,
            )),
            Instruction::BitwiseAnd {
                destination,
                left,
                right,
            }
            | Instruction::IntBitwiseAnd {
                destination,
                left,
                right,
            } => Some((
                destination,
                constant_int_binary(value(left)?, value(right)?, |left, right| left & right)?,
            )),
            Instruction::BitwiseOr {
                destination,
                left,
                right,
            }
            | Instruction::IntBitwiseOr {
                destination,
                left,
                right,
            } => Some((
                destination,
                constant_int_binary(value(left)?, value(right)?, |left, right| left | right)?,
            )),
            Instruction::BitwiseXor {
                destination,
                left,
                right,
            }
            | Instruction::IntBitwiseXor {
                destination,
                left,
                right,
            } => Some((
                destination,
                constant_int_binary(value(left)?, value(right)?, |left, right| left ^ right)?,
            )),
            Instruction::BitwiseNot {
                destination,
                source,
            }
            | Instruction::IntBitwiseNot {
                destination,
                source,
            } => {
                let ConstantValue::Int(value) = value(source)? else {
                    return None;
                };
                Some((destination, ConstantValue::Int(!value)))
            }
            Instruction::ShiftLeft {
                destination,
                left,
                right,
            }
            | Instruction::IntShiftLeft {
                destination,
                left,
                right,
            } => Some((
                destination,
                constant_shift(value(left)?, value(right)?, true)?,
            )),
            Instruction::ShiftRight {
                destination,
                left,
                right,
            }
            | Instruction::IntShiftRight {
                destination,
                left,
                right,
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
                ConstantValue::Int(self.constant_length_fact(self.fact(index, source), depth + 1)?),
            )),
            Instruction::IndexGet {
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

    pub(in crate::optimizer) fn pure_constant_destination(&self, index: usize) -> Option<Register> {
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
            Instruction::NewVec {
                element_count,
                destination,
                first_element,
            }
            | Instruction::NewTuple {
                element_count,
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
            Instruction::NewDict {
                pair_count,
                destination,
                first_pair,
            } if (0..usize::from(pair_count.value())).all(|pair| {
                let key = first_pair.index() + (pair * 2) as u16;
                matches!(
                    self.constant_value_fact(self.fact(index, Register::new(key)), 0),
                    Some(ConstantValue::Int(_) | ConstantValue::String(_))
                ) && self.fact_is_constant(self.fact(index, Register::new(key + 1)), 0)
            }) =>
            {
                Some(destination)
            }
            _ => None,
        }
    }

    pub(in crate::optimizer::type_flow) fn constant_length_fact(
        &self,
        fact: Fact,
        depth: usize,
    ) -> Option<i64> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        if let Some(TypeDescriptor::StringLiteral(value)) =
            self.origin_descriptor(fact.origin, depth + 1)
        {
            return i64::try_from(value.as_bytes().len()).ok();
        }
        let index = instruction_index(fact.origin)?;
        match self.chunk.code[index] {
            Instruction::LoadConstant { constant, .. } => {
                let Literal::String(value) = &self.chunk.constants[usize::from(constant.index())]
                else {
                    return None;
                };
                i64::try_from(value.as_bytes().len()).ok()
            }
            Instruction::NewVec { element_count, .. }
            | Instruction::NewTuple { element_count, .. } => Some(i64::from(element_count.value())),
            Instruction::NewDict { pair_count, .. } => Some(i64::from(pair_count.value())),
            Instruction::Concatenate { left, right, .. } => {
                let left = self.constant_length_fact(self.fact(index, left), depth + 1)?;
                let right = self.constant_length_fact(self.fact(index, right), depth + 1)?;
                left.checked_add(right)
            }
            _ => None,
        }
    }

    pub(in crate::optimizer::type_flow) fn constant_value_fact(
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
                TypeDescriptor::FloatLiteral(value) => Some(ConstantValue::Float(*value)),
                TypeDescriptor::StringLiteral(value) => Some(ConstantValue::String(value.clone())),
                _ => None,
            };
        }
        let index = instruction_index(fact.origin)?;
        self.constant_result_at(index, depth + 1)
            .map(|(_, value)| value)
    }

    pub(in crate::optimizer::type_flow) fn fact_is_constant(
        &self,
        fact: Fact,
        depth: usize,
    ) -> bool {
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
            Instruction::NewVec {
                element_count,
                first_element,
                ..
            }
            | Instruction::NewTuple {
                element_count,
                first_element,
                ..
            } => (0..usize::from(element_count.value())).all(|offset| {
                self.fact_is_constant(
                    self.fact(index, Register::new(first_element.index() + offset as u16)),
                    depth + 1,
                )
            }),
            Instruction::NewDict {
                pair_count,
                first_pair,
                ..
            } => (0..usize::from(pair_count.value())).all(|pair| {
                let key = first_pair.index() + (pair * 2) as u16;
                matches!(
                    self.constant_value_fact(self.fact(index, Register::new(key)), depth + 1,),
                    Some(ConstantValue::Int(_) | ConstantValue::String(_))
                ) && self.fact_is_constant(self.fact(index, Register::new(key + 1)), depth + 1)
            }),
            _ => false,
        }
    }

    pub(in crate::optimizer::type_flow) fn constant_index(
        &self,
        container: Fact,
        key: ConstantValue,
        depth: usize,
    ) -> Option<ConstantValue> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        let index = instruction_index(container.origin)?;
        match self.chunk.code[index] {
            Instruction::NewVec {
                element_count,
                first_element,
                ..
            }
            | Instruction::NewTuple {
                element_count,
                first_element,
                ..
            } => {
                let ConstantValue::Int(key) = key else {
                    return None;
                };
                let key = usize::try_from(key).ok()?;
                if key >= usize::from(element_count.value()) {
                    return None;
                }
                self.constant_value_fact(
                    self.fact(index, Register::new(first_element.index() + key as u16)),
                    depth + 1,
                )
            }
            Instruction::NewDict {
                pair_count,
                first_pair,
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

pub(in crate::optimizer) fn constant_power(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    match (left, right) {
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
        (ConstantValue::Int(base), ConstantValue::Float(exponent)) => {
            Some(ConstantValue::Float((base as f64).powf(exponent)))
        }
        (ConstantValue::Float(base), ConstantValue::Int(exponent)) => {
            Some(ConstantValue::Float(base.powf(exponent as f64)))
        }
        _ => None,
    }
}

pub(in crate::optimizer) fn constant_negate(value: ConstantValue) -> Option<ConstantValue> {
    match value {
        ConstantValue::Int(value) => value.checked_neg().map(ConstantValue::Int),
        ConstantValue::Float(value) => Some(ConstantValue::Float(-value)),
        _ => None,
    }
}

pub(in crate::optimizer) fn constant_unary_plus(value: ConstantValue) -> Option<ConstantValue> {
    match value {
        ConstantValue::Int(_) | ConstantValue::Float(_) => Some(value),
        _ => None,
    }
}

pub(in crate::optimizer) fn constant_int_binary(
    left: ConstantValue,
    right: ConstantValue,
    operation: impl FnOnce(i64, i64) -> i64,
) -> Option<ConstantValue> {
    let (ConstantValue::Int(left), ConstantValue::Int(right)) = (left, right) else {
        return None;
    };
    Some(ConstantValue::Int(operation(left, right)))
}

pub(in crate::optimizer) fn constant_shift(
    left: ConstantValue,
    right: ConstantValue,
    shift_left: bool,
) -> Option<ConstantValue> {
    let (ConstantValue::Int(left), ConstantValue::Int(right)) = (left, right) else {
        return None;
    };
    if !(0..=63).contains(&right) {
        return None;
    }
    Some(ConstantValue::Int(if shift_left {
        ((left as u64) << right as u32) as i64
    } else {
        left >> right as u32
    }))
}

pub(in crate::optimizer) fn constant_numeric(value: ConstantValue) -> Option<f64> {
    match value {
        ConstantValue::Int(value) => Some(value as f64),
        ConstantValue::Float(value) => Some(value),
        _ => None,
    }
}

pub(in crate::optimizer) fn constant_equals(left: &ConstantValue, right: &ConstantValue) -> bool {
    match (left, right) {
        (ConstantValue::Null, ConstantValue::Null) => true,
        (ConstantValue::Bool(left), ConstantValue::Bool(right)) => left == right,
        (ConstantValue::Int(left), ConstantValue::Int(right)) => left == right,
        (ConstantValue::Float(left), ConstantValue::Float(right)) => left == right,
        (ConstantValue::String(left), ConstantValue::String(right)) => {
            left.as_bytes() == right.as_bytes()
        }
        _ => false,
    }
}

fn constant_compare(left: &ConstantValue, right: &ConstantValue) -> Option<ConstantOrdering> {
    match (left, right) {
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

pub(in crate::optimizer) fn constant_from_literal(literal: &Literal) -> ConstantValue {
    match literal {
        Literal::Null => ConstantValue::Null,
        Literal::Bool(value) => ConstantValue::Bool(*value),
        Literal::Int(value) => ConstantValue::Int(*value),
        Literal::Float(value) => ConstantValue::Float(*value),
        Literal::String(value) => ConstantValue::String(value.clone()),
    }
}

pub(in crate::optimizer) fn constant_add(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            left.checked_add(right).map(ConstantValue::Int)
        }
        (ConstantValue::Float(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left + right))
        }
        (ConstantValue::Int(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left as f64 + right))
        }
        (ConstantValue::Float(left), ConstantValue::Int(right)) => {
            Some(ConstantValue::Float(left + right as f64))
        }
        _ => None,
    }
}

pub(in crate::optimizer) fn constant_subtract(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            left.checked_sub(right).map(ConstantValue::Int)
        }
        (ConstantValue::Float(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left - right))
        }
        (ConstantValue::Int(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left as f64 - right))
        }
        (ConstantValue::Float(left), ConstantValue::Int(right)) => {
            Some(ConstantValue::Float(left - right as f64))
        }
        _ => None,
    }
}

pub(in crate::optimizer) fn constant_multiply(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Int(left), ConstantValue::Int(right)) => {
            left.checked_mul(right).map(ConstantValue::Int)
        }
        (ConstantValue::Float(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left * right))
        }
        (ConstantValue::Int(left), ConstantValue::Float(right)) => {
            Some(ConstantValue::Float(left as f64 * right))
        }
        (ConstantValue::Float(left), ConstantValue::Int(right)) => {
            Some(ConstantValue::Float(left * right as f64))
        }
        _ => None,
    }
}

pub(in crate::optimizer) fn constant_concatenate(
    left: ConstantValue,
    right: ConstantValue,
    allocator: &Heap,
) -> Option<ConstantValue> {
    let mut bytes = Vec::new();
    append_constant_text(&mut bytes, left)?;
    append_constant_text(&mut bytes, right)?;
    Some(ConstantValue::String(allocator.intern(&bytes)))
}

pub(in crate::optimizer) fn constant_divide(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    let left = constant_numeric(left)?;
    let right = constant_numeric(right)?;
    (right != 0.0).then_some(ConstantValue::Float(left / right))
}

pub(in crate::optimizer) fn constant_modulo(
    left: ConstantValue,
    right: ConstantValue,
) -> Option<ConstantValue> {
    let (ConstantValue::Int(left), ConstantValue::Int(right)) = (left, right) else {
        return None;
    };
    if right == 0 {
        return None;
    }
    Some(ConstantValue::Int(left.checked_rem(right).unwrap_or(0)))
}
