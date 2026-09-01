//! The arithmetic, bitwise, and comparison operations the loop performs.

use std::cmp::Ordering;

use crate::value::ValueView;
use crate::vm::ByteStringObject;
use crate::vm::Fault;
use crate::vm::Heap;
use crate::vm::Value;
use crate::vm::ops;

fn narrow(value: i128) -> Result<Value, Fault> {
    if value > i128::from(i64::MAX) {
        Err(Fault::Overflow)
    } else if value < i128::from(i64::MIN) {
        Err(Fault::Underflow)
    } else {
        Ok(Value::int(value as i64))
    }
}

/// `+` per the arithmetic table: int stays int with overflow checks, a
/// float operand promotes.
#[inline(always)]
pub(in crate::vm) fn arithmetic_add(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => integer_add(*a, *b).map(Value::int),
        (ValueView::Float(a), ValueView::Float(b)) => Ok(Value::float(a + b)),
        (ValueView::Int(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 + b)),
        (ValueView::Float(a), ValueView::Int(b)) => Ok(Value::float(a + *b as f64)),
        _ => Err(Fault::Incompatible),
    }
}

/// `-` per the arithmetic table.
#[inline(always)]
pub(in crate::vm) fn arithmetic_subtract(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => integer_subtract(*a, *b).map(Value::int),
        (ValueView::Float(a), ValueView::Float(b)) => Ok(Value::float(a - b)),
        (ValueView::Int(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 - b)),
        (ValueView::Float(a), ValueView::Int(b)) => Ok(Value::float(a - *b as f64)),
        _ => Err(Fault::Incompatible),
    }
}

/// `*` per the arithmetic table.
#[inline(always)]
pub(in crate::vm) fn arithmetic_multiply(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => integer_multiply(*a, *b).map(Value::int),
        (ValueView::Float(a), ValueView::Float(b)) => Ok(Value::float(a * b)),
        (ValueView::Int(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 * b)),
        (ValueView::Float(a), ValueView::Int(b)) => Ok(Value::float(a * *b as f64)),
        _ => Err(Fault::Incompatible),
    }
}

/// `/`: always a float division, and a zero right operand throws, including
/// `1.0 / 0.0`.
#[inline(always)]
pub(in crate::vm) fn arithmetic_divide(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    let dividend = numeric_operand(left)?;
    let divisor = numeric_operand(right)?;
    if divisor == 0.0 {
        return Err(Fault::DivisionByZero);
    }

    Ok(Value::float(dividend / divisor))
}

fn numeric_operand(value: &Value) -> Result<f64, Fault> {
    match value.transparent() {
        ValueView::Int(value) => Ok(*value as f64),
        ValueView::Float(value) => Ok(*value),
        _ => Err(Fault::Incompatible),
    }
}

/// `%`: int-only remainder whose sign follows the left operand.
#[inline(always)]
pub(in crate::vm) fn arithmetic_modulo(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => integer_modulo(*a, *b).map(Value::int),
        _ => Err(Fault::Incompatible),
    }
}

#[inline(always)]
pub(in crate::vm) fn integer_add(left: i64, right: i64) -> Result<i64, Fault> {
    left.checked_add(right).ok_or(if left >= 0 {
        Fault::Overflow
    } else {
        Fault::Underflow
    })
}

#[inline(always)]
pub(in crate::vm) fn integer_subtract(left: i64, right: i64) -> Result<i64, Fault> {
    left.checked_sub(right).ok_or(if left >= 0 {
        Fault::Overflow
    } else {
        Fault::Underflow
    })
}

#[inline(always)]
pub(in crate::vm) fn integer_multiply(left: i64, right: i64) -> Result<i64, Fault> {
    left.checked_mul(right).ok_or(if (left > 0) == (right > 0) {
        Fault::Overflow
    } else {
        Fault::Underflow
    })
}

#[inline(always)]
pub(in crate::vm) fn integer_modulo(left: i64, right: i64) -> Result<i64, Fault> {
    if right == 0 {
        return Err(Fault::DivisionByZero);
    }

    Ok(left.checked_rem(right).unwrap_or(0))
}

