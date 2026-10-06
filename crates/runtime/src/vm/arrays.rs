//! Indexing, updating, and iterating vecs and dicts.

use std::fmt::Display;

use whim_base::unwrap_option_invariant;
use whim_base::unwrap_result_invariant;
use whim_bytecode::instruction::operands::ArrayValueMode;
use whim_bytecode::instruction::operands::IndexUpdateOperation;
use whim_value::Value;
use whim_value::ValueView;
use whim_value::dict::keys::Key;
use whim_value::dict::keys::KeyRef;
use whim_value::heap::Heap;
use whim_value::ops;

use crate::vm::ArrayFault;
use crate::vm::Fault;
use crate::vm::arithmetic_add;
use crate::vm::arithmetic_divide;
use crate::vm::arithmetic_modulo;
use crate::vm::arithmetic_multiply;
use crate::vm::arithmetic_power;
use crate::vm::arithmetic_subtract;
use crate::vm::bitwise_and;
use crate::vm::bitwise_or;
use crate::vm::bitwise_xor;
use crate::vm::debug_render;
use crate::vm::integer_add;
use crate::vm::shift_left;
use crate::vm::shift_right;
use crate::vm::unreachable_invariant;

#[cfg(test)]
mod path_tests;

/// The dict key of a value, following the language's key strictness.
#[inline(always)]
pub(in crate::vm) fn dict_key(value: &Value) -> Result<Key, ArrayFault> {
    Key::from_value(value).ok_or_else(|| bad_dict_key(value))
}

#[cold]
#[inline(never)]
fn bad_dict_key(value: &Value) -> ArrayFault {
    ArrayFault::type_error(format!(
        "a dict key must be int, uint, bool, string, or fresh, {} given",
        value.kind_name()
    ))
}

#[cold]
#[inline(never)]
fn bad_container(value: &Value) -> ArrayFault {
    ArrayFault::type_error(format!("cannot index into {}", value.kind_name()))
}

#[cold]
#[inline(never)]
fn missing_dict_key(heap: &Heap, index: &Value) -> ArrayFault {
    ArrayFault::out_of_bounds(format!(
        "the dict key {} is not present",
        debug_render(heap, index, 0)
    ))
}

/// `$c[$i]` per the indexing table.
pub(in crate::vm) fn index_get(
    heap: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    match container.transparent() {
        ValueView::Vec(vec) => {
            let position = vec_position(index, vec.len())?;
            match vec.get(position) {
                Some(value) => Ok(value.clone()),
                // SAFETY: the surrounding invariant makes this path unreachable.
                None => unsafe { unreachable_invariant("the position check bounds the index") },
            }
        }
        ValueView::Dict(dict) => {
            let key = dict_key_ref(index)?;
            match dict.get_ref(key) {
                Some(value) => Ok(value.clone()),
                None => Err(missing_dict_key(heap, index)),
            }
        }
        ValueView::Tuple(tuple) => {
            let position = vec_position(index, tuple.len())?;
            match tuple.get(position) {
                Some(value) => Ok(value.clone()),
                // SAFETY: the surrounding invariant makes this path unreachable.
                None => unsafe { unreachable_invariant("the position check bounds the index") },
            }
        }
        ValueView::String(_) | ValueView::ShortString(_) => {
            // SAFETY: the value's tag proves this projection is valid.
            let bytes = unsafe { container.as_string_bytes().unwrap_unchecked() };
            let position = vec_position(index, bytes.len())?;
            // SAFETY: the surrounding invariant keeps this index in bounds.
            let byte = unsafe { *bytes.get_unchecked(position) };
            Ok(Value::string(heap.byte_string(byte)))
        }
        _ => Err(bad_container(container)),
    }
}

#[inline(always)]
fn dict_key_ref(value: &Value) -> Result<KeyRef<'_>, ArrayFault> {
    KeyRef::from_value(value).ok_or_else(|| bad_dict_key(value))
}

#[cold]
#[inline(never)]
pub(in crate::vm) fn dict_index_get_tagged_key(
    heap: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    index_get(heap, container, index)
}

#[cold]
#[inline(never)]
pub(in crate::vm) fn dict_index_get_tagged_key_or_null(container: &Value, index: &Value) -> Value {
    // SAFETY: specialized reads prove both the dictionary and scalar key types.
    let dict = unsafe {
        unwrap_option_invariant(container.as_dict(), "a specialized read has a dictionary")
    };
    // SAFETY: specialized reads prove that the key is int, uint, or string.
    let key = unsafe {
        unwrap_option_invariant(
            KeyRef::from_value(index),
            "a specialized read has a dictionary key",
        )
    };
    dict.get_ref(key)
        .map(Value::clone_inline_scalar)
        .unwrap_or_else(Value::null)
}

#[cold]
#[inline(never)]
pub(in crate::vm) fn dict_index_set_tagged_key(container: &mut Value, index: Value, value: Value) {
    // SAFETY: specialized writes prove both the dictionary and scalar key types.
    let dict = unsafe {
        unwrap_option_invariant(
            container.as_dict_mut(),
            "a specialized write has a dictionary",
        )
    };
    // SAFETY: specialized writes prove that the key is int, uint, or string.
    let key = unsafe {
        unwrap_option_invariant(
            Key::from_owned_value(index),
            "a specialized write has a dictionary key",
        )
    };
    dict.make_mut().insert(key, value);
}

