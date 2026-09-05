//! Host numeric primitives used by the Whim-written `Whim\Math` namespace.

#![expect(
    clippy::inline_always,
    reason = "base conversion is a measured standard-library hot path"
)]

use std::cmp::Ordering;
use std::f64;

use num_bigint::BigUint;
use whim_macros::whim_constant;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::classes::names;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::ops::compare_int_float;
use crate::value::string::short::ShortString;

const BASE_DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseParseError {
    InvalidDigit,
    Overflow,
}

#[whim_function("Whim\\_Private\\math_sum_ints(vec<int> $numbers): int")]
fn sum_ints<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let numbers = arguments.vec(0);
    let mut sum = 0_i64;
    for number in numbers.iter() {
        // SAFETY: the built-in declaration requires every vec element to be an int.
        let number = unsafe { number.as_int_unchecked() };
        let Some(next) = sum.checked_add(number) else {
            let (class, message) = if sum >= 0 {
                (
                    names::OVERFLOW_ERROR,
                    "the integer result overflows the 64-bit range",
                )
            } else {
                (
                    names::UNDERFLOW_ERROR,
                    "the integer result underflows the 64-bit range",
                )
            };
            let class = context.vm.intern(class);
            return Err(context.vm.throw(class, message, 0));
        };
        sum = next;
    }

    Ok(Value::int(sum))
}

#[whim_function("Whim\\_Private\\math_sum_floats(vec<float> $numbers): float")]
fn sum_floats(arguments: Arguments<'_>) -> Value {
    let numbers = arguments.vec(0);
    Value::float(
        numbers
            .iter()
            .map(|number| {
                // SAFETY: the built-in declaration requires every vec element to be a float.
                unsafe { number.as_float_unchecked() }
            })
            .sum(),
    )
}

#[whim_function("Whim\\_Private\\math_div(int $numerator, int $denominator): null|int")]
fn div<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let numerator = arguments.int(0);
    let denominator = arguments.int(1);
    if denominator == 0 {
        let class = context.vm.intern(names::DIVISION_BY_ZERO_ERROR);
        return Err(context.vm.throw(class, "division by zero", 0));
    }

    Ok(numerator
        .checked_div(denominator)
        .map_or_else(Value::null, Value::int))
}

#[whim_function("Whim\\_Private\\math_to_base(int $number, int $base): string&!''")]
#[inline(always)]
fn to_base<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let number = arguments.int(0);
    let base = arguments.int(1);
    if number < 0 || !(2..=36).contains(&base) {
        let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
        return Err(context.vm.throw(
            class,
            "math_to_base requires a non-negative number and a base from 2 through 36",
            0,
        ));
    }

    // SAFETY: the surrounding invariant proves this result is successful.
    let number = unsafe {
        unwrap_result_invariant(
            u64::try_from(number),
            "a validated non-negative integer fits u64",
        )
    };
    // SAFETY: the surrounding invariant proves this result is successful.
    let base = unsafe { unwrap_result_invariant(u64::try_from(base), "a validated base fits u64") };

    Ok(encode_base_value(context, number, base))
}

#[whim_function("Whim\\Math\\from_base((string&!'') $number, 2..=36 $base): (0..)")]
fn from_base<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let number = arguments.bytes(0);
    let base = arguments.int(1);
    // SAFETY: the surrounding invariant proves this result is successful.
    let base = unsafe { unwrap_result_invariant(u64::try_from(base), "a validated base fits u64") };

    let result = match parse_base(number, base) {
        Ok(result) => {
            let Ok(result) = i64::try_from(result) else {
                let class = context.vm.intern(b"Whim\\Unwind\\OverflowException");
                return Err(context.vm.throw(
                    class,
                    "the converted integer does not fit in int",
                    0,
                ));
            };

            result
        }
        Err(BaseParseError::Overflow) => {
            let class = context.vm.intern(b"Whim\\Unwind\\OverflowException");
            return Err(context
                .vm
                .throw(class, "the converted integer does not fit in int", 0));
        }
        Err(BaseParseError::InvalidDigit) => {
            let class = context.vm.intern(b"Whim\\Unwind\\InvalidArgumentException");
            return Err(context
                .vm
                .throw(class, "the number contains a digit outside its base", 0));
        }
    };

    Ok(Value::int(result))
}

