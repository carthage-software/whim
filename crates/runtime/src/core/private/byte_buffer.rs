//! Mutable byte storage used by Whim-written binary code.

use std::cell::RefCell;

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::value::Value;

const CLASS: &str = "Whim\\_Private\\ByteBuffer";

#[whim_class("Whim\\_Private\\ByteBuffer", final)]
#[derive(Default)]
pub(crate) struct ByteBuffer {
    bytes: RefCell<Vec<u8>>,
}

default_built_in_state!(ByteBuffer);

#[whim_methods]
impl ByteBuffer {
    #[whim_method(
        "__construct(int $capacity = 0): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let capacity = if arguments.is_absent(0) {
            0
        } else {
            non_negative(context, arguments.int(0), "buffer capacity")?
        };
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| value_error(context, "the byte buffer capacity is too large"))?;
        *context.state::<Self>()?.bytes.borrow_mut() = bytes;
        Ok(Value::null())
    }

    #[whim_method(
        "fromString(string $bytes): Whim\\_Private\\ByteBuffer",
        static,
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn from_string(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.bytes(0).to_vec();
        let object = context.new_built_in_instance(CLASS)?;
        // SAFETY: the surrounding invariant proves this option contains a value.
        let state = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&object),
                "a new byte buffer has its built-in state",
            )
        };
        *state.bytes.borrow_mut() = bytes;
        Ok(object)
    }

    #[whim_method("append(string $bytes): void", no_track_caller, no_trace_boundary)]
    fn append(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let input = arguments.bytes(0);
        context
            .state::<Self>()?
            .bytes
            .borrow_mut()
            .extend_from_slice(input);
        Ok(Value::null())
    }

    #[whim_method(
        "appendAll(vec<string> $chunks): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn append_all(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let chunks = arguments.vec(0);
        let additional = chunks.iter().try_fold(0usize, |length, chunk| {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let bytes = unsafe {
                unwrap_option_invariant(
                    chunk.as_string_bytes(),
                    "a validated byte-buffer chunk is a string",
                )
            };
            length.checked_add(bytes.len())
        });
        let Some(additional) = additional else {
            return Err(value_error(
                context,
                "the combined byte-buffer chunks are too large",
            ));
        };
        let mut buffer = context.state::<Self>()?.bytes.borrow_mut();
        buffer.reserve(additional);
        for chunk in chunks.iter() {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let bytes = unsafe {
                unwrap_option_invariant(
                    chunk.as_string_bytes(),
                    "a validated byte-buffer chunk is a string",
                )
            };
            buffer.extend_from_slice(bytes);
        }
        Ok(Value::null())
    }

    #[whim_method(
        "write(int $offset, string $bytes): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn write(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let offset = non_negative(context, arguments.int(0), "buffer offset")?;
        let input = arguments.bytes(1);
        let end = checked_end(context, offset, input.len())?;
        let mut bytes = context.state::<Self>()?.bytes.borrow_mut();
        if offset == bytes.len() {
            bytes.extend_from_slice(input);
            return Ok(Value::null());
        }

        if bytes.len() < end {
            bytes.resize(end, 0);
        }
        bytes[offset..end].copy_from_slice(input);
        Ok(Value::null())
    }

    #[whim_method(
        "read(int $offset, int $length): string",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn read(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let offset = non_negative(context, arguments.int(0), "buffer offset")?;
        let length = non_negative(context, arguments.int(1), "buffer length")?;
        let end = checked_end(context, offset, length)?;
        let state = context.state::<Self>()?;
        let bytes = state.bytes.borrow();
        let result = bytes.get(offset..end).map(<[u8]>::to_vec);
        drop(bytes);
        let Some(result) = result else {
            return Err(value_error(
                context,
                "the byte buffer range exceeds its length",
            ));
        };
        Ok(Value::from_string_vec(context.vm.heap(), result))
    }

    #[whim_method(
        "appendInteger(int $value, int $width, bool $little): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn append_integer(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let value = arguments.int(0);
        let width = width(context, arguments.int(1))?;
        let little = arguments.bool(2);
        let mut bytes = context.state::<Self>()?.bytes.borrow_mut();
        append_integer_bytes(&mut bytes, value, width, little);
        Ok(Value::null())
    }

    #[whim_method(
        "appendFloat(float $value, int $width, bool $little): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn append_float(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let value = arguments.float(0);
        let width = float_width(context, arguments.int(1))?;
        let little = arguments.bool(2);
        let mut bytes = context.state::<Self>()?.bytes.borrow_mut();
        append_float_bytes(&mut bytes, value, width, little);
        Ok(Value::null())
    }

    #[whim_method("toString(): string", no_track_caller, no_trace_boundary, must_use)]
    fn to_string(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let bytes = context.state::<Self>()?.bytes.borrow().clone();
        Ok(Value::from_string_vec(context.vm.heap(), bytes))
    }
}

fn non_negative(context: &mut Context<'_, '_, '_>, value: i64, name: &str) -> Result<usize, Throw> {
    usize::try_from(value)
        .map_err(|_| value_error(context, &format!("the {name} must be non-negative")))
}

fn checked_end(
    context: &mut Context<'_, '_, '_>,
    offset: usize,
    length: usize,
) -> Result<usize, Throw> {
    offset
        .checked_add(length)
        .ok_or_else(|| value_error(context, "the byte buffer range is too large"))
}

fn width(context: &mut Context<'_, '_, '_>, value: i64) -> Result<usize, Throw> {
    match value {
        1 => Ok(1),
        2 => Ok(2),
        4 => Ok(4),
        8 => Ok(8),
        _ => Err(value_error(
            context,
            "the integer width must be 1, 2, 4, or 8 bytes",
        )),
    }
}

fn float_width(context: &mut Context<'_, '_, '_>, value: i64) -> Result<usize, Throw> {
    match value {
        4 => Ok(4),
        8 => Ok(8),
        _ => Err(value_error(context, "the float width must be 4 or 8 bytes")),
    }
}

fn value_error(context: &mut Context<'_, '_, '_>, message: &str) -> Throw {
    let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
    context.vm.throw(class, message, 0)
}

fn append_integer_bytes(bytes: &mut Vec<u8>, value: i64, width: usize, little: bool) {
    let encoded = if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    if little {
        bytes.extend_from_slice(&encoded[..width]);
    } else {
        bytes.extend_from_slice(&encoded[encoded.len() - width..]);
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a four-byte write explicitly narrows a double to single precision"
)]
fn append_float_bytes(bytes: &mut Vec<u8>, value: f64, width: usize, little: bool) {
    if width == 4 {
        let encoded = if little {
            (value as f32).to_le_bytes()
        } else {
            (value as f32).to_be_bytes()
        };
        bytes.extend_from_slice(&encoded);
    } else {
        let encoded = if little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        bytes.extend_from_slice(&encoded);
    }
}