#[inline(always)]
pub(in crate::vm) fn index_get_or_null(
    heap: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    let found = match container.transparent() {
        ValueView::Vec(values) => values
            .get(probe_position(index)?)
            .map(Value::clone_inline_scalar),
        ValueView::Tuple(values) => values
            .get(probe_position(index)?)
            .map(Value::clone_inline_scalar),
        ValueView::Dict(values) => values
            .get_ref(dict_key_ref(index)?)
            .map(Value::clone_inline_scalar),
        ValueView::String(_) | ValueView::ShortString(_) => {
            return string_index_get_or_null(heap, container, index);
        }
        _ => return Err(bad_container(container)),
    };
    Ok(found.unwrap_or_else(Value::null))
}

#[inline(always)]
fn probe_position(index: &Value) -> Result<usize, ArrayFault> {
    match index.transparent() {
        ValueView::Int(position) => Ok(usize::try_from(*position).unwrap_or(usize::MAX)),
        ValueView::Uint(position) => Ok(usize::try_from(*position).unwrap_or(usize::MAX)),
        _ => Err(ArrayFault::type_error(format!(
            "an index must be int or uint, {} given",
            index.kind_name()
        ))),
    }
}

#[inline(always)]
fn proven_probe_position(index: &Value) -> usize {
    // SAFETY: type flow proves the integer index of a specialized vec read.
    usize::try_from(unsafe { index.as_integer_bits_unchecked() }).unwrap_or(usize::MAX)
}

#[inline(always)]
pub(in crate::vm) fn vec_index_get_or_null(
    _: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    let Some(values) = container.as_vec() else {
        // SAFETY: type flow proves the container type for this specialized read.
        unsafe { unreachable_invariant("a specialized probe has its proven container") }
    };
    Ok(values
        .get(proven_probe_position(index))
        .map(Value::clone_inline_scalar)
        .unwrap_or_else(Value::null))
}

#[inline(always)]
pub(in crate::vm) fn dict_index_get_int_key_or_null(
    _: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    let Some(values) = container.as_dict() else {
        // SAFETY: type flow proves the container type for this specialized read.
        unsafe { unreachable_invariant("a specialized probe has its proven container") }
    };
    if index.newtype_id().is_some() {
        return Ok(dict_index_get_tagged_key_or_null(container, index));
    }
    // SAFETY: type flow proves the integer key.
    let key = unsafe { index.as_int_unchecked() };
    Ok(values
        .get_int(key)
        .map(Value::clone_inline_scalar)
        .unwrap_or_else(Value::null))
}

#[inline(always)]
pub(in crate::vm) fn dict_index_get_string_key_or_null(
    _: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    let Some(values) = container.as_dict() else {
        // SAFETY: type flow proves the container type for this specialized read.
        unsafe { unreachable_invariant("a specialized probe has its proven container") }
    };
    if index.newtype_id().is_some() {
        return Ok(dict_index_get_tagged_key_or_null(container, index));
    }
    let found = match index.transparent() {
        ValueView::String(key) => values.get_string(key),
        ValueView::ShortString(key) => values.get_short_string(*key),
        // SAFETY: type flow proves the string key.
        _ => unsafe { unreachable_invariant("a specialized string probe has a string key") },
    };
    Ok(found
        .map(Value::clone_inline_scalar)
        .unwrap_or_else(Value::null))
}

#[inline(always)]
pub(in crate::vm) fn string_index_get_or_null(
    heap: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    let Some(bytes) = container.as_string_bytes() else {
        return Err(bad_container(container));
    };
    Ok(bytes
        .get(probe_position(index)?)
        .map(|byte| Value::string(heap.byte_string(*byte)))
        .unwrap_or_else(Value::null))
}

#[inline(always)]
fn vec_position(index: &Value, length: usize) -> Result<usize, ArrayFault> {
    match index.transparent() {
        ValueView::Int(position) => int_position(*position, length),
        ValueView::Uint(position) => uint_position(*position, length),
        _ => Err(ArrayFault::type_error(format!(
            "an index must be int or uint, {} given",
            index.kind_name()
        ))),
    }
}

/// Appends to a vec, transparently mutating through a nominal newtype layer.
#[inline(always)]
pub(in crate::vm) fn append_value(container: &mut Value, value: Value) -> Result<(), ArrayFault> {
    match container.as_vec_mut() {
        Some(vec) => {
            vec.make_mut().push(value);
            Ok(())
        }
        None => Err(ArrayFault::type_error(format!(
            "cannot append to {}; `[]=` requires a vec",
            container.kind_name()
        ))),
    }
}

/// The size of any value accepted by `length!()`.
#[inline(always)]
pub(in crate::vm) fn array_length(value: &Value) -> Result<u64, ArrayFault> {
    match value.transparent() {
        ValueView::String(_) | ValueView::ShortString(_) => {
            // SAFETY: the value's tag proves this projection is valid.
            Ok(unsafe { value.as_string_len().unwrap_unchecked() as u64 })
        }
        ValueView::Vec(vec) => Ok(vec.len() as u64),
        ValueView::Dict(dict) => Ok(dict.len() as u64),
        ValueView::Tuple(tuple) => Ok(tuple.len() as u64),
        _ => Err(ArrayFault::type_error(format!(
            "length!() accepts a string, vec, dict, or tuple, {} given",
            value.kind_name()
        ))),
    }
}

/// Whether an array contains a value, using the language's deep equality.
#[inline]
pub(in crate::vm) fn array_contains(array: &Value, needle: &Value) -> Result<bool, ArrayFault> {
    match array.transparent() {
        ValueView::Vec(values) => Ok(values.iter().any(|value| ops::equals(value, needle))),
        ValueView::Dict(values) => Ok(values.iter().any(|(_, value)| ops::equals(value, needle))),
        ValueView::Tuple(values) => Ok(values.iter().any(|value| ops::equals(value, needle))),
        other => Err(ArrayFault::type_error(format!(
            "contains!() accepts a vec, dict, or tuple, {} given",
            other.kind_name()
        ))),
    }
}