#[whim_function(
    "Whim\\Math\\base_convert((string&!'') $number, 2..=36 $fromBase, 2..=36 $toBase): (string&!'')"
)]
fn base_convert<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let number = arguments.bytes(0);
    let from_base = arguments.int(1);
    let to_base = arguments.int(2);
    let from_base =
        // SAFETY: the surrounding invariant proves this result is successful.
        unsafe { unwrap_result_invariant(u64::try_from(from_base), "a validated base fits u64") };
    let to_base =
        // SAFETY: the surrounding invariant proves this result is successful.
        unsafe { unwrap_result_invariant(u64::try_from(to_base), "a validated base fits u64") };

    match parse_base(number, from_base) {
        Ok(number) => {
            return Ok(encode_base_value(context, number, to_base));
        }
        Err(BaseParseError::InvalidDigit) => {
            let class = context.vm.intern(b"Whim\\Unwind\\InvalidArgumentException");
            return Err(context
                .vm
                .throw(class, "the number contains a digit outside its base", 0));
        }
        Err(BaseParseError::Overflow) => {}
    }

    let mut digits = Vec::with_capacity(number.len());
    for byte in number {
        let Some(digit) = base_digit(*byte) else {
            let class = context.vm.intern(b"Whim\\Unwind\\InvalidArgumentException");
            return Err(context
                .vm
                .throw(class, "the number contains a digit outside its base", 0));
        };

        if u64::from(digit) >= from_base {
            let class = context.vm.intern(b"Whim\\Unwind\\InvalidArgumentException");
            return Err(context
                .vm
                .throw(class, "the number contains a digit outside its base", 0));
        }

        if !digits.is_empty() || digit != 0 {
            digits.push(digit);
        }
    }

    if digits.is_empty() {
        return Ok(context.string(b"0"));
    }

    let from_base =
        // SAFETY: the surrounding invariant proves this result is successful.
        unsafe { unwrap_result_invariant(u32::try_from(from_base), "a supported base fits u32") };
    let to_base =
        // SAFETY: the surrounding invariant proves this result is successful.
        unsafe { unwrap_result_invariant(u32::try_from(to_base), "a supported base fits u32") };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let number = unsafe {
        unwrap_option_invariant(
            BigUint::from_radix_be(&digits, from_base),
            "validated digits form a big integer",
        )
    };
    let output = number.to_str_radix(to_base);

    Ok(context.owned_string(output.into_bytes()))
}

macro_rules! with_constant_base {
    ($base:expr, $function:ident($($argument:expr),*)) => {
        match $base {
            2 => $function::<2>($($argument),*),
            3 => $function::<3>($($argument),*),
            4 => $function::<4>($($argument),*),
            5 => $function::<5>($($argument),*),
            6 => $function::<6>($($argument),*),
            7 => $function::<7>($($argument),*),
            8 => $function::<8>($($argument),*),
            9 => $function::<9>($($argument),*),
            10 => $function::<10>($($argument),*),
            11 => $function::<11>($($argument),*),
            12 => $function::<12>($($argument),*),
            13 => $function::<13>($($argument),*),
            14 => $function::<14>($($argument),*),
            15 => $function::<15>($($argument),*),
            16 => $function::<16>($($argument),*),
            17 => $function::<17>($($argument),*),
            18 => $function::<18>($($argument),*),
            19 => $function::<19>($($argument),*),
            20 => $function::<20>($($argument),*),
            21 => $function::<21>($($argument),*),
            22 => $function::<22>($($argument),*),
            23 => $function::<23>($($argument),*),
            24 => $function::<24>($($argument),*),
            25 => $function::<25>($($argument),*),
            26 => $function::<26>($($argument),*),
            27 => $function::<27>($($argument),*),
            28 => $function::<28>($($argument),*),
            29 => $function::<29>($($argument),*),
            30 => $function::<30>($($argument),*),
            31 => $function::<31>($($argument),*),
            32 => $function::<32>($($argument),*),
            33 => $function::<33>($($argument),*),
            34 => $function::<34>($($argument),*),
            35 => $function::<35>($($argument),*),
            36 => $function::<36>($($argument),*),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe {
                crate::unreachable_invariant("a conversion base is between 2 and 36")
            },
        }
    };
}

