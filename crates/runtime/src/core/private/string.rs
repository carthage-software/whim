//! Byte string functions.

use memchr::memchr as find_byte;
use memchr::memchr_iter as find_byte_positions;
use memchr::memmem::find as find_bytes;
use memchr::memmem::find_iter as find_bytes_positions;
use memchr::memmem::rfind as find_bytes_reverse;
use memchr::memrchr as find_byte_reverse;
use whim_base::unreachable_invariant;
use whim_base::unwrap_option_invariant;
use whim_base::unwrap_result_invariant;
use whim_macros::whim_function;
use whim_value::Value;
use whim_value::ValueView;
use whim_value::string::ByteStringObject;
use whim_value::string::FlatStringSlices;
use whim_value::string::short::ShortString;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::classes::names;

#[whim_function("Whim\\Str\\to_bytes(string $string): vec<0..=255>", must_use)]
pub(crate) fn string_to_bytes<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.bytes(0);
    context.vec(value.iter().map(|byte| Value::int(i64::from(*byte))))
}

#[whim_function("Whim\\Str\\from_bytes(vec<0..=255> $bytes): string", must_use)]
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

    context.owned_string(bytes)
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

#[whim_function("Whim\\Str\\byte_at(string $string, (0..) $offset): 0..=255", must_use)]
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
    "Whim\\_Private\\memchr(string $haystack, 0..=255 $needle, (0..) $offset): null|uint"
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
    "Whim\\_Private\\memrchr(string $haystack, 0..=255 $needle, (0..) $offset): null|uint"
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
    "Whim\\_Private\\memmem(string $haystack, string $needle, (0..) $offset, bool $ci): null|uint"
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
    "Whim\\_Private\\memrmem(string $haystack, string $needle, (0..) $offset, bool $ci): null|uint"
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
    "Whim\\_Private\\string_split(string $string, string $delimiter, uint $limit): vec<string>"
)]
pub(crate) fn string_split<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let haystack = arguments.bytes(0);
    let delimiter = arguments.bytes(1);
    let limit = arguments.uint(2);
    if delimiter.is_empty() || limit == 1 {
        return context.vec([arguments.local(0).with_newtype(None)]);
    }

    if let [byte] = delimiter {
        split_at_positions(
            context,
            arguments,
            haystack,
            1,
            limit,
            find_byte_positions(*byte, haystack),
        )
    } else {
        split_at_positions(
            context,
            arguments,
            haystack,
            delimiter.len(),
            limit,
            find_bytes_positions(haystack, delimiter),
        )
    }
}

fn split_at_positions<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
    haystack: &[u8],
    delimiter_length: usize,
    limit: u64,
    positions: impl Iterator<Item = usize>,
) -> Value {
    let mut positions = positions.peekable();
    if positions.peek().is_none() {
        return context.vec([arguments.local(0).with_newtype(None)]);
    }

    let string = (haystack.len() > ShortString::CAPACITY).then(|| arguments.string(0));
    let source = string.as_ref().map(FlatStringSlices::new);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut parts: Vec<Value> = Vec::new();
    let mut start = 0usize;
    for position in positions {
        if limit != 0 && parts.len() + 1 == limit {
            break;
        }

        parts.push(split_part(
            context,
            source.as_ref(),
            haystack,
            start,
            position,
        ));
        start = position + delimiter_length;
    }

    parts.push(split_part(
        context,
        source.as_ref(),
        haystack,
        start,
        haystack.len(),
    ));
    context.vec(parts)
}

#[expect(
    clippy::inline_always,
    reason = "each split part avoids a wrapper call and an indirect Value return"
)]
#[inline(always)]
fn split_part(
    context: &Context<'_, '_, '_>,
    source: Option<&FlatStringSlices<'_>>,
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Value {
    if end - start <= ShortString::CAPACITY {
        let bytes = source.map_or(bytes, FlatStringSlices::bytes);
        Value::short_string(split_short_part(&bytes[start..end]))
    } else {
        Value::string(source.expect("a long split part has a heap string").slice(
            context.vm.heap(),
            start,
            end - start,
        ))
    }
}

#[inline(never)]
fn split_short_part(bytes: &[u8]) -> ShortString {
    ShortString::from_bytes(bytes).expect("a short split part fits inline storage")
}

#[whim_function("Whim\\_Private\\string_join(vec<string> $values, string $separator): string")]
pub(crate) fn string_join<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    let separator = arguments.bytes(1);
    let capacity = join_capacity(
        values.iter().map(|value| match value.transparent() {
            ValueView::String(string) => string.len(),
            ValueView::ShortString(string) => string.as_bytes().len(),
            // SAFETY: the argument's structural type check validated every element.
            _ => unsafe { unreachable_invariant("a validated string vector contains strings") },
        }),
        separator.len(),
    );
    // An unrepresentable total keeps the incremental allocation failure path.
    let mut result = Vec::with_capacity(capacity.unwrap_or(0));
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

    context.owned_string(result)
}