/// Whether an array contains a key.
#[inline]
pub(in crate::vm) fn array_contains_key(array: &Value, key: &Value) -> Result<bool, ArrayFault> {
    match array.transparent() {
        ValueView::Vec(values) => sequence_contains_key(values.len(), key),
        ValueView::Dict(values) => Ok(values.get_ref(dict_key_ref(key)?).is_some()),
        ValueView::Tuple(values) => sequence_contains_key(values.len(), key),
        other => Err(ArrayFault::type_error(format!(
            "contains_key!() accepts a vec, dict, or tuple, {} given",
            other.kind_name()
        ))),
    }
}

#[inline(always)]
fn sequence_contains_key(length: usize, key: &Value) -> Result<bool, ArrayFault> {
    match key.transparent() {
        ValueView::Int(index) => Ok(usize::try_from(*index).is_ok_and(|index| index < length)),
        ValueView::Uint(index) => Ok(usize::try_from(*index).is_ok_and(|index| index < length)),
        other => Err(ArrayFault::type_error(format!(
            "a vec or tuple key must be int or uint, {} given",
            other.kind_name()
        ))),
    }
}

/// The number of values a collection-backed `foreach` can yield, when it is
/// available without invoking user code.
#[inline]
pub(in crate::vm) fn array_length_hint(value: &Value) -> Option<usize> {
    match value.transparent() {
        ValueView::Vec(value) => Some(value.len()),
        ValueView::Dict(value) => Some(value.len()),
        ValueView::Tuple(value) => Some(value.len()),
        _ => None,
    }
}

/// Reserves capacity in a uniquely owned collection populated by a loop.
/// Shared values keep their ordinary copy-on-write path.
#[inline]
pub(in crate::vm) fn reserve_array_hint(value: &mut Value, additional: usize) {
    if additional == 0 {
        return;
    }
    if let Some(value) = value.as_vec_mut() {
        if let Some(value) = value.get_mut() {
            value.reserve_hint(additional);
        }
    } else if let Some(value) = value.as_dict_mut()
        && let Some(value) = value.get_mut()
    {
        value.reserve_for_build(additional);
    }
}

/// A bounded position from an index already proven to be an integer.
#[inline(always)]
pub(in crate::vm) fn int_position(position: i64, length: usize) -> Result<usize, ArrayFault> {
    if (position as u64) < length as u64 {
        Ok(position as usize)
    } else {
        Err(out_of_bounds_position(position, length))
    }
}

#[inline(always)]
pub(in crate::vm) fn integer_position(index: &Value, length: usize) -> Result<usize, ArrayFault> {
    // SAFETY: type flow proves the index is an int or uint.
    let position = unsafe { index.as_integer_bits_unchecked() };
    if position < length as u64 {
        Ok(position as usize)
    } else {
        Err(out_of_bounds_integer_position(index, position, length))
    }
}

#[cold]
#[inline(never)]
fn out_of_bounds_integer_position(index: &Value, position: u64, length: usize) -> ArrayFault {
    if index.is_int() {
        out_of_bounds_position(position as i64, length)
    } else {
        out_of_bounds_position(position, length)
    }
}

#[inline(always)]
fn uint_position(position: u64, length: usize) -> Result<usize, ArrayFault> {
    if position < length as u64 {
        Ok(position as usize)
    } else {
        Err(out_of_bounds_position(position, length))
    }
}

#[cold]
#[inline(never)]
fn out_of_bounds_position(position: impl Display, length: usize) -> ArrayFault {
    ArrayFault::out_of_bounds(format!(
        "the index {position} is outside the range 0 to {}",
        length as i64 - 1
    ))
}

/// Reads a vec through optimizer-proven container and index types.
pub(in crate::vm) fn vec_index_get(
    container: &Value,
    index: &Value,
    value_mode: ArrayValueMode,
) -> Result<Value, ArrayFault> {
    let Some(vec) = container.as_vec() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec read has a vec container") }
    };
    let position = integer_position(index, vec.len())?;
    // SAFETY: the surrounding invariant keeps this index in bounds.
    Ok(array_value(
        unsafe { vec.get_unchecked(position) },
        value_mode,
    ))
}

/// Reads an integer element through optimizer-proven vec and element types.
pub(in crate::vm) fn vec_int_index_get(
    container: &Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    let Some(vec) = container.as_vec() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec read has a vec container") }
    };
    let position = integer_position(index, vec.len())?;
    // SAFETY: the position check bounds the index.
    Ok(array_value(
        unsafe { vec.get_unchecked(position) },
        ArrayValueMode::Int,
    ))
}

/// Writes a vec through optimizer-proven container and index types.
pub(in crate::vm) fn vec_index_set(
    container: &mut Value,
    index: &Value,
    value: Value,
) -> Result<(), ArrayFault> {
    let Some(vec) = container.as_vec_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec write has a vec container") }
    };
    let position = integer_position(index, vec.len())?;
    if vec.make_mut().set(position, value).is_none() {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("the position check bounds the vec write") }
    }
    Ok(())
}

/// Appends through an optimizer-proven vec container.
pub(in crate::vm) fn vec_append(container: &mut Value, value: Value) {
    let Some(vec) = container.as_vec_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized append has a vec container") }
    };
    vec.make_mut().push(value);
}

/// Reads a dict through an optimizer-proven integer key.
pub(in crate::vm) fn dict_index_get_int_key(
    container: &Value,
    index: i64,
    value_mode: ArrayValueMode,
) -> Result<Value, ArrayFault> {
    let Some(dict) = container.as_dict() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict read has a dict container") }
    };
    dict.get_int(index)
        .map(|value| array_value(value, value_mode))
        .ok_or_else(|| ArrayFault::out_of_bounds(format!("the dict key {index} is not present")))
}

