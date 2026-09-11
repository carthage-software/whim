//! Unstable random primitives exposed to the Whim standard library.

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

#[whim_function("Whim\\_Private\\random_bytes(int $length): null|string")]
pub(crate) fn random_bytes(scope: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let length = arguments.int(0);
    let Ok(length) = usize::try_from(length) else {
        return Value::null();
    };

    let mut bytes = vec![0_u8; length];
    if getrandom::fill(&mut bytes).is_err() {
        return Value::null();
    }

    scope.owned_string(bytes)
}

#[whim_function("Whim\\_Private\\random_int(): null|int")]
pub(crate) fn random_int() -> Value {
    let mut bytes = [0; 8];
    if getrandom::fill(&mut bytes).is_err() {
        return Value::null();
    }

    Value::int(i64::from_ne_bytes(bytes))
}

#[whim_function("Whim\\_Private\\random_string(int $length, string[2..] $alphabet): null|string")]
pub(crate) fn random_string(scope: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Ok(length) = usize::try_from(arguments.int(0)) else {
        return Value::null();
    };
    let alphabet = arguments.bytes(1);
    let bits = usize::BITS - (alphabet.len() - 1).leading_zeros();
    let width = bits.div_ceil(8) as usize;
    let mask = usize::MAX >> (usize::BITS - bits);
    let mut result = Vec::with_capacity(length);
    let mut buffer = [0; 256];
    while result.len() < length {
        let draws = (length - result.len()).min(buffer.len() / width);
        let bytes = &mut buffer[..draws * width];
        if getrandom::fill(bytes).is_err() {
            return Value::null();
        }
        for draw in bytes.chunks_exact(width) {
            let mut position = [0; size_of::<usize>()];
            position[..width].copy_from_slice(draw);
            let position = usize::from_le_bytes(position) & mask;
            if let Some(byte) = alphabet.get(position) {
                result.push(*byte);
            }
        }
    }

    scope.owned_string(result)
}