const INVALID_DIGIT: u8 = 0xFF;

static DIGIT_VALUES: [u8; 256] = build_digit_values();

const fn build_digit_values() -> [u8; 256] {
    let mut table = [INVALID_DIGIT; 256];
    let mut digit = 0u8;
    while digit < 10 {
        table[(b'0' + digit) as usize] = digit;
        digit += 1;
    }

    let mut letter = 0u8;
    while letter < 26 {
        table[(b'a' + letter) as usize] = 10 + letter;
        table[(b'A' + letter) as usize] = 10 + letter;
        letter += 1;
    }

    table
}

fn parse_base(number: &[u8], base: u64) -> Result<u64, BaseParseError> {
    with_constant_base!(base, parse_fixed_base(number))
}

/// Checks overflow only after the input length can exceed `u64`.
#[inline(always)]
fn parse_fixed_base<const BASE: u64>(number: &[u8]) -> Result<u64, BaseParseError> {
    // SAFETY: the surrounding invariant proves this result is successful.
    let proven_digits = unsafe {
        unwrap_result_invariant(
            usize::try_from(u64::MAX.ilog(BASE)),
            "a u64 digit count fits usize",
        )
    };
    let (proven, careful) = if number.len() <= proven_digits {
        (number, &[][..])
    } else {
        number.split_at(proven_digits)
    };

    let mut result = 0u64;
    for &byte in proven {
        let digit = u64::from(DIGIT_VALUES[usize::from(byte)]);
        if digit >= BASE {
            return Err(BaseParseError::InvalidDigit);
        }

        result = result * BASE + digit;
    }

    for &byte in careful {
        let digit = u64::from(DIGIT_VALUES[usize::from(byte)]);
        if digit >= BASE {
            return Err(BaseParseError::InvalidDigit);
        }

        result = result
            .checked_mul(BASE)
            .and_then(|value| value.checked_add(digit))
            .ok_or(BaseParseError::Overflow)?;
    }

    Ok(result)
}

fn encode_base(number: u64, base: u64, buffer: &mut [u8; 64]) -> &[u8] {
    with_constant_base!(base, encode_fixed_base(number, buffer))
}

#[inline(never)]
fn encode_base_value(context: &Context<'_, '_, '_>, number: u64, base: u64) -> Value {
    if let Some(value) = encode_short_base(number, base) {
        return Value::short_string(value);
    }

    let mut buffer = [0u8; 64];
    context.string(encode_base(number, base, &mut buffer))
}

#[inline(always)]
fn encode_short_base(number: u64, base: u64) -> Option<ShortString> {
    if base.is_power_of_two() {
        return encode_short_power_of_two(number, base);
    }

    with_constant_base!(base, encode_short_fixed_base(number))
}

#[inline(always)]
fn encode_short_power_of_two(mut number: u64, base: u64) -> Option<ShortString> {
    let shift = base.trailing_zeros();
    let mask = base - 1;
    let mut packed = 0u64;
    let mut length = 0u8;
    loop {
        packed = (packed << 8) | u64::from(BASE_DIGITS[base_digit_index(number & mask)]);
        length += 1;
        number >>= shift;
        if number == 0 {
            // SAFETY: each packed byte is ASCII and `length` is within capacity.
            return Some(unsafe { ShortString::from_packed_unchecked(packed, length) });
        }
        if usize::from(length) == ShortString::CAPACITY {
            return None;
        }
    }
}