/// Reads an integer value through optimizer-proven dict key and value types.
pub(in crate::vm) fn dict_index_get_int_key_int_value(
    container: &Value,
    index: i64,
) -> Result<Value, ArrayFault> {
    let Some(dict) = container.as_dict() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict read has a dict container") }
    };
    dict.get_int(index)
        .ok_or_else(|| ArrayFault::out_of_bounds(format!("the dict key {index} is not present")))
        .map(|value| array_value(value, ArrayValueMode::Int))
}

/// Writes a dict through an optimizer-proven integer key.
pub(in crate::vm) fn dict_index_set_int_key(container: &mut Value, index: i64, value: Value) {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict write has a dict container") }
    };

    dict.make_mut().insert_int(index, value);
}

#[inline(always)]
fn dict_uint_value(container: &Value, index: u64) -> Option<&Value> {
    let Some(dict) = container.as_dict() else {
        // SAFETY: type flow proves the container is a dict.
        unsafe { unreachable_invariant("a specialized dict read has a dict container") }
    };

    dict.get_ref(KeyRef::Uint(index))
}

#[inline(always)]
pub(in crate::vm) fn dict_index_get_uint_key(
    container: &Value,
    index: u64,
    value_mode: ArrayValueMode,
) -> Result<Value, ArrayFault> {
    dict_uint_value(container, index)
        .map(|value| array_value(value, value_mode))
        .ok_or_else(|| ArrayFault::out_of_bounds(format!("the dict key {index}u is not present")))
}

#[inline(always)]
pub(in crate::vm) fn dict_index_get_uint_key_or_null(container: &Value, index: u64) -> Value {
    dict_uint_value(container, index)
        .map(Value::clone_inline_scalar)
        .unwrap_or_else(Value::null)
}

pub(in crate::vm) fn dict_index_set_uint_key(container: &mut Value, index: u64, value: Value) {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: type flow proves the container is a dict.
        unsafe { unreachable_invariant("a specialized dict write has a dict container") }
    };

    dict.make_mut().insert(Key::Uint(index), value);
}

#[inline(always)]
pub(in crate::vm) fn array_value(value: &Value, value_mode: ArrayValueMode) -> Value {
    match value_mode {
        ArrayValueMode::Int => {
            // SAFETY: type flow proves the array's value type.
            Value::int(unsafe { value.as_int_unchecked() }).with_newtype(value.newtype_id())
        }
        ArrayValueMode::Uint => {
            // SAFETY: type flow proves the array's value type.
            Value::uint(unsafe { value.as_uint_unchecked() }).with_newtype(value.newtype_id())
        }
        ArrayValueMode::Generic | ArrayValueMode::Float => value.clone(),
    }
}

/// Reads a dict through an optimizer-proven string key.
#[inline(always)]
pub(in crate::vm) fn dict_index_get_string_key(
    heap: &Heap,
    container: &Value,
    index: &Value,
    value_mode: ArrayValueMode,
) -> Result<Value, ArrayFault> {
    let Some(dict) = container.as_dict() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict read has a dict container") }
    };

    if index.newtype_id().is_some() {
        return dict_index_get_tagged_key(heap, container, index);
    }
    let found = match index.transparent() {
        ValueView::String(string) => dict.get_string(string),
        ValueView::ShortString(string) => dict.get_short_string(*string),
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("a specialized string-key read has a string index") },
    };

    found
        .map(|value| array_value(value, value_mode))
        .ok_or_else(|| {
            ArrayFault::out_of_bounds(format!(
                "the dict key {} is not present",
                debug_render(heap, index, 0)
            ))
        })
}

/// Writes a dict through an optimizer-proven string key. The index is taken
/// by value so its string handle moves into the key instead of being cloned.
#[inline(always)]
pub(in crate::vm) fn dict_index_set_string_key(container: &mut Value, index: Value, value: Value) {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict write has a dict container") }
    };

    if index.newtype_id().is_some() {
        dict_index_set_tagged_key(container, index, value);
        return;
    }
    if let Some(key) = index.as_short_string() {
        dict.make_mut().insert_short_string(key, value);
        return;
    }

    if !index.is_string() {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized string key has a string value") }
    }

    // SAFETY: the value's tag proves this projection is valid.
    let key = Key::String(unsafe { index.into_string_unchecked() });
    dict.make_mut().insert(key, value);
}

/// Writes through an optimizer-proven dict container while retaining the
/// language's dynamic key validation.
pub(in crate::vm) fn dict_index_set(
    container: &mut Value,
    index: Value,
    value: Value,
) -> Result<(), ArrayFault> {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict write has a dict container") }
    };

    let Some(key) = Key::from_owned_value(index) else {
        return Err(ArrayFault::type_error(
            "a dict key must be int, bool, or string".to_string(),
        ));
    };

    dict.make_mut().insert(key, value);
    Ok(())
}

/// `$c[$i] = $x` per the indexing table, separating shared state first.
pub(in crate::vm) fn index_set(
    container: &mut Value,
    index: &Value,
    value: Value,
) -> Result<(), ArrayFault> {
    if let Some(vec) = container.as_vec_mut() {
        let position = vec_position(index, vec.len())?;
        let replaced = vec.make_mut().set(position, value);
        if replaced.is_none() {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the position check bounds the write") }
        }

        return Ok(());
    }

    if let Some(dict) = container.as_dict_mut() {
        let key = dict_key(index)?;
        dict.make_mut().insert(key, value);
        return Ok(());
    }

    match container.transparent() {
        ValueView::Tuple(_) => Err(ArrayFault::type_error(
            "a tuple element cannot be written".to_string(),
        )),
        ValueView::String(_) | ValueView::ShortString(_) => Err(ArrayFault::type_error(
            "a string byte cannot be written".to_string(),
        )),
        _ => Err(bad_container(container)),
    }
}

