//! The arithmetic, bitwise, and comparison operations the loop performs.

use std::cmp::Ordering;

use whim_value::Value;
use whim_value::ValueView;
use whim_value::atom::Atom;
use whim_value::heap::Heap;
use whim_value::ops;
use whim_value::string::ByteStringObject;

use crate::vm::Fault;

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
        (ValueView::Uint(a), ValueView::Uint(b)) => {
            a.checked_add(*b).map(Value::uint).ok_or(Fault::Overflow)
        }
        (ValueView::Float(a), ValueView::Float(b)) => Ok(Value::float(a + b)),
        (ValueView::Int(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 + b)),
        (ValueView::Float(a), ValueView::Int(b)) => Ok(Value::float(a + *b as f64)),
        (ValueView::Uint(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 + b)),
        (ValueView::Float(a), ValueView::Uint(b)) => Ok(Value::float(a + *b as f64)),
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
        (ValueView::Uint(a), ValueView::Uint(b)) => {
            a.checked_sub(*b).map(Value::uint).ok_or(Fault::Underflow)
        }
        (ValueView::Float(a), ValueView::Float(b)) => Ok(Value::float(a - b)),
        (ValueView::Int(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 - b)),
        (ValueView::Float(a), ValueView::Int(b)) => Ok(Value::float(a - *b as f64)),
        (ValueView::Uint(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 - b)),
        (ValueView::Float(a), ValueView::Uint(b)) => Ok(Value::float(a - *b as f64)),
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
        (ValueView::Uint(a), ValueView::Uint(b)) => {
            a.checked_mul(*b).map(Value::uint).ok_or(Fault::Overflow)
        }
        (ValueView::Float(a), ValueView::Float(b)) => Ok(Value::float(a * b)),
        (ValueView::Int(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 * b)),
        (ValueView::Float(a), ValueView::Int(b)) => Ok(Value::float(a * *b as f64)),
        (ValueView::Uint(a), ValueView::Float(b)) => Ok(Value::float(*a as f64 * b)),
        (ValueView::Float(a), ValueView::Uint(b)) => Ok(Value::float(a * *b as f64)),
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
        ValueView::Uint(value) => Ok(*value as f64),
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
        (ValueView::Uint(a), ValueView::Uint(b)) => a
            .checked_rem(*b)
            .map(Value::uint)
            .ok_or(Fault::DivisionByZero),
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

#[inline(always)]
pub(in crate::vm) fn unsigned_add(left: u64, right: u64) -> Result<u64, Fault> {
    left.checked_add(right).ok_or(Fault::Overflow)
}

#[inline(always)]
pub(in crate::vm) fn unsigned_subtract(left: u64, right: u64) -> Result<u64, Fault> {
    left.checked_sub(right).ok_or(Fault::Underflow)
}

#[inline(always)]
pub(in crate::vm) fn unsigned_multiply(left: u64, right: u64) -> Result<u64, Fault> {
    left.checked_mul(right).ok_or(Fault::Overflow)
}

#[inline(always)]
pub(in crate::vm) fn unsigned_modulo(left: u64, right: u64) -> Result<u64, Fault> {
    left.checked_rem(right).ok_or(Fault::DivisionByZero)
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
        (ValueView::Uint(base), ValueView::Uint(exponent)) => {
            let result = match (*base, *exponent) {
                (_, 0) | (1, _) => 1,
                (0, _) => 0,
                (_, 64..) => return Err(Fault::Overflow),
                (base, exponent) => base.checked_pow(exponent as u32).ok_or(Fault::Overflow)?,
            };
            Ok(Value::uint(result))
        }
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
        (ValueView::Uint(base), ValueView::Float(exponent)) => {
            Ok(Value::float((*base as f64).powf(*exponent)))
        }
        (ValueView::Float(base), ValueView::Uint(exponent)) => {
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
        ValueView::Uint(current) => {
            current
                .checked_add_signed(step)
                .map(Value::uint)
                .ok_or(if step < 0 {
                    Fault::Underflow
                } else {
                    Fault::Overflow
                })
        }
        _ => Err(Fault::Incompatible),
    }
}

