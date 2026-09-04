//! Unstable float representation primitives used by `Whim\Float`.

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::unwrap_result_invariant;
use crate::value::Value;

#[whim_function("Whim\\_Private\\float_to_bits(float $value): int")]
fn to_bits(arguments: Arguments<'_>) -> Value {
    Value::int(i64::from_ne_bytes(arguments.float(0).to_ne_bytes()))
}

#[whim_function("Whim\\_Private\\bits_to_float(int $bits): float")]
fn from_bits(arguments: Arguments<'_>) -> Value {
    Value::float(f64::from_ne_bytes(arguments.int(0).to_ne_bytes()))
}

#[whim_function("Whim\\_Private\\float_to_bits32(float $value): 0..=4294967295")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "this operation explicitly narrows a double to single precision"
)]
fn to_bits32(arguments: Arguments<'_>) -> Value {
    Value::int(i64::from((arguments.float(0) as f32).to_bits()))
}

#[whim_function("Whim\\_Private\\bits32_to_float(int $bits): float")]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a 32-bit representation uses the low bits of the supplied integer"
)]
fn from_bits32(arguments: Arguments<'_>) -> Value {
    Value::float(f64::from(f32::from_bits(arguments.int(0) as u32)))
}

#[whim_function("Whim\\_Private\\float_to_be_bytes(float $value): string[8]")]
fn to_be_bytes(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    context.string(&arguments.float(0).to_be_bytes())
}

#[whim_function("Whim\\_Private\\float_from_be_bytes(string[8] $bytes): float")]
fn from_be_bytes(arguments: Arguments<'_>) -> Value {
    Value::float(f64::from_be_bytes(float_bytes(arguments)))
}

#[whim_function("Whim\\_Private\\float_to_le_bytes(float $value): string[8]")]
fn to_le_bytes(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    context.string(&arguments.float(0).to_le_bytes())
}

#[whim_function("Whim\\_Private\\float_from_le_bytes(string[8] $bytes): float")]
fn from_le_bytes(arguments: Arguments<'_>) -> Value {
    Value::float(f64::from_le_bytes(float_bytes(arguments)))
}

fn float_bytes(arguments: Arguments<'_>) -> [u8; 8] {
    // SAFETY: the argument type guarantees exactly eight bytes.
    unsafe {
        unwrap_result_invariant(
            arguments.bytes(0).try_into(),
            "a float byte string contains eight bytes",
        )
    }
}