/// `**` per the arithmetic table: an int base with a non-negative int
/// exponent stays int with overflow checks. A negative int exponent produces
/// a float unless the base is zero, which is division by zero. Every other
/// numeric combination is a float power.
#[inline(always)]
pub(in crate::vm) fn arithmetic_power(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(base), ValueView::Int(exponent)) => {
            if *exponent >= 0 {
                integer_power(*base, *exponent as u64)
            } else if *base == 0 {
                Err(Fault::DivisionByZero)
            } else {
                Ok(Value::float((*base as f64).powf(*exponent as f64)))
            }
        }
        (ValueView::Float(base), ValueView::Float(exponent)) => {
            Ok(Value::float(base.powf(*exponent)))
        }
        (ValueView::Int(base), ValueView::Float(exponent)) => {
            Ok(Value::float((*base as f64).powf(*exponent)))
        }
        (ValueView::Float(base), ValueView::Int(exponent)) => {
            Ok(Value::float(base.powf(*exponent as f64)))
        }
        _ => Err(Fault::Incompatible),
    }
}

fn integer_power(base: i64, exponent: u64) -> Result<Value, Fault> {
    match base {
        0 => Ok(Value::int(i64::from(exponent == 0))),
        1 => Ok(Value::int(1)),
        -1 => Ok(Value::int(if exponent.is_multiple_of(2) { 1 } else { -1 })),
        _ => {
            if exponent > 63 {
                return Err(power_direction(base, exponent));
            }
            let mut result: i128 = 1;
            for _ in 0..exponent {
                result = match result.checked_mul(i128::from(base)) {
                    Some(next) => next,
                    None => return Err(power_direction(base, exponent)),
                };
            }
            narrow(result)
        }
    }
}

fn power_direction(base: i64, exponent: u64) -> Fault {
    if base < 0 && !exponent.is_multiple_of(2) {
        Fault::Underflow
    } else {
        Fault::Overflow
    }
}

/// `++`/`--` stepping: an int steps with overflow checks, a float steps by
/// the same amount.
#[inline(always)]
pub(in crate::vm) fn step_by(value: &Value, step: i64) -> Result<Value, Fault> {
    match value.transparent() {
        ValueView::Int(current) => match current.checked_add(step) {
            Some(stepped) => Ok(Value::int(stepped)),
            None => Err(if step >= 0 {
                Fault::Overflow
            } else {
                Fault::Underflow
            }),
        },
        ValueView::Float(current) => Ok(Value::float(current + step as f64)),
        _ => Err(Fault::Incompatible),
    }
}

/// Unary `-`: negates an int with an overflow check or a float.
#[inline(always)]
pub(in crate::vm) fn negate(value: &Value) -> Result<Value, Fault> {
    match value.transparent() {
        ValueView::Int(operand) => match operand.checked_neg() {
            Some(negated) => Ok(Value::int(negated)),
            None => Err(Fault::Overflow),
        },
        ValueView::Float(operand) => Ok(Value::float(-operand)),
        _ => Err(Fault::Incompatible),
    }
}

/// `&` over int operands.
#[inline(always)]
pub(in crate::vm) fn bitwise_and(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => Ok(Value::int(a & b)),
        _ => Err(Fault::Incompatible),
    }
}

/// `|` over int operands.
#[inline(always)]
pub(in crate::vm) fn bitwise_or(_heap: &Heap, left: &Value, right: &Value) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => Ok(Value::int(a | b)),
        _ => Err(Fault::Incompatible),
    }
}

/// `^` over int operands.
#[inline(always)]
pub(in crate::vm) fn bitwise_xor(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => Ok(Value::int(a ^ b)),
        _ => Err(Fault::Incompatible),
    }
}

/// `<<`: a logical shift modulo the 64-bit space, with the count gated to
/// `0..=63`.
#[inline(always)]
pub(in crate::vm) fn shift_left(_heap: &Heap, left: &Value, right: &Value) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => integer_shift_left(*a, *b).map(Value::int),
        _ => Err(Fault::Incompatible),
    }
}