fn join_capacity(mut lengths: impl Iterator<Item = usize>, separator: usize) -> Option<usize> {
    let first = lengths.next().unwrap_or(0);
    lengths.try_fold(first, |total, length| {
        total.checked_add(separator)?.checked_add(length)
    })
}

#[whim_function(
    "Whim\\_Private\\string_replace(string $haystack, string $needle, string $replacement, bool $ci): string"
)]
pub(crate) fn string_replace<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let needle = arguments.bytes(1);
    let replacement = arguments.bytes(2);
    let ci = arguments.bool(3);
    let haystack = arguments.bytes(0);
    if needle.is_empty() {
        return arguments.local(0).with_newtype(None);
    }

    let mut result = Vec::new();
    let capacity = haystack
        .len()
        .saturating_sub(needle.len())
        .saturating_add(replacement.len());
    let mut start = 0usize;
    let mut append_match = |position| {
        if start == 0 {
            result.reserve(capacity);
        }
        result.extend_from_slice(&haystack[start..position]);
        result.extend_from_slice(replacement);
        start = position + needle.len();
    };
    if ci {
        let folded_haystack = haystack.to_ascii_lowercase();
        let folded_needle = needle.to_ascii_lowercase();
        for position in find_bytes_positions(&folded_haystack, &folded_needle) {
            append_match(position);
        }
    } else if let [byte] = needle {
        for position in find_byte_positions(*byte, haystack) {
            append_match(position);
        }
    } else {
        for position in find_bytes_positions(haystack, needle) {
            append_match(position);
        }
    }

    if start == 0 {
        return arguments.local(0).with_newtype(None);
    }

    result.extend_from_slice(&haystack[start..]);
    context.owned_string(result)
}

#[whim_function("Whim\\Str\\ord(string[1] $character): 0..=255", must_use)]
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

#[whim_function("Whim\\Str\\chr(0..=255 $ascii): string[1]", must_use)]
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
    let bytes = arguments.bytes(0);
    let mask = arguments.bytes(1);
    let mode = arguments.int(2);
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
        return arguments.local(0).with_newtype(None);
    }

    if end - start <= ShortString::CAPACITY {
        return context.string(&bytes[start..end]);
    }

    let string = arguments.string(0);
    Value::string(ByteStringObject::slice(
        context.vm.heap(),
        &string,
        start,
        end - start,
    ))
}

#[whim_function(
    "Whim\\_Private\\string_pad(string $string, uint $length, (string&!'') $pad, 0..=2 $mode): string"
)]
pub(crate) fn string_pad<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let length = arguments.uint(1);
    let pad = arguments.bytes(2);
    let mode = arguments.int(3);
    let length = usize::try_from(length).map_err(|_| {
        let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
        context
            .vm
            .throw(class, "the padded string length is too large", 0)
    })?;
    if length <= bytes.len() {
        return Ok(arguments.local(0).with_newtype(None));
    }

    let needed = length - bytes.len();
    let (left, right) = match mode {
        0 => (needed, 0),
        2 => (0, needed),
        _ => (needed / 2, needed - needed / 2),
    };
    if length <= ShortString::CAPACITY {
        let mut result = [0; ShortString::CAPACITY];
        for (output, byte) in result[..left].iter_mut().zip(pad.iter().cycle()) {
            *output = *byte;
        }
        result[left..left + bytes.len()].copy_from_slice(bytes);
        for (output, byte) in result[left + bytes.len()..length]
            .iter_mut()
            .zip(pad.iter().cycle())
        {
            *output = *byte;
        }
        return Ok(context.string(&result[..length]));
    }

    let mut result = Vec::new();
    result.try_reserve_exact(length).map_err(|_| {
        let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
        context
            .vm
            .throw(class, "the padded string length is too large", 0)
    })?;
    result.extend(pad.iter().cycle().take(left));
    result.extend_from_slice(bytes);
    result.extend(pad.iter().cycle().take(right));
    Ok(context.owned_string(result))
}