pub(in crate::vm) fn index_get_path(
    heap: &Heap,
    mut container: &Value,
    indexes: &[Value],
) -> Result<Value, ArrayFault> {
    // SAFETY: verification rejects empty index paths.
    let (last, parents) = unsafe {
        unwrap_option_invariant(indexes.split_last(), "a verified index path is nonempty")
    };
    for (depth, index) in parents.iter().enumerate() {
        container = match container.transparent() {
            ValueView::Vec(values) => {
                let position = vec_position(index, values.len())?;
                // SAFETY: vec_position checked the bounds.
                unsafe { unwrap_option_invariant(values.get(position), "the index is in bounds") }
            }
            ValueView::Tuple(values) => {
                let position = vec_position(index, values.len())?;
                // SAFETY: vec_position checked the bounds.
                unsafe { unwrap_option_invariant(values.get(position), "the index is in bounds") }
            }
            ValueView::Dict(values) => values
                .get_ref(dict_key_ref(index)?)
                .ok_or_else(|| missing_dict_key(heap, index))?,
            _ => {
                let mut value = index_get(heap, container, index)?;
                for index in &indexes[depth + 1..] {
                    value = index_get(heap, &value, index)?;
                }
                return Ok(value);
            }
        };
    }
    index_get(heap, container, last)
}

pub(in crate::vm) fn index_set_path(
    heap: &Heap,
    mut container: &mut Value,
    indexes: &[Value],
    value: Value,
) -> Result<(), ArrayFault> {
    // SAFETY: verification rejects empty index paths.
    let (last, parents) = unsafe {
        unwrap_option_invariant(indexes.split_last(), "a verified index path is nonempty")
    };

    for (depth, index) in parents.iter().enumerate() {
        if container.is_vec() {
            // SAFETY: is_vec checked the container's tag.
            let vector = unsafe {
                unwrap_option_invariant(container.as_vec_mut(), "the container is a vector")
            };
            let position = vec_position(index, vector.len())?;
            // SAFETY: vec_position checked the bounds, and make_mut preserves the length.
            container = unsafe {
                unwrap_option_invariant(
                    vector.make_mut().get_mut(position),
                    "the index is in bounds",
                )
            };
        } else if container.is_dict() {
            // SAFETY: is_dict checked the container's tag.
            let dictionary = unsafe {
                unwrap_option_invariant(container.as_dict_mut(), "the container is a dictionary")
            };
            let key = dict_key_ref(index)?;
            container = dictionary
                .make_mut()
                .get_mut_ref(key)
                .ok_or_else(|| missing_dict_key(heap, index))?;
        } else {
            return index_set_path_fallback(heap, container, &indexes[depth..], value);
        }
    }

    index_set(container, last, value)
}

#[cold]
fn index_set_path_fallback(
    heap: &Heap,
    container: &mut Value,
    indexes: &[Value],
    mut value: Value,
) -> Result<(), ArrayFault> {
    let mut levels = Vec::with_capacity(indexes.len());
    levels.push(container.clone());
    for index in &indexes[..indexes.len() - 1] {
        // SAFETY: levels starts with the root and only grows here.
        let parent = unsafe { unwrap_option_invariant(levels.last(), "the path has a root") };
        levels.push(index_get(heap, parent, index)?);
    }

    for (mut level, index) in levels.into_iter().zip(indexes).rev() {
        index_set(&mut level, index, value)?;
        value = level;
    }

    *container = value;
    Ok(())
}

pub(in crate::vm) fn index_update_path(
    heap: &Heap,
    mut container: &mut Value,
    indexes: &[Value],
    operand: &Value,
    operation: IndexUpdateOperation,
) -> Result<Value, IndexAddFault> {
    for (depth, index) in indexes.iter().enumerate() {
        if container.is_vec() {
            // SAFETY: is_vec checked the container's tag.
            let vector = unsafe {
                unwrap_option_invariant(container.as_vec_mut(), "the container is a vector")
            };
            let position = vec_position(index, vector.len()).map_err(IndexAddFault::Array)?;
            // SAFETY: vec_position checked the bounds, and make_mut preserves the length.
            container = unsafe {
                unwrap_option_invariant(
                    vector.make_mut().get_mut(position),
                    "the index is in bounds",
                )
            };
        } else if container.is_dict() {
            // SAFETY: is_dict checked the container's tag.
            let dictionary = unsafe {
                unwrap_option_invariant(container.as_dict_mut(), "the container is a dictionary")
            };
            let key = dict_key_ref(index).map_err(IndexAddFault::Array)?;
            container = dictionary
                .make_mut()
                .get_mut_ref(key)
                .ok_or_else(|| IndexAddFault::Array(missing_dict_key(heap, index)))?;
        } else {
            let current =
                index_get_path(heap, container, &indexes[depth..]).map_err(IndexAddFault::Array)?;
            let next = index_update_value(heap, &current, operand, operation)?;
            index_set_path(heap, container, &indexes[depth..], next.clone())
                .map_err(IndexAddFault::Array)?;
            return Ok(next);
        }
    }

    let next = index_update_value(heap, container, operand, operation)?;
    *container = next.clone();
    Ok(next)
}

