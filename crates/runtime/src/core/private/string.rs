//! Unstable byte primitives exposed to the Whim standard library.

use memchr::memchr as find_byte;
use memchr::memmem::find as find_bytes;
use memchr::memmem::find_iter as find_bytes_positions;
use memchr::memmem::rfind as find_bytes_reverse;
use memchr::memrchr as find_byte_reverse;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::classes::names;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::string::ByteStringObject;

#[whim_function("Whim\\_Private\\string_to_bytes(string $value): vec<0..=255>")]
pub(crate) fn string_to_bytes<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.bytes(0);
    context.vec(value.iter().map(|byte| Value::int(i64::from(*byte))))
}

#[whim_function("Whim\\_Private\\string_from_bytes(vec<0..=255> $bytes): string")]
pub(crate) fn string_from_bytes<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    let mut bytes = Vec::with_capacity(values.len());
    for value in values.iter() {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let value = unsafe {
            unwrap_option_invariant(value.as_int(), "a validated byte vector contains integers")
        };
        // SAFETY: the surrounding invariant proves this result is successful.
        let byte = unsafe {
            unwrap_result_invariant(
                u8::try_from(value),
                "a validated byte-vector integer fits u8",
            )
        };
        bytes.push(byte);
    }

    context.string(&bytes)
}

#[whim_function(
    "Whim\\_Private\\string_slice(string $string, (0..) $offset, (0..) $length): string"
)]
pub(crate) fn string_slice<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    // SAFETY: the surrounding invariant proves this result is successful.
    let offset = unsafe {
        unwrap_result_invariant(
            usize::try_from(arguments.int(1)),
            "a validated string offset fits usize",
        )
    };
    // SAFETY: the surrounding invariant proves this result is successful.
    let length = unsafe {
        unwrap_result_invariant(
            usize::try_from(arguments.int(2)),
            "a validated string length fits usize",
        )
    };

    let Some(end) = offset.checked_add(length) else {
        let class = context.vm.intern(names::OUT_OF_BOUNDS_ERROR);
        return Err(context
            .vm
            .throw(class, "string slice end is out of bounds", 0));
    };
    if end > bytes.len() {
        let class = context.vm.intern(names::OUT_OF_BOUNDS_ERROR);
        return Err(context
            .vm
            .throw(class, "string slice end is out of bounds", 0));
    }

    if length <= 7 {
        return Ok(context.string(&bytes[offset..end]));
    }

    let string = arguments.string(0);
    if offset == 0 && length == string.len() {
        return Ok(Value::string(string));
    }

    Ok(Value::string(ByteStringObject::slice(
        context.vm.heap(),
        &string,
        offset,
        length,
    )))
}

#[whim_function("Whim\\_Private\\string_byte_at(string $string, (0..) $offset): 0..=255")]
pub(crate) fn string_byte_at<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let offset = string_index(arguments.int(1));
    let Some(byte) = bytes.get(offset) else {
        let class = context.vm.intern(names::OUT_OF_BOUNDS_ERROR);
        return Err(context.vm.throw(class, "string offset is out of bounds", 0));
    };

    Ok(Value::int(i64::from(*byte)))
}

#[whim_function(
    "Whim\\_Private\\memchr(string $haystack, 0..=255 $needle, (0..) $offset): null|(0..)"
)]
pub(crate) fn memchr(arguments: Arguments<'_>) -> Value {
    let haystack = arguments.bytes(0);
    let needle = byte_value(arguments.int(1));
    let offset = string_index(arguments.int(2));
    let Some(haystack) = haystack.get(offset..) else {
        return Value::null();
    };

    search_result(find_byte(needle, haystack), offset)
}

#[whim_function(
    "Whim\\_Private\\memrchr(string $haystack, 0..=255 $needle, (0..) $offset): null|(0..)"
)]
pub(crate) fn memrchr(arguments: Arguments<'_>) -> Value {
    let haystack = arguments.bytes(0);
    let needle = byte_value(arguments.int(1));
    let offset = string_index(arguments.int(2));
    let Some(haystack) = haystack.get(offset..) else {
        return Value::null();
    };

    search_result(find_byte_reverse(needle, haystack), offset)
}

#[whim_function(
    "Whim\\_Private\\memmem(string $haystack, string $needle, (0..) $offset, bool $ci): null|(0..)"
)]
pub(crate) fn memmem(arguments: Arguments<'_>) -> Value {
    let haystack = arguments.bytes(0);
    let needle = arguments.bytes(1);
    let offset = string_index(arguments.int(2));
    let ci = arguments.bool(3);
    let Some(haystack) = haystack.get(offset..) else {
        return Value::null();
    };

    if ci {
        let haystack = haystack.to_ascii_lowercase();
        let needle = needle.to_ascii_lowercase();
        return search_result(find_bytes(&haystack, &needle), offset);
    }

    search_result(find_bytes(haystack, needle), offset)
}