#[inline(always)]
fn encode_short_fixed_base<const BASE: u64>(mut number: u64) -> Option<ShortString> {
    let mut packed = 0u64;
    let mut length = 0u8;
    loop {
        packed = (packed << 8) | u64::from(BASE_DIGITS[base_digit_index(number % BASE)]);
        length += 1;
        number /= BASE;
        if number == 0 {
            // SAFETY: each packed byte is ASCII and `length` is within capacity.
            return Some(unsafe { ShortString::from_packed_unchecked(packed, length) });
        }
        if usize::from(length) == ShortString::CAPACITY {
            return None;
        }
    }
}

#[inline(always)]
fn encode_fixed_base<const BASE: u64>(mut number: u64, buffer: &mut [u8; 64]) -> &[u8] {
    let mut cursor = buffer.len();
    loop {
        cursor -= 1;
        buffer[cursor] = BASE_DIGITS[base_digit_index(number % BASE)];
        number /= BASE;
        if number == 0 {
            return &buffer[cursor..];
        }
    }
}

fn base_digit(byte: u8) -> Option<u8> {
    match DIGIT_VALUES[usize::from(byte)] {
        INVALID_DIGIT => None,
        digit => Some(digit),
    }
}

#[inline(always)]
fn base_digit_index(digit: u64) -> usize {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            usize::try_from(digit),
            "a digit in a supported base fits usize",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_base_encoding_matches_the_full_encoder() {
        for base in 2..=36u64 {
            let seventh_power = base.checked_pow(7);
            let values = [
                0,
                1,
                base - 1,
                base,
                seventh_power
                    .and_then(|value| value.checked_sub(1))
                    .unwrap_or(u64::MAX),
                seventh_power.unwrap_or(u64::MAX),
                u64::MAX,
            ];
            for value in values {
                let short = encode_short_base(value, base);
                let mut buffer = [0u8; 64];
                let encoded = encode_base(value, base, &mut buffer);
                match short {
                    Some(short) => assert_eq!(short.as_bytes(), encoded),
                    None => assert!(encoded.len() > ShortString::CAPACITY),
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Number {
    Int(i64),
    Float(f64),
}

fn number(value: &Value) -> Number {
    value.as_int().map_or_else(
        || {
            // SAFETY: the surrounding invariant proves this option contains a value.
            Number::Float(unsafe {
                unwrap_option_invariant(value.as_float(), "a numeric value is an int or float")
            })
        },
        Number::Int,
    )
}

fn compare_numbers(left: Number, right: Number) -> Option<Ordering> {
    match (left, right) {
        (Number::Int(left), Number::Int(right)) => left.partial_cmp(&right),
        (Number::Float(left), Number::Float(right)) => left.partial_cmp(&right),
        (Number::Int(left), Number::Float(right)) => compare_int_float(left, right),
        (Number::Float(left), Number::Int(right)) => {
            compare_int_float(right, left).map(Ordering::reverse)
        }
    }
}

#[whim_function("Whim\\_Private\\math_compare(int|float $left, int|float $right): int")]
fn compare(arguments: Arguments<'_>) -> Value {
    let left = arguments.local(0);
    let right = arguments.local(1);
    let result = match compare_numbers(number(&left), number(&right)) {
        Some(Ordering::Less) => -1,
        Some(Ordering::Equal) | None => 0,
        Some(Ordering::Greater) => 1,
    };

    Value::int(result)
}

macro_rules! unary_float {
    ($rust:ident, $name:literal, $operation:expr) => {
        #[whim_function($name)]
        fn $rust(arguments: Arguments<'_>) -> Value {
            let number = arguments.float(0);
            Value::float($operation(number))
        }
    };
}

unary_float!(
    sqrt,
    "Whim\\_Private\\math_sqrt(float $number): float",
    f64::sqrt
);
unary_float!(
    exp,
    "Whim\\_Private\\math_exp(float $number): float",
    f64::exp
);
unary_float!(ln, "Whim\\_Private\\math_ln(float $number): float", f64::ln);
unary_float!(
    floor,
    "Whim\\_Private\\math_floor(float $number): float",
    f64::floor
);
unary_float!(
    ceil,
    "Whim\\_Private\\math_ceil(float $number): float",
    f64::ceil
);
unary_float!(
    sin,
    "Whim\\_Private\\math_sin(float $number): float",
    f64::sin
);
unary_float!(
    cos,
    "Whim\\_Private\\math_cos(float $number): float",
    f64::cos
);
unary_float!(
    tan,
    "Whim\\_Private\\math_tan(float $number): float",
    f64::tan
);
unary_float!(
    asin,
    "Whim\\_Private\\math_asin(float $number): float",
    f64::asin
);
unary_float!(
    acos,
    "Whim\\_Private\\math_acos(float $number): float",
    f64::acos
);
unary_float!(
    atan,
    "Whim\\_Private\\math_atan(float $number): float",
    f64::atan
);

#[whim_function("Whim\\_Private\\math_log_base(float $number, float $base): float")]
fn log_base(arguments: Arguments<'_>) -> Value {
    let number = arguments.float(0);
    let base = arguments.float(1);
    Value::float(number.log(base))
}

#[whim_function("Whim\\_Private\\math_atan2(float $y, float $x): float")]
fn atan2(arguments: Arguments<'_>) -> Value {
    let y = arguments.float(0);
    let x = arguments.float(1);
    Value::float(y.atan2(x))
}

#[whim_function("Whim\\_Private\\math_round(float $number, int $precision): float")]
fn round(arguments: Arguments<'_>) -> Value {
    let number = arguments.float(0);
    let precision = arguments.int(1);
    if !number.is_finite() {
        return Value::float(number);
    }

    // SAFETY: the surrounding invariant proves this result is successful.
    let precision = unsafe {
        unwrap_result_invariant(
            i32::try_from(precision.clamp(i64::from(i32::MIN), i64::from(i32::MAX))),
            "a clamped i32 precision fits i32",
        )
    };
    let factor = 10.0_f64.powi(precision);
    if factor.is_infinite() {
        return Value::float(number);
    }
    if factor == 0.0 {
        return Value::float(0.0_f64.copysign(number));
    }

    let scaled = number * factor;
    if scaled.is_infinite() {
        return Value::float(number);
    }

    Value::float(scaled.round() / factor)
}

#[whim_constant("Whim\\_Private\\MATH_INT_MAX", "int")]
const INT_MAX: i64 = i64::MAX;

#[whim_constant("Whim\\_Private\\MATH_INT_MIN", "int")]
const INT_MIN: i64 = i64::MIN;

#[whim_constant("Whim\\_Private\\MATH_FLOAT_MAX", "float")]
const FLOAT_MAX: f64 = f64::MAX;

#[whim_constant("Whim\\_Private\\MATH_FLOAT_MIN", "float")]
const FLOAT_MIN: f64 = f64::MIN;

#[whim_constant("Whim\\_Private\\MATH_FLOAT_EPSILON", "float")]
const FLOAT_EPSILON: f64 = f64::EPSILON;

#[whim_constant("Whim\\_Private\\MATH_NAN", "float")]
const NAN: f64 = f64::NAN;

#[whim_constant("Whim\\_Private\\MATH_INFINITY", "float")]
const INFINITY: f64 = f64::INFINITY;

#[whim_constant("Whim\\_Private\\MATH_E", "float")]
const E: f64 = f64::consts::E;

#[whim_constant("Whim\\_Private\\MATH_PI", "float")]
const PI: f64 = f64::consts::PI;

#[whim_constant("Whim\\_Private\\MATH_FLOAT32_MAX", "float")]
const FLOAT32_MAX: f64 = f32::MAX as f64;

#[whim_constant("Whim\\_Private\\MATH_FLOAT32_MIN", "float")]
const FLOAT32_MIN: f64 = -(f32::MAX as f64);