fn index_update_value(
    heap: &Heap,
    current: &Value,
    operand: &Value,
    operation: IndexUpdateOperation,
) -> Result<Value, IndexAddFault> {
    match operation {
        IndexUpdateOperation::Add => arithmetic_add(heap, current, operand),
        IndexUpdateOperation::Subtract => arithmetic_subtract(heap, current, operand),
        IndexUpdateOperation::Multiply => arithmetic_multiply(heap, current, operand),
        IndexUpdateOperation::Divide => arithmetic_divide(heap, current, operand),
        IndexUpdateOperation::Modulo => arithmetic_modulo(heap, current, operand),
        IndexUpdateOperation::Power => arithmetic_power(heap, current, operand),
        IndexUpdateOperation::BitwiseAnd => bitwise_and(heap, current, operand),
        IndexUpdateOperation::BitwiseOr => bitwise_or(heap, current, operand),
        IndexUpdateOperation::BitwiseXor => bitwise_xor(heap, current, operand),
        IndexUpdateOperation::ShiftLeft => shift_left(heap, current, operand),
        IndexUpdateOperation::ShiftRight => shift_right(heap, current, operand),
    }
    .map_err(|fault| IndexAddFault::Arithmetic {
        fault,
        left_kind: current.kind_name(),
        right_kind: operand.kind_name(),
    })
}

pub(in crate::vm) enum IndexSetRollback {
    Vector { position: usize, previous: Value },
    Dictionary { key: Key, previous: Option<Value> },
}

pub(in crate::vm) fn index_set_reversible(
    container: &mut Value,
    index: &Value,
    value: Value,
) -> Result<IndexSetRollback, ArrayFault> {
    if container.is_vec() {
        let Some(vector) = container.as_vec() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a vec value exposes its vec storage") }
        };

        let position = vec_position(index, vector.len())?;
        let Some(vector) = container.as_vec_mut() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a vec value exposes mutable vec storage") }
        };

        let Some(previous) = vector.make_mut().set(position, value) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the position check bounds the vec write") }
        };

        return Ok(IndexSetRollback::Vector { position, previous });
    }

    if container.is_dict() {
        let key = dict_key(index)?;
        let Some(dictionary) = container.as_dict_mut() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a dict value exposes mutable dict storage") }
        };

        let previous = dictionary.make_mut().insert(key.clone(), value);

        return Ok(IndexSetRollback::Dictionary { key, previous });
    }

    Err(bad_container(container))
}

pub(in crate::vm) fn rollback_index_set(container: &mut Value, rollback: IndexSetRollback) {
    match rollback {
        IndexSetRollback::Vector { position, previous } => {
            let Some(vector) = container.as_vec_mut() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("an indexed rollback retains its vec container") }
            };

            let Some(rejected) = vector.make_mut().set(position, previous) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("an indexed rollback replaces its vec element") }
            };

            drop(rejected);
        }
        IndexSetRollback::Dictionary { key, previous } => {
            let Some(dictionary) = container.as_dict_mut() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("an indexed rollback retains its dict container") }
            };

            if let Some(previous) = previous {
                let Some(rejected) = dictionary.make_mut().insert(key, previous) else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("an indexed rollback replaces its dict entry") }
                };

                drop(rejected);
            } else {
                let Some(rejected) = dictionary.make_mut().remove(&key) else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe {
                        unreachable_invariant("an indexed rollback removes its new dict entry")
                    }
                };

                drop(rejected);
            }
        }
    }
}

/// A fault from an indexed `+=`: either locating the element or adding its
/// current value failed.
pub(in crate::vm) enum IndexAddFault {
    Array(ArrayFault),
    Arithmetic {
        fault: Fault,
        left_kind: &'static str,
        right_kind: &'static str,
    },
}

/// Adds `increment` to an existing indexed element with one lookup.
pub(in crate::vm) fn index_add_assign(
    heap: &Heap,
    container: &mut Value,
    index: &Value,
    increment: &Value,
) -> Result<(), IndexAddFault> {
    if let Some(vec) = container.as_vec_mut() {
        let position = vec_position(index, vec.len()).map_err(IndexAddFault::Array)?;
        let values = vec.make_mut();
        // SAFETY: the surrounding invariant keeps this index in bounds.
        let current = unsafe { values.get_unchecked(position) };
        let next = arithmetic_add(heap, current, increment).map_err(|fault| {
            IndexAddFault::Arithmetic {
                fault,
                left_kind: current.kind_name(),
                right_kind: increment.kind_name(),
            }
        })?;

        if values.set(position, next).is_none() {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the position check bounds the vec write") }
        }

        return Ok(());
    }

    if let Some(dict) = container.as_dict_mut() {
        let key = dict_key_ref(index).map_err(IndexAddFault::Array)?;
        let current = dict.make_mut().get_mut_ref(key).ok_or_else(|| {
            IndexAddFault::Array(ArrayFault::out_of_bounds(format!(
                "the dict key {} is not present",
                debug_render(heap, index, 0)
            )))
        })?;

        let next = arithmetic_add(heap, current, increment).map_err(|fault| {
            IndexAddFault::Arithmetic {
                fault,
                left_kind: current.kind_name(),
                right_kind: increment.kind_name(),
            }
        })?;

        *current = next;
        return Ok(());
    }

    match container.transparent() {
        ValueView::Tuple(_) => Err(IndexAddFault::Array(ArrayFault::type_error(
            "a tuple element cannot be written".to_string(),
        ))),
        ValueView::String(_) | ValueView::ShortString(_) => Err(IndexAddFault::Array(
            ArrayFault::type_error("a string byte cannot be written".to_string()),
        )),
        _ => Err(IndexAddFault::Array(ArrayFault::type_error(format!(
            "cannot index into {}",
            container.kind_name()
        )))),
    }
}