#[whim_function("Whim\\Str\\lowercase(string $string): string", must_use)]
pub(crate) fn string_lowercase<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let bytes = arguments.bytes(0);
    if !bytes.iter().any(u8::is_ascii_uppercase) {
        return arguments.local(0).with_newtype(None);
    }

    if bytes.len() <= ShortString::CAPACITY {
        let mut result = [0; ShortString::CAPACITY];
        result[..bytes.len()].copy_from_slice(bytes);
        result[..bytes.len()].make_ascii_lowercase();
        return context.string(&result[..bytes.len()]);
    }

    context.owned_string(bytes.to_ascii_lowercase())
}

#[whim_function("Whim\\Str\\uppercase(string $string): string", must_use)]
pub(crate) fn string_uppercase<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let bytes = arguments.bytes(0);
    if !bytes.iter().any(u8::is_ascii_lowercase) {
        return arguments.local(0).with_newtype(None);
    }

    if bytes.len() <= ShortString::CAPACITY {
        let mut result = [0; ShortString::CAPACITY];
        result[..bytes.len()].copy_from_slice(bytes);
        result[..bytes.len()].make_ascii_uppercase();
        return context.string(&result[..bytes.len()]);
    }

    context.owned_string(bytes.iter().map(u8::to_ascii_uppercase).collect())
}

#[whim_function(
    "Whim\\Str\\capitalize_words(string $string, string $delimiters = \" \\t\\r\\n\\f\\v\"): string",
    must_use
)]
pub(crate) fn string_capitalize_words<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let bytes = arguments.bytes(0);
    let delimiters = arguments
        .get(1)
        .and_then(Value::as_string_bytes)
        .unwrap_or(b" \t\r\n\x0c\x0b");
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

#[whim_function("Whim\\Str\\reverse(string $string): string", must_use)]
pub(crate) fn string_reverse<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let bytes = arguments.bytes(0);
    let result = bytes.iter().rev().copied().collect();

    Value::from_string_vec(context.vm.heap(), result)
}

#[whim_function("Whim\\Str\\rot13(string $string): string", must_use)]
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

const fn search_result(result: Option<usize>, offset: usize) -> Value {
    let Some(position) = result else {
        return Value::null();
    };

    Value::uint((position + offset) as u64)
}

#[cfg(test)]
mod tests {
    use whim_value::Value;
    use whim_value::ValueView;
    use whim_value::newtype::NewtypeValueId;
    use whim_value::object::TypeEnvironmentId;
    use whim_value::string::ByteStringObject;

    use crate::builtin::Context;
    use crate::builtin::arguments::Arguments;
    use crate::engine::Engine;
    use crate::engine::EngineConfiguration;
    use crate::vm::VirtualMachine;

    use super::__whim_direct_handler_string_pad;
    use super::__whim_direct_handler_string_trim;
    use super::join_capacity;
    use super::string_lowercase;
    use super::string_split;
    use super::string_uppercase;

    #[test]
    fn string_split_keeps_storage_limits_binary_bytes_and_tags() {
        let mut engine = Engine::new(EngineConfiguration::default());
        let mut vm = VirtualMachine::new(&mut engine);
        let fields: &[&[u8]] = &[
            b"",
            b"a",
            b"1234567",
            b"12345678",
            b"\xfe",
            b"a longer final field",
            b"",
        ];
        for delimiter in [b"|".as_slice(), b"::", b"\0", b"\xff"] {
            let bytes = fields.join(delimiter);
            for (needle, limit) in [
                (delimiter, 0),
                (delimiter, 1),
                (delimiter, 2),
                (delimiter, u64::MAX),
                (b"".as_slice(), 0),
                (b"\xfd".as_slice(), 0),
            ] {
                let expected = if needle != delimiter || limit == 1 {
                    vec![bytes.as_slice()]
                } else if limit == 2 {
                    vec![b"".as_slice(), &bytes[delimiter.len()..]]
                } else {
                    fields.to_vec()
                };
                for representation in 0..3 {
                    let source = match representation {
                        0 => ByteStringObject::from_bytes(vm.heap(), &bytes),
                        1 => {
                            let middle = bytes.len() / 2;
                            let left = ByteStringObject::from_bytes(vm.heap(), &bytes[..middle]);
                            let right = ByteStringObject::from_bytes(vm.heap(), &bytes[middle..]);
                            ByteStringObject::concat(vm.heap(), &left, &right)
                        }
                        _ => {
                            let base = ByteStringObject::from_bytes(
                                vm.heap(),
                                &[b"prefix".as_slice(), &bytes, b"suffix"].concat(),
                            );
                            ByteStringObject::slice(vm.heap(), &base, 6, bytes.len())
                        }
                    };
                    assert_eq!(source.is_flat(), representation == 0);
                    let tag = NewtypeValueId(0);
                    let values = [
                        Value::string(source).with_newtype(Some(tag)),
                        Value::from_string_bytes(vm.heap(), needle).with_newtype(Some(tag)),
                        Value::uint(limit),
                    ];
                    let arguments = Arguments::new(&values, vm.heap());
                    let context = Context::over_called(&mut vm, None, TypeEnvironmentId::default());
                    let result = string_split(&context, arguments);
                    assert_eq!(values[0].newtype_id(), Some(tag));
                    assert_eq!(values[0].as_string_bytes(), Some(bytes.as_slice()));
                    drop(values);
                    let parts = result.as_vec().unwrap();
                    assert_eq!(parts.len(), expected.len());
                    for (part, expected) in parts.iter().zip(&expected) {
                        assert_eq!(part.as_string_bytes(), Some(*expected));
                        assert_eq!(part.newtype_id(), None);
                    }
                }
            }
        }
    }

