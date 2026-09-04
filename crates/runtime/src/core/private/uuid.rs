//! UUID generation and conversion.

use std::str::from_utf8;

use uuid::Uuid as RawUuid;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::unwrap_result_invariant;
use crate::value::Value;

#[whim_function("Whim\\_Private\\uuid_v4(): string[16]", must_use)]
pub(crate) fn v4(context: &Context<'_, '_, '_>) -> Value {
    context.string(RawUuid::new_v4().as_bytes())
}

#[whim_function("Whim\\_Private\\uuid_v7(): string[16]", must_use)]
pub(crate) fn v7(context: &Context<'_, '_, '_>) -> Value {
    context.string(RawUuid::now_v7().as_bytes())
}

#[whim_function("Whim\\_Private\\uuid_parse(string $value): null|string[16]", must_use)]
pub(crate) fn parse(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let Ok(value) = from_utf8(value) else {
        return Value::null();
    };
    let Ok(uuid) = RawUuid::try_parse(value) else {
        return Value::null();
    };
    let mut buffer = RawUuid::encode_buffer();
    if uuid.hyphenated().encode_lower(&mut buffer) != value {
        return Value::null();
    }

    context.string(uuid.as_bytes())
}

#[whim_function("Whim\\_Private\\uuid_format(string[16] $bytes): string[36]", must_use)]
pub(crate) fn format(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    // SAFETY: the argument type guarantees exactly 16 bytes.
    let bytes = unsafe {
        unwrap_result_invariant(
            arguments.bytes(0).try_into(),
            "a UUID byte string contains 16 bytes",
        )
    };
    let uuid = RawUuid::from_bytes(bytes);
    let mut buffer = RawUuid::encode_buffer();
    let formatted = uuid.hyphenated().encode_lower(&mut buffer);

    context.string(formatted.as_bytes())
}

#[whim_function(
    "Whim\\_Private\\uuid_version(string[16] $bytes): null|1..=8",
    must_use
)]
pub(crate) fn version(arguments: Arguments<'_>) -> Value {
    // SAFETY: the argument type guarantees exactly 16 bytes.
    let bytes = unsafe {
        unwrap_result_invariant(
            arguments.bytes(0).try_into(),
            "a UUID byte string contains 16 bytes",
        )
    };
    let uuid = RawUuid::from_bytes(bytes);
    if uuid.get_version().is_none() {
        return Value::null();
    }

    // SAFETY: the surrounding invariant proves this result is successful.
    let version = unsafe {
        unwrap_result_invariant(
            i64::try_from(uuid.get_version_num()),
            "a UUID version nibble fits in a signed integer",
        )
    };
    if !(1..=8).contains(&version) {
        return Value::null();
    }

    Value::int(version)
}