/// Adds a proven integer to a proven integer under a proven string dict key.
pub(in crate::vm) fn dict_add_assign_string_key_int_value(
    heap: &Heap,
    container: &mut Value,
    index: &Value,
    increment: i64,
) -> Result<(), IndexAddFault> {
    dict_add_assign_any_key_int_value(heap, container, index, increment)
}

/// Adds a proven integer to a proven integer under an arbitrary dict key.
pub(in crate::vm) fn dict_add_assign_any_key_int_value(
    heap: &Heap,
    container: &mut Value,
    index: &Value,
    increment: i64,
) -> Result<(), IndexAddFault> {
    if index.newtype_id().is_some() {
        return dict_add_assign_tagged_key_int_value(heap, container, index, increment);
    }
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized indexed add has a dict container") }
    };

    let key = match index.transparent() {
        ValueView::Int(key) => KeyRef::Int(*key),
        ValueView::Uint(key) => KeyRef::Uint(*key),
        ValueView::Fresh(key) => KeyRef::Fresh(*key),
        ValueView::Bool(key) => KeyRef::Bool(*key),
        ValueView::String(key) => KeyRef::String(key),
        ValueView::ShortString(key) => KeyRef::ShortString(*key),
        _ => return Err(IndexAddFault::Array(bad_dict_key(index))),
    };

    let slot = dict.make_mut().get_mut_ref(key).ok_or_else(|| {
        IndexAddFault::Array(ArrayFault::out_of_bounds(format!(
            "the dict key {} is not present",
            debug_render(heap, index, 0)
        )))
    })?;

    // SAFETY: the value's tag proves this projection is valid.
    let current = unsafe { slot.as_int_unchecked() };
    let next = integer_add(current, increment).map_err(|fault| IndexAddFault::Arithmetic {
        fault,
        left_kind: "int",
        right_kind: "int",
    })?;

    *slot = Value::int(next);
    Ok(())
}

#[cold]
#[inline(never)]
fn dict_add_assign_tagged_key_int_value(
    heap: &Heap,
    container: &mut Value,
    index: &Value,
    increment: i64,
) -> Result<(), IndexAddFault> {
    index_add_assign(heap, container, index, &Value::int(increment))
}

/// Replaces an element already proven to exist and returns its old value.
/// Unlike [`index_set`], a dict key absent from the container is an
/// out-of-bounds error rather than an insertion.
pub(in crate::vm) fn index_replace_existing(
    container: &mut Value,
    index: &Value,
    value: Value,
) -> Result<Value, ArrayFault> {
    if let Some(vec) = container.as_vec_mut() {
        let position = vec_position(index, vec.len())?;
        return vec
            .make_mut()
            .set(position, value)
            .ok_or_else(|| ArrayFault::out_of_bounds("the vec index is not present".to_string()));
    }

    if let Some(dict) = container.as_dict_mut() {
        let key = dict_key(index)?;
        return dict
            .make_mut()
            .insert(key, value)
            .ok_or_else(|| ArrayFault::out_of_bounds("the dict key is not present".to_string()));
    }

    match container.transparent() {
        ValueView::Tuple(_) => Err(ArrayFault::type_error(
            "a tuple element cannot be written".to_string(),
        )),
        ValueView::String(_) | ValueView::ShortString(_) => Err(ArrayFault::type_error(
            "a string byte cannot be written".to_string(),
        )),
        _ => Err(bad_container(container)),
    }
}

/// `vec[...$s]` and `dict[...$s]`: spreads every element of `value` into the
/// literal under construction, following the container's kind.
pub(in crate::vm) fn spread_into(container: &mut Value, value: &Value) -> Result<(), ArrayFault> {
    let value = value.transparent();
    if let Some(vec) = container.as_vec_mut() {
        let elements: &[Value] = match value {
            ValueView::Vec(source) => source.as_slice(),
            ValueView::Tuple(source) => source.as_slice(),
            other => {
                return Err(ArrayFault::type_error(format!(
                    "a vec literal spreads a vec or a tuple, {} given",
                    other.kind_name()
                )));
            }
        };

        let target = vec.make_mut();
        target.reserve_hint(elements.len());
        for element in elements {
            target.push(element.clone());
        }

        return Ok(());
    }

    if let Some(dict) = container.as_dict_mut() {
        let target = dict.make_mut();
        match value {
            ValueView::Vec(source) => {
                target.reserve_hint(source.len());
                for (index, element) in source.iter().enumerate() {
                    target.insert(Key::Uint(index as u64), element.clone());
                }

                Ok(())
            }
            ValueView::Tuple(source) => {
                target.reserve_hint(source.len());
                for (index, element) in source.iter().enumerate() {
                    target.insert(Key::Uint(index as u64), element.clone());
                }

                Ok(())
            }
            ValueView::Dict(source) => {
                target.reserve_hint(source.len());
                for (key, element) in source.iter() {
                    target.insert(key.to_owned(), element.clone());
                }

                Ok(())
            }
            other => Err(ArrayFault::type_error(format!(
                "a dict literal spreads a vec, a tuple, or a dict, {} given",
                other.kind_name()
            ))),
        }
    } else {
        Err(ArrayFault::type_error(format!(
            "cannot spread into {}",
            container.kind_name()
        )))
    }
}

/// `remove!($c, $k)`: a vec index removal shifts later elements down; a dict
/// key removal yields the removed value.
pub(in crate::vm) fn remove_entry(
    heap: &Heap,
    container: &mut Value,
    key: &Value,
) -> Result<Value, ArrayFault> {
    if let Some(vec) = container.as_vec_mut() {
        let position = vec_position(key, vec.len())?;
        return match vec.make_mut().remove(position) {
            Some(removed) => Ok(removed),
            // SAFETY: the surrounding invariant makes this path unreachable.
            None => unsafe { unreachable_invariant("the position check bounds the removal") },
        };
    }
    if let Some(dict) = container.as_dict_mut() {
        let dictionary_key = dict_key(key)?;
        return match dict.make_mut().remove(&dictionary_key) {
            Some(removed) => Ok(removed),
            None => Err(missing_dict_key(heap, key)),
        };
    }
    Err(ArrayFault::type_error(format!(
        "remove!() accepts a vec or dict, {} given",
        container.kind_name()
    )))
}