#[whim_function(
    "Whim\\_Private\\memrmem(string $haystack, string $needle, (0..) $offset, bool $ci): null|(0..)"
)]
pub(crate) fn memrmem(arguments: Arguments<'_>) -> Value {
    let haystack = arguments.bytes(0);
    let needle = arguments.bytes(1);
    let offset = string_index(arguments.int(2));
    let ci = arguments.bool(3);
    let Some(haystack) = haystack.get(offset..) else {
        return Value::null();
    };

    if ci {
        let haystack = haystack.to_ascii_lowercase();
        let needle = needle.to_ascii_lowercase();
        return search_result(find_bytes_reverse(&haystack, &needle), offset);
    }

    search_result(find_bytes_reverse(haystack, needle), offset)
}

#[whim_function(
    "Whim\\_Private\\string_split(string $string, string $delimiter, (0..) $limit): vec<string>"
)]
pub(crate) fn string_split<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let string = arguments.string(0);
    let delimiter = arguments.bytes(1);
    let limit = arguments.int(2);
    let haystack = ByteStringObject::handle_bytes(&string);
    if delimiter.is_empty() {
        let whole = Value::string(string.clone());
        return context.vec([whole]);
    }

    let limit = string_index(limit);
    let mut parts: Vec<Value> = Vec::new();
    let mut start = 0usize;
    for position in find_bytes_positions(haystack, delimiter) {
        if limit != 0 && parts.len() + 1 == limit {
            break;
        }

        parts.push(Value::string(ByteStringObject::slice(
            context.vm.heap(),
            &string,
            start,
            position - start,
        )));
        start = position + delimiter.len();
    }

    parts.push(Value::string(ByteStringObject::slice(
        context.vm.heap(),
        &string,
        start,
        haystack.len() - start,
    )));
    context.vec(parts)
}

#[whim_function("Whim\\_Private\\string_join(vec<string> $values, string $separator): string")]
pub(crate) fn string_join<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    let separator = arguments.bytes(1);
    let mut result: Vec<u8> = Vec::new();
    let mut first = true;
    for value in values.iter() {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let bytes = unsafe {
            unwrap_option_invariant(
                value.as_string_bytes(),
                "a validated string vector contains strings",
            )
        };
        if !first {
            result.extend_from_slice(separator);
        }

        result.extend_from_slice(bytes);
        first = false;
    }

    context.string(&result)
}

#[whim_function(
    "Whim\\_Private\\string_replace(string $haystack, string $needle, string $replacement, bool $ci): string"
)]
pub(crate) fn string_replace<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let string = arguments.string(0);
    let needle = arguments.bytes(1);
    let replacement = arguments.bytes(2);
    let ci = arguments.bool(3);
    let haystack = ByteStringObject::handle_bytes(&string);
    if needle.is_empty() {
        return Value::string(string);
    }

    let mut result = Vec::with_capacity(haystack.len());
    let mut start = 0usize;
    if ci {
        let folded_haystack = haystack.to_ascii_lowercase();
        let folded_needle = needle.to_ascii_lowercase();
        for position in find_bytes_positions(&folded_haystack, &folded_needle) {
            result.extend_from_slice(&haystack[start..position]);
            result.extend_from_slice(replacement);
            start = position + needle.len();
        }
    } else {
        for position in find_bytes_positions(haystack, needle) {
            result.extend_from_slice(&haystack[start..position]);
            result.extend_from_slice(replacement);
            start = position + needle.len();
        }
    }

    if start == 0 {
        return Value::string(string);
    }

    result.extend_from_slice(&haystack[start..]);
    context.string(&result)
}

#[whim_function("Whim\\_Private\\string_ord(string[1] $character): 0..=255")]
pub(crate) fn string_ord(arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let byte = unsafe {
        unwrap_option_invariant(
            value.first(),
            "a validated string_ord argument is not empty",
        )
    };

    Value::int(i64::from(*byte))
}

#[whim_function("Whim\\_Private\\string_chr(0..=255 $byte): string[1]")]
pub(crate) fn string_chr<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let byte = byte_value(arguments.int(0));

    context.string(&[byte])
}