/// `<<` over operands already proven to be integers.
pub(in crate::vm) fn integer_shift_left(left: i64, right: i64) -> Result<i64, Fault> {
    if !(0..=63).contains(&right) {
        return Err(Fault::ShiftRange);
    }

    Ok(((left as u64) << right as u32) as i64)
}

/// `>>`: an arithmetic shift preserving the sign, with the count gated to
/// `0..=63`.
#[inline(always)]
pub(in crate::vm) fn shift_right(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => integer_shift_right(*a, *b).map(Value::int),
        _ => Err(Fault::Incompatible),
    }
}

/// `>>` over operands already proven to be integers.
pub(in crate::vm) fn integer_shift_right(left: i64, right: i64) -> Result<i64, Fault> {
    if !(0..=63).contains(&right) {
        return Err(Fault::ShiftRange);
    }

    Ok(left >> right as u32)
}

/// `.`: both operands stringify by the concatenation rules.
pub(in crate::vm) fn concatenate(heap: &Heap, left: &Value, right: &Value) -> Result<Value, Fault> {
    let Some(left_text) = ops::stringify_for_concat(heap, left) else {
        return Err(Fault::Incompatible);
    };

    let Some(right_text) = ops::stringify_for_concat(heap, right) else {
        return Err(Fault::Incompatible);
    };

    if left_text.is_empty() {
        return Ok(Value::string(right_text));
    }
    if right_text.is_empty() {
        return Ok(Value::string(left_text));
    }

    Ok(Value::string(ByteStringObject::concat(
        heap,
        &left_text,
        &right_text,
    )))
}

/// `<`: ordered strictly below; an unordered comparison (NaN) is `false`.
pub(in crate::vm) fn compare_less(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    if let (ValueView::Float(left), ValueView::Float(right)) =
        (left.transparent(), right.transparent())
    {
        return Ok(Value::bool(left < right));
    }

    match ops::compare(left, right) {
        Ok(ordering) => Ok(Value::bool(matches!(ordering, Some(Ordering::Less)))),
        Err(_) => Err(Fault::Incompatible),
    }
}

/// `<=`, with NaN yielding `false`.
pub(in crate::vm) fn compare_less_or_equal(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    if let (ValueView::Float(left), ValueView::Float(right)) =
        (left.transparent(), right.transparent())
    {
        return Ok(Value::bool(left <= right));
    }

    match ops::compare(left, right) {
        Ok(ordering) => Ok(Value::bool(matches!(
            ordering,
            Some(Ordering::Less | Ordering::Equal)
        ))),
        Err(_) => Err(Fault::Incompatible),
    }
}

/// `>`, with NaN yielding `false`.
pub(in crate::vm) fn compare_greater(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    if let (ValueView::Float(left), ValueView::Float(right)) =
        (left.transparent(), right.transparent())
    {
        return Ok(Value::bool(left > right));
    }

    match ops::compare(left, right) {
        Ok(ordering) => Ok(Value::bool(matches!(ordering, Some(Ordering::Greater)))),
        Err(_) => Err(Fault::Incompatible),
    }
}

/// `>=`, with NaN yielding `false`.
pub(in crate::vm) fn compare_greater_or_equal(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    if let (ValueView::Float(left), ValueView::Float(right)) =
        (left.transparent(), right.transparent())
    {
        return Ok(Value::bool(left >= right));
    }

    match ops::compare(left, right) {
        Ok(ordering) => Ok(Value::bool(matches!(
            ordering,
            Some(Ordering::Greater | Ordering::Equal)
        ))),
        Err(_) => Err(Fault::Incompatible),
    }
}

/// `<=>`: `-1`, `0`, or `1`; an unordered comparison throws.
pub(in crate::vm) fn compare_spaceship(
    _heap: &Heap,
    left: &Value,
    right: &Value,
) -> Result<Value, Fault> {
    match ops::compare(left, right) {
        Ok(Some(ordering)) => Ok(Value::int(match ordering {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        })),
        Ok(None) => Err(Fault::Unordered),
        Err(_) => Err(Fault::Incompatible),
    }
}