/// `swap_remove!($v, $i)`: removes an element without preserving vec order.
pub(in crate::vm) fn swap_remove_entry(
    container: &mut Value,
    index: &Value,
) -> Result<Value, ArrayFault> {
    if let Some(vec) = container.as_vec_mut() {
        let position = vec_position(index, vec.len())?;
        return match vec.make_mut().swap_remove(position) {
            Some(removed) => Ok(removed),
            // SAFETY: the surrounding invariant makes this path unreachable.
            None => unsafe { unreachable_invariant("the position check bounds the removal") },
        };
    }

    Err(ArrayFault::type_error(format!(
        "swap_remove!() accepts a vec, {} given",
        container.kind_name()
    )))
}

/// `remove_first!`/`remove_last!` over a vec.
pub(in crate::vm) fn remove_end(container: &mut Value, first: bool) -> Result<Value, ArrayFault> {
    if let Some(vec) = container.as_vec_mut() {
        let removed = if first {
            vec.make_mut().remove_first()
        } else {
            vec.make_mut().remove_last()
        };

        return match removed {
            Some(value) => Ok(value),
            None => Err(ArrayFault::out_of_bounds(
                "cannot remove from an empty vec".to_string(),
            )),
        };
    }

    Err(ArrayFault::type_error(format!(
        "remove_first!() and remove_last!() accept a vec, {} given",
        container.kind_name()
    )))
}

/// Advances a `foreach` cursor, yielding the next key and value.
pub(in crate::vm) fn advance_cursor(cursor: &mut Value) -> Option<(Value, Value)> {
    if let Some((vec, index)) = cursor.as_vec_cursor_mut() {
        let position = index_value(index);
        if position == vec.len() {
            return None;
        }

        // SAFETY: the surrounding invariant keeps this index in bounds.
        let value = unsafe { vec.get_unchecked(position) }.clone();
        *index = next_index(position);
        return Some((Value::uint(position as u64), value));
    }

    if let Some((tuple, index)) = cursor.as_tuple_cursor_mut() {
        let position = index_value(index);
        if position == tuple.len() {
            return None;
        }

        // SAFETY: the surrounding invariant keeps this index in bounds.
        let value = unsafe { tuple.as_slice().get_unchecked(position) }.clone();
        *index = next_index(position);
        return Some((Value::uint(position as u64), value));
    }

    advance_dict_cursor(cursor, ArrayValueMode::Generic)
}

/// Advances a cursor whose vec shape was proven by bytecode type flow.
pub(in crate::vm) fn advance_vec_cursor(
    cursor: &mut Value,
    value_mode: ArrayValueMode,
) -> Option<(Value, Value)> {
    let Some((vec, index)) = cursor.as_vec_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec cursor traverses a vec") }
    };

    let position = index_value(index);
    if position == vec.len() {
        return None;
    }

    // SAFETY: the surrounding invariant keeps this index in bounds.
    let value = array_value(unsafe { vec.get_unchecked(position) }, value_mode);
    *index = next_index(position);
    Some((Value::uint(position as u64), value))
}

/// Advances a proven vec cursor whose elements are all integers.
pub(in crate::vm) fn advance_vec_int_cursor(cursor: &mut Value) -> Option<(u64, Value)> {
    let Some((vec, index)) = cursor.as_vec_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec cursor traverses a vec") }
    };

    let position = index_value(index);
    if position == vec.len() {
        return None;
    }

    // SAFETY: the cursor position is below the vec length.
    let value = array_value(unsafe { vec.get_unchecked(position) }, ArrayValueMode::Int);
    *index = next_index(position);
    Some((position as u64, value))
}

/// Advances a cursor whose dict shape was proven by bytecode type flow.
pub(in crate::vm) fn advance_dict_cursor(
    cursor: &mut Value,
    value_mode: ArrayValueMode,
) -> Option<(Value, Value)> {
    let Some((dict, cursor_position)) = cursor.as_dict_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict cursor traverses a dict") }
    };

    let mut position = index_value(cursor_position);
    loop {
        match dict.entry_at_slot(position)? {
            None => position += 1,
            Some((key, value)) => {
                let key = key.to_value();
                let value = array_value(value, value_mode);
                *cursor_position = next_index(position);
                return Some((key, value));
            }
        }
    }
}

/// Advances a proven dict cursor whose values are all integers.
pub(in crate::vm) fn advance_dict_cursor_int_values(
    cursor: &mut Value,
    include_key: bool,
) -> Option<(Option<Value>, Value)> {
    let Some((dict, cursor_position)) = cursor.as_dict_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict cursor traverses a dict") }
    };

    let mut position = index_value(cursor_position);
    loop {
        match dict.entry_at_slot(position)? {
            None => position += 1,
            Some((key, value)) => {
                let key = include_key.then(|| key.to_value());
                let value = array_value(value, ArrayValueMode::Int);
                *cursor_position = next_index(position);
                return Some((key, value));
            }
        }
    }
}

#[inline(always)]
fn index_value(index: &u32) -> usize {
    *index as usize
}

#[inline(always)]
fn next_index(index: usize) -> u32 {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe { unwrap_result_invariant(u32::try_from(index + 1), "an array cursor index fits u32") }
}
