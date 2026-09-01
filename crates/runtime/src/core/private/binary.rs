//! Unstable fixed-width binary primitives used by `Whim\Binary`.

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::unreachable_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;

#[whim_function(
    "Whim\\_Private\\binary_decode(string $bytes, int $offset, int $width, bool $signed, bool $little): null|int"
)]
fn decode(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let offset = arguments.int(1);
    let width = binary_width(context, arguments.int(2))?;
    let signed = arguments.bool(3);
    let little = arguments.bool(4);
    let Ok(offset) = usize::try_from(offset) else {
        return invalid_offset(context);
    };
    let Some(end) = offset.checked_add(width) else {
        return invalid_offset(context);
    };
    let Some(bytes) = bytes.get(offset..end) else {
        return Ok(Value::null());
    };

    let value = match (width, signed, little) {
        (1, false, _) => i64::from(bytes[0]),
        (1, true, _) => i64::from(bytes[0].cast_signed()),
        (2, false, false) => i64::from(u16::from_be_bytes(fixed(bytes))),
        (2, false, true) => i64::from(u16::from_le_bytes(fixed(bytes))),
        (2, true, false) => i64::from(i16::from_be_bytes(fixed(bytes))),
        (2, true, true) => i64::from(i16::from_le_bytes(fixed(bytes))),
        (4, false, false) => i64::from(u32::from_be_bytes(fixed(bytes))),
        (4, false, true) => i64::from(u32::from_le_bytes(fixed(bytes))),
        (4, true, false) => i64::from(i32::from_be_bytes(fixed(bytes))),
        (4, true, true) => i64::from(i32::from_le_bytes(fixed(bytes))),
        (8, true, false) => i64::from_be_bytes(fixed(bytes)),
        (8, true, true) => i64::from_le_bytes(fixed(bytes)),
        (8, false, false) => match i64::try_from(u64::from_be_bytes(fixed(bytes))) {
            Ok(value) => value,
            Err(_) => return Ok(Value::null()),
        },
        (8, false, true) => match i64::try_from(u64::from_le_bytes(fixed(bytes))) {
            Ok(value) => value,
            Err(_) => return Ok(Value::null()),
        },
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("binary widths are limited to 1, 2, 4, or 8") },
    };

    Ok(Value::int(value))
}

#[whim_function("Whim\\_Private\\binary_encode(int $value, int $width, bool $little): string")]
fn encode(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let value = arguments.int(0);
    let width = binary_width(context, arguments.int(1))?;
    let little = arguments.bool(2);
    let bytes = if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    let bytes = if little {
        &bytes[..width]
    } else {
        &bytes[bytes.len() - width..]
    };

    Ok(context.string(bytes))
}

fn binary_width(context: &mut Context<'_, '_, '_>, width: i64) -> Result<usize, Throw> {
    match width {
        1 => Ok(1),
        2 => Ok(2),
        4 => Ok(4),
        8 => Ok(8),
        _ => {
            let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
            Err(context
                .vm
                .throw(class, "binary width must be 1, 2, 4, or 8 bytes", 0))
        }
    }
}

fn invalid_offset(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
    Err(context
        .vm
        .throw(class, "binary offset must be a non-negative integer", 0))
}

fn fixed<const N: usize>(bytes: &[u8]) -> [u8; N] {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            bytes.try_into(),
            "the selected binary width matches the fixed-width decoder",
        )
    }
}