#[whim_function("Whim\\_Private\\string_trim(string $string, string $mask, 0..=2 $mode): string")]
pub(crate) fn string_trim<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let string = arguments.string(0);
    let mask = arguments.bytes(1);
    let mode = arguments.int(2);
    let bytes = ByteStringObject::handle_bytes(&string);
    let table = byte_mask_table(mask);
    let mut start = 0usize;
    let mut end = bytes.len();
    if mode <= 1 {
        while start < end && table[usize::from(bytes[start])] {
            start += 1;
        }
    }

    if mode >= 1 {
        while end > start && table[usize::from(bytes[end - 1])] {
            end -= 1;
        }
    }

    if start == 0 && end == bytes.len() {
        return Value::string(string);
    }

    Value::string(ByteStringObject::slice(
        context.vm.heap(),
        &string,
        start,
        end - start,
    ))
}

#[whim_function(
    "Whim\\_Private\\string_pad(string $string, (0..) $length, (string&!'') $pad, 0..=2 $mode): string"
)]
pub(crate) fn string_pad<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let string = arguments.string(0);
    let length = arguments.int(1);
    let pad = arguments.bytes(2);
    let mode = arguments.int(3);
    let bytes = ByteStringObject::handle_bytes(&string);
    let length = string_index(length);
    if length <= bytes.len() {
        return Value::string(string);
    }

    let needed = length - bytes.len();
    let (left, right) = match mode {
        0 => (needed, 0),
        2 => (0, needed),
        _ => (needed / 2, needed - needed / 2),
    };
    let mut result = Vec::with_capacity(length);
    result.extend(pad.iter().cycle().take(left));
    result.extend_from_slice(bytes);
    result.extend(pad.iter().cycle().take(right));
    context.string(&result)
}

#[whim_function("Whim\\_Private\\string_lowercase(string $string): string")]
pub(crate) fn string_lowercase<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let string = arguments.string(0);
    let bytes = ByteStringObject::handle_bytes(&string);
    if !bytes.iter().any(u8::is_ascii_uppercase) {
        return Value::string(string);
    }

    context.string(&bytes.to_ascii_lowercase())
}

#[whim_function("Whim\\_Private\\string_uppercase(string $string): string")]
pub(crate) fn string_uppercase<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let string = arguments.string(0);
    let bytes = ByteStringObject::handle_bytes(&string);
    if !bytes.iter().any(u8::is_ascii_lowercase) {
        return Value::string(string);
    }

    context.string(&bytes.to_ascii_uppercase())
}

#[whim_function(
    "Whim\\_Private\\string_capitalize_words(string $string, string $delimiters): string"
)]
pub(crate) fn string_capitalize_words<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let bytes = arguments.bytes(0);
    let delimiters = arguments.bytes(1);
    let mut delimiter_table = [false; 256];
    for delimiter in delimiters {
        delimiter_table[usize::from(*delimiter)] = true;
    }

    let mut result = Vec::with_capacity(bytes.len());
    let mut capitalize = true;
    for byte in bytes {
        result.push(if capitalize {
            byte.to_ascii_uppercase()
        } else {
            *byte
        });
        capitalize = delimiter_table[usize::from(*byte)];
    }

    Value::from_string_vec(context.vm.heap(), result)
}

#[whim_function("Whim\\_Private\\string_reverse(string $string): string")]
pub(crate) fn string_reverse<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let bytes = arguments.bytes(0);
    let result = bytes.iter().rev().copied().collect();

    Value::from_string_vec(context.vm.heap(), result)
}

#[whim_function("Whim\\_Private\\string_rot13(string $string): string")]
pub(crate) fn string_rot13<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let bytes = arguments.bytes(0);
    let result = bytes
        .iter()
        .map(|byte| match *byte {
            value @ (b'A'..=b'M' | b'a'..=b'm') => value + 13,
            value @ (b'N'..=b'Z' | b'n'..=b'z') => value - 13,
            value => value,
        })
        .collect();

    Value::from_string_vec(context.vm.heap(), result)
}

fn byte_mask_table(mask: &[u8]) -> [bool; 256] {
    let mut table = [false; 256];
    let mut index = 0;
    while index < mask.len() {
        let start = mask[index];
        if index + 3 < mask.len() && mask[index + 1] == b'.' && mask[index + 2] == b'.' {
            let end = mask[index + 3];
            let (low, high) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            for byte in low..=high {
                table[usize::from(byte)] = true;
            }

            index += 4;
            continue;
        }

        table[usize::from(start)] = true;
        index += 1;
    }

    table
}

fn byte_value(value: i64) -> u8 {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe { unwrap_result_invariant(u8::try_from(value), "a validated byte value fits u8") }
}

fn string_index(value: i64) -> usize {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            usize::try_from(value),
            "a validated string index fits usize",
        )
    }
}

fn string_position(value: usize) -> i64 {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            i64::try_from(value),
            "a string position fits in a Whim integer",
        )
    }
}

fn search_result(result: Option<usize>, offset: usize) -> Value {
    let Some(position) = result else {
        return Value::null();
    };

    Value::int(string_position(position + offset))
}