pub(in crate::vm) fn arithmetic_immediate(value: &Value, step: i64) -> Result<Value, Fault> {
    if value.is_uint() {
        Err(Fault::Incompatible)
    } else {
        step_by(value, step)
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
        ValueView::Uint(operand) => {
            if *operand == 0 {
                Ok(Value::uint(0))
            } else {
                Err(Fault::Underflow)
            }
        }
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
        (ValueView::Uint(a), ValueView::Uint(b)) => Ok(Value::uint(a & b)),
        _ => Err(Fault::Incompatible),
    }
}

/// `|` over int operands.
#[inline(always)]
pub(in crate::vm) fn bitwise_or(_heap: &Heap, left: &Value, right: &Value) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => Ok(Value::int(a | b)),
        (ValueView::Uint(a), ValueView::Uint(b)) => Ok(Value::uint(a | b)),
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
        (ValueView::Uint(a), ValueView::Uint(b)) => Ok(Value::uint(a ^ b)),
        _ => Err(Fault::Incompatible),
    }
}

/// `<<`: a logical shift modulo the 64-bit space, with the count gated to
/// `0..=63`.
#[inline(always)]
pub(in crate::vm) fn shift_left(_heap: &Heap, left: &Value, right: &Value) -> Result<Value, Fault> {
    match (left.transparent(), right.transparent()) {
        (ValueView::Int(a), ValueView::Int(b)) => integer_shift_left(*a, *b).map(Value::int),
        (ValueView::Int(a), ValueView::Uint(b)) => {
            if *b >= 64 {
                return Err(Fault::ShiftRange);
            }

            Ok(Value::int(*a << *b as u32))
        }
        (ValueView::Uint(a), ValueView::Int(b)) => {
            if !(0..=63).contains(b) {
                return Err(Fault::ShiftRange);
            }

            Ok(Value::uint(*a << *b as u32))
        }
        (ValueView::Uint(a), ValueView::Uint(b)) => {
            if *b >= 64 {
                return Err(Fault::ShiftRange);
            }

            Ok(Value::uint(*a << *b as u32))
        }
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
        (ValueView::Int(a), ValueView::Uint(b)) => {
            if *b >= 64 {
                return Err(Fault::ShiftRange);
            }

            Ok(Value::int(*a >> *b as u32))
        }
        (ValueView::Uint(a), ValueView::Int(b)) => {
            if !(0..=63).contains(b) {
                return Err(Fault::ShiftRange);
            }

            Ok(Value::uint(*a >> *b as u32))
        }
        (ValueView::Uint(a), ValueView::Uint(b)) => {
            if *b >= 64 {
                return Err(Fault::ShiftRange);
            }

            Ok(Value::uint(*a >> *b as u32))
        }
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
    if let (Some(left_length), Some(right_length)) = (left.as_string_len(), right.as_string_len())
        && let Some(length) = left_length.checked_add(right_length)
        && length <= 32
        && let (Some(left), Some(right)) = (left.as_string_bytes(), right.as_string_bytes())
    {
        let mut bytes = [0; 32];
        bytes[..left_length].copy_from_slice(left);
        bytes[left_length..length].copy_from_slice(right);
        return Ok(Value::from_string_bytes(heap, &bytes[..length]));
    }

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

pub(in crate::vm) fn prepare_string_append(value: &Value) {
    if value.newtype_id().is_none()
        && let ValueView::String(string) = value.transparent()
        && !string.is_flat()
        && string.is_unique()
    {
        string.flatten();
    }
}

#[inline(never)]
pub(in crate::vm) fn concatenate_right_constant(
    heap: &Heap,
    source: &Value,
    extra: &Atom,
    in_place: bool,
) -> Result<Option<Value>, Fault> {
    if in_place && let ValueView::String(target) = source.transparent() {
        prepare_string_append(source);
        // SAFETY: constant storage cannot overlap a uniquely owned target.
        if unsafe { ByteStringObject::append_unique(target, extra.as_bytes()) } {
            return Ok(None);
        }
    }

    let extra = Value::string(extra.to_handle());
    concatenate(heap, source, &extra).map(Some)
}

#[inline(never)]
pub(in crate::vm) fn concatenate_left_constant(
    heap: &Heap,
    extra: &Atom,
    source: &Value,
) -> Result<Value, Fault> {
    let Some(source) = ops::stringify_for_concat(heap, source) else {
        return Err(Fault::Incompatible);
    };

    if extra.as_bytes().is_empty() {
        return Ok(Value::string(source));
    }
    if source.is_empty() {
        return Ok(Value::string(extra.to_handle()));
    }

    Ok(Value::string(ByteStringObject::concat(
        heap,
        extra.as_handle(),
        &source,
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

#[cfg(test)]
mod tests {
    use whim_value::Value;
    use whim_value::ValueView;
    use whim_value::heap::Heap;
    use whim_value::newtype::NewtypeValueId;
    use whim_value::string::ByteStringObject;

    use super::concatenate;
    use super::prepare_string_append;

    #[test]
    fn short_concatenation_preserves_bytes_aliases_and_tags() {
        let heap = Heap::new();
        for left_length in [0, 1, 7, 8, 16, 31, 32, 33, 64] {
            for right_length in [0, 1, 7, 8, 16, 32] {
                let left_bytes = (0..left_length)
                    .map(|index| index as u8)
                    .collect::<Vec<_>>();
                let right_bytes = (0..right_length)
                    .map(|index| 255 - index as u8)
                    .collect::<Vec<_>>();
                let expected = [left_bytes.as_slice(), right_bytes.as_slice()].concat();
                let base = ByteStringObject::from_bytes(
                    &heap,
                    &[b"prefix", left_bytes.as_slice(), b"suffix"].concat(),
                );
                for left in [
                    Value::from_string_bytes(&heap, &left_bytes),
                    Value::string(ByteStringObject::from_bytes(&heap, &left_bytes)),
                    Value::string(ByteStringObject::slice(&heap, &base, 6, left_length)),
                ] {
                    let left = left.with_newtype(Some(NewtypeValueId(0)));
                    for right in [
                        Value::from_string_bytes(&heap, &right_bytes),
                        Value::string(ByteStringObject::from_bytes(&heap, &right_bytes)),
                    ] {
                        let right = right.with_newtype(Some(NewtypeValueId(1)));
                        let result = concatenate(&heap, &left, &right)
                            .unwrap_or_else(|_| panic!("string concatenation must succeed"));
                        assert_eq!(result.as_string_bytes().unwrap(), expected);
                        assert!(result.newtype_id().is_none());
                        assert_eq!(left.as_string_bytes().unwrap(), left_bytes);
                        assert_eq!(right.as_string_bytes().unwrap(), right_bytes);
                        assert_eq!(left.newtype_id(), Some(NewtypeValueId(0)));
                        assert_eq!(right.newtype_id(), Some(NewtypeValueId(1)));
                    }
                }
            }
        }
        assert_eq!(
            concatenate(
                &heap,
                &Value::from_string_bytes(&heap, b"x"),
                &Value::int(-42)
            )
            .unwrap_or_else(|_| panic!("integer concatenation must succeed"))
            .as_string_bytes()
            .unwrap(),
            b"x-42"
        );
        assert!(
            concatenate(
                &heap,
                &Value::from_string_bytes(&heap, b"x"),
                &Value::bool(true)
            )
            .is_err()
        );
    }

    #[test]
    fn rope_append_preparation_flattens_only_unique_ropes() {
        let heap = Heap::new();
        let left = ByteStringObject::from_bytes(&heap, b"abcdefghijklmnopqrstuvwxyz");
        let right = ByteStringObject::from_bytes(&heap, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        let value = Value::string(ByteStringObject::concat(&heap, &left, &right));
        let ValueView::String(string) = value.transparent() else {
            panic!("the concatenation has heap storage");
        };
        assert!(!string.is_flat());
        let alias = value.clone();
        prepare_string_append(&value);
        assert!(!string.is_flat());
        drop(alias);
        prepare_string_append(&value);
        assert!(string.is_flat());
        assert_eq!(
            value.as_string_bytes().unwrap(),
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        );
        assert_eq!(left.flatten(), b"abcdefghijklmnopqrstuvwxyz");
        assert_eq!(right.flatten(), b"ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        let slice = Value::string(ByteStringObject::slice(&heap, string, 3, 32));
        prepare_string_append(&slice);
        let ValueView::String(string) = slice.transparent() else {
            panic!("the slice has heap storage");
        };
        assert!(!string.is_flat());
        assert_eq!(
            slice.as_string_bytes().unwrap(),
            b"defghijklmnopqrstuvwxyzABCDEFGHI"
        );
    }
}