    #[test]
    fn string_transforms_keep_short_results_inline() {
        let mut engine = Engine::new(EngineConfiguration::default());
        let mut vm = VirtualMachine::new(&mut engine);
        for bytes in [b"".as_slice(), b"aZ\0\xff", b"abcdefg", b"ABCDEFG"] {
            let input =
                Value::from_string_bytes(vm.heap(), bytes).with_newtype(Some(NewtypeValueId(0)));
            let values = [input];
            let arguments = Arguments::new(&values, vm.heap());
            let context = Context::over_called(&mut vm, None, TypeEnvironmentId::default());
            for result in [
                string_lowercase(&context, arguments),
                string_uppercase(&context, arguments),
            ] {
                assert!(result.as_short_string().is_some());
                assert_eq!(result.newtype_id(), None);
            }

            let values = [
                values[0].clone(),
                Value::from_string_bytes(vm.heap(), b" "),
                Value::int(1),
            ];
            let result = __whim_direct_handler_string_trim(&mut vm, &values).unwrap();
            assert!(result.as_short_string().is_some());
            assert_eq!(result.newtype_id(), None);

            let values = [
                values[0].clone(),
                Value::uint(7),
                Value::from_string_bytes(vm.heap(), b"ab"),
                Value::int(1),
            ];
            let result = __whim_direct_handler_string_pad(&mut vm, &values).unwrap();
            assert!(result.as_short_string().is_some());
            assert_eq!(result.newtype_id(), None);
        }

        let values = [
            Value::from_string_bytes(vm.heap(), b" 1234567 "),
            Value::from_string_bytes(vm.heap(), b" "),
            Value::int(1),
        ];
        let result = __whim_direct_handler_string_trim(&mut vm, &values).unwrap();
        assert!(result.as_short_string().is_some());
        assert_eq!(result.as_string_bytes(), Some(b"1234567".as_slice()));
    }

    #[test]
    fn trimming_long_results_keeps_shared_slices() {
        let mut engine = Engine::new(EngineConfiguration::default());
        let mut vm = VirtualMachine::new(&mut engine);
        let values = [
            Value::from_string_bytes(vm.heap(), b" 12345678 "),
            Value::from_string_bytes(vm.heap(), b" "),
            Value::int(1),
        ];
        let result = __whim_direct_handler_string_trim(&mut vm, &values).unwrap();
        let ValueView::String(string) = result.transparent() else {
            panic!("a long trim result must be a heap string");
        };
        assert!(!string.is_flat());
        drop(values);
        assert_eq!(result.as_string_bytes(), Some(b"12345678".as_slice()));
    }

    #[test]
    fn join_capacity_checks_payload_and_separator_overflow() {
        assert_eq!(join_capacity([].into_iter(), usize::MAX), Some(0));
        assert_eq!(
            join_capacity([usize::MAX].into_iter(), usize::MAX),
            Some(usize::MAX)
        );
        assert_eq!(join_capacity([2, 0, 3].into_iter(), 1), Some(7));
        assert_eq!(
            join_capacity([0, 0].into_iter(), usize::MAX),
            Some(usize::MAX)
        );
        assert_eq!(join_capacity([1, 0].into_iter(), usize::MAX), None);
        assert_eq!(join_capacity([usize::MAX, 1].into_iter(), 0), None);
    }
}
