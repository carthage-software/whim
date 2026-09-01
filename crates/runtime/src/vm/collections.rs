//! Indexing, updating, and iterating vecs and dicts.

use crate::bytecode::instruction::operands::CollectionValueMode;
use crate::unwrap_result_invariant;
use crate::value::ValueView;
use crate::value::ops;
use crate::vm::CollectionFault;
use crate::vm::Fault;
use crate::vm::Heap;
use crate::vm::Key;
use crate::vm::KeyRef;
use crate::vm::Value;
use crate::vm::arithmetic_add;
use crate::vm::debug_render;
use crate::vm::integer_add;
use crate::vm::unreachable_invariant;

/// The dict key of a value, following the language's key strictness.
#[inline(always)]
pub(in crate::vm) fn dict_key(value: &Value) -> Result<Key, CollectionFault> {
    match value.transparent() {
        ValueView::Int(key) => Ok(Key::Int(*key)),
        ValueView::Bool(key) => Ok(Key::Bool(*key)),
        ValueView::String(key) => Ok(Key::String(key.clone())),
        ValueView::ShortString(key) => Ok(Key::ShortString(*key)),
        _ => Err(bad_dict_key(value)),
    }
}

#[cold]
#[inline(never)]
fn bad_dict_key(value: &Value) -> CollectionFault {
    CollectionFault::type_error(format!(
        "a dict key must be int, bool, or string, {} given",
        value.kind_name()
    ))
}

#[cold]
#[inline(never)]
fn bad_container(value: &Value) -> CollectionFault {
    CollectionFault::type_error(format!("cannot index into {}", value.kind_name()))
}

#[cold]
#[inline(never)]
fn missing_dict_key(heap: &Heap, index: &Value) -> CollectionFault {
    CollectionFault::out_of_bounds(format!(
        "the dict key {} is not present",
        debug_render(heap, index, 0)
    ))
}

/// `$c[$i]` per the indexing table.
pub(in crate::vm) fn index_get(
    heap: &Heap,
    container: &Value,
    index: &Value,
) -> Result<Value, CollectionFault> {
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
fn dict_key_ref(value: &Value) -> Result<KeyRef<'_>, CollectionFault> {
    match value.transparent() {
        ValueView::Int(key) => Ok(KeyRef::Int(*key)),
        ValueView::Bool(key) => Ok(KeyRef::Bool(*key)),
        ValueView::String(key) => Ok(KeyRef::String(key)),
        ValueView::ShortString(key) => Ok(KeyRef::ShortString(*key)),
        _ => Err(bad_dict_key(value)),
    }
}

#[inline(always)]
fn vec_position(index: &Value, length: usize) -> Result<usize, CollectionFault> {
    match index.transparent() {
        ValueView::Int(position) => int_position(*position, length),
        _ => Err(CollectionFault::type_error(format!(
            "an index must be int, {} given",
            index.kind_name()
        ))),
    }
}

/// Appends to a vec, transparently mutating through a nominal newtype layer.
#[inline(always)]
pub(in crate::vm) fn append_value(
    container: &mut Value,
    value: Value,
) -> Result<(), CollectionFault> {
    match container.as_vec_mut() {
        Some(vec) => {
            vec.make_mut().push(value);
            Ok(())
        }
        None => Err(CollectionFault::type_error(format!(
            "cannot append to {}; `[]=` requires a vec",
            container.kind_name()
        ))),
    }
}

/// The size of any value accepted by `length!()`.
#[inline(always)]
pub(in crate::vm) fn collection_length(value: &Value) -> Result<i64, CollectionFault> {
    match value.transparent() {
        ValueView::String(_) | ValueView::ShortString(_) => {
            // SAFETY: the value's tag proves this projection is valid.
            Ok(unsafe { value.as_string_bytes().unwrap_unchecked().len() as i64 })
        }
        ValueView::Vec(vec) => Ok(vec.len() as i64),
        ValueView::Dict(dict) => Ok(dict.len() as i64),
        ValueView::Tuple(tuple) => Ok(tuple.len() as i64),
        _ => Err(CollectionFault::type_error(format!(
            "length!() accepts a string, vec, dict, or tuple, {} given",
            value.kind_name()
        ))),
    }
}

/// Whether an array contains a value, using the language's deep equality.
#[inline]
pub(in crate::vm) fn array_contains(
    array: &Value,
    needle: &Value,
) -> Result<bool, CollectionFault> {
    match array.transparent() {
        ValueView::Vec(values) => Ok(values.iter().any(|value| ops::equals(value, needle))),
        ValueView::Dict(values) => Ok(values.iter().any(|(_, value)| ops::equals(value, needle))),
        ValueView::Tuple(values) => Ok(values.iter().any(|value| ops::equals(value, needle))),
        other => Err(CollectionFault::type_error(format!(
            "contains!() accepts a vec, dict, or tuple, {} given",
            other.kind_name()
        ))),
    }
}

/// Whether an array contains a key.
#[inline]
pub(in crate::vm) fn array_contains_key(
    array: &Value,
    key: &Value,
) -> Result<bool, CollectionFault> {
    match array.transparent() {
        ValueView::Vec(values) => sequence_contains_key(values.len(), key),
        ValueView::Dict(values) => Ok(values.get_ref(dict_key_ref(key)?).is_some()),
        ValueView::Tuple(values) => sequence_contains_key(values.len(), key),
        other => Err(CollectionFault::type_error(format!(
            "contains_key!() accepts a vec, dict, or tuple, {} given",
            other.kind_name()
        ))),
    }
}

#[inline(always)]
fn sequence_contains_key(length: usize, key: &Value) -> Result<bool, CollectionFault> {
    match key.transparent() {
        ValueView::Int(index) => Ok(usize::try_from(*index).is_ok_and(|index| index < length)),
        other => Err(CollectionFault::type_error(format!(
            "a vec or tuple key must be int, {} given",
            other.kind_name()
        ))),
    }
}

/// The number of values a collection-backed `foreach` can yield, when it is
/// available without invoking user code.
#[inline]
pub(in crate::vm) fn collection_length_hint(value: &Value) -> Option<usize> {
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
pub(in crate::vm) fn reserve_collection_hint(value: &mut Value, additional: usize) {
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
pub(in crate::vm) fn int_position(position: i64, length: usize) -> Result<usize, CollectionFault> {
    if (position as u64) < length as u64 {
        Ok(position as usize)
    } else {
        Err(out_of_bounds_position(position, length))
    }
}

#[cold]
#[inline(never)]
fn out_of_bounds_position(position: i64, length: usize) -> CollectionFault {
    CollectionFault::out_of_bounds(format!(
        "the index {position} is outside the range 0 to {}",
        length as i64 - 1
    ))
}

/// Reads a vec through optimizer-proven container and index types.
pub(in crate::vm) fn vec_index_get(
    container: &Value,
    index: i64,
) -> Result<Value, CollectionFault> {
    let Some(vec) = container.as_vec() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec read has a vec container") }
    };
    let position = int_position(index, vec.len())?;
    // SAFETY: the surrounding invariant keeps this index in bounds.
    Ok(unsafe { vec.get_unchecked(position) }.clone())
}

/// Reads an integer element through optimizer-proven vec and element types.
pub(in crate::vm) fn vec_int_index_get(
    container: &Value,
    index: i64,
) -> Result<i64, CollectionFault> {
    let Some(vec) = container.as_vec() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec read has a vec container") }
    };
    let position = int_position(index, vec.len())?;
    // SAFETY: the value's tag proves this projection is valid.
    Ok(unsafe { vec.get_unchecked(position).as_int_unchecked() })
}

/// Writes a vec through optimizer-proven container and index types.
pub(in crate::vm) fn vec_index_set(
    container: &mut Value,
    index: i64,
    value: Value,
) -> Result<(), CollectionFault> {
    let Some(vec) = container.as_vec_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec write has a vec container") }
    };
    let position = int_position(index, vec.len())?;
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
) -> Result<Value, CollectionFault> {
    let Some(dict) = container.as_dict() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict read has a dict container") }
    };
    dict.get_int(index).cloned().ok_or_else(|| {
        CollectionFault::out_of_bounds(format!("the dict key {index} is not present"))
    })
}

/// Reads an integer value through optimizer-proven dict key and value types.
pub(in crate::vm) fn dict_index_get_int_key_int_value(
    container: &Value,
    index: i64,
) -> Result<i64, CollectionFault> {
    let Some(dict) = container.as_dict() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict read has a dict container") }
    };
    dict.get_int(index)
        .ok_or_else(|| {
            CollectionFault::out_of_bounds(format!("the dict key {index} is not present"))
        })
        // SAFETY: the value's tag proves this projection is valid.
        .map(|value| unsafe { value.as_int_unchecked() })
}

/// Writes a dict through an optimizer-proven integer key.
pub(in crate::vm) fn dict_index_set_int_key(container: &mut Value, index: i64, value: Value) {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict write has a dict container") }
    };
    dict.make_mut().insert_int(index, value);
}

/// Reads a dict through an optimizer-proven string key.
#[inline(always)]
pub(in crate::vm) fn dict_index_get_string_key(
    heap: &Heap,
    container: &Value,
    index: &Value,
    value_mode: CollectionValueMode,
) -> Result<Value, CollectionFault> {
    let Some(dict) = container.as_dict() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict read has a dict container") }
    };
    let found = match index.transparent() {
        ValueView::String(string) => dict.get_string(string),
        ValueView::ShortString(string) => dict.get_short_string(*string),
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("a specialized string-key read has a string index") },
    };
    found
        .map(|value| match value_mode {
            // SAFETY: the value's tag proves this projection is valid.
            CollectionValueMode::Int => Value::int(unsafe { value.as_int_unchecked() }),
            CollectionValueMode::Generic | CollectionValueMode::Float => value.clone(),
        })
        .ok_or_else(|| {
            CollectionFault::out_of_bounds(format!(
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
) -> Result<(), CollectionFault> {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict write has a dict container") }
    };
    let Some(key) = Key::from_owned_value(index) else {
        return Err(CollectionFault::type_error(
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
) -> Result<(), CollectionFault> {
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
        ValueView::Tuple(_) => Err(CollectionFault::type_error(
            "a tuple element cannot be written".to_string(),
        )),
        ValueView::String(_) | ValueView::ShortString(_) => Err(CollectionFault::type_error(
            "a string byte cannot be written".to_string(),
        )),
        _ => Err(bad_container(container)),
    }
}

pub(in crate::vm) enum IndexSetRollback {
    Vector { position: usize, previous: Value },
    Dictionary { key: Key, previous: Option<Value> },
}

pub(in crate::vm) fn index_set_reversible(
    container: &mut Value,
    index: &Value,
    value: Value,
) -> Result<IndexSetRollback, CollectionFault> {
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
    Collection(CollectionFault),
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
        let position = vec_position(index, vec.len()).map_err(IndexAddFault::Collection)?;
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
        let key = dict_key_ref(index).map_err(IndexAddFault::Collection)?;
        let current = dict.make_mut().get_mut_ref(key).ok_or_else(|| {
            IndexAddFault::Collection(CollectionFault::out_of_bounds(format!(
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
        ValueView::Tuple(_) => Err(IndexAddFault::Collection(CollectionFault::type_error(
            "a tuple element cannot be written".to_string(),
        ))),
        ValueView::String(_) | ValueView::ShortString(_) => Err(IndexAddFault::Collection(
            CollectionFault::type_error("a string byte cannot be written".to_string()),
        )),
        _ => Err(IndexAddFault::Collection(CollectionFault::type_error(
            format!("cannot index into {}", container.kind_name()),
        ))),
    }
}

/// Adds a proven integer to a proven integer under a proven string dict key.
pub(in crate::vm) fn dict_add_assign_string_key_int_value(
    heap: &Heap,
    container: &mut Value,
    index: &Value,
    increment: i64,
) -> Result<(), IndexAddFault> {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized indexed add has a dict container") }
    };
    let dictionary = match dict.get_mut() {
        Some(dictionary) => dictionary,
        None => dict.make_mut(),
    };
    let current = match index.transparent() {
        ValueView::String(key) => dictionary.get_int_mut_string(key),
        ValueView::ShortString(key) => dictionary.get_int_mut_short_string(*key),
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("a specialized indexed add has a string key") },
    }
    .ok_or_else(|| {
        IndexAddFault::Collection(CollectionFault::out_of_bounds(format!(
            "the dict key {} is not present",
            debug_render(heap, index, 0)
        )))
    })?;
    *current = integer_add(*current, increment).map_err(|fault| IndexAddFault::Arithmetic {
        fault,
        left_kind: "int",
        right_kind: "int",
    })?;
    Ok(())
}

/// Adds a proven integer to a proven integer under an arbitrary dict key.
pub(in crate::vm) fn dict_add_assign_any_key_int_value(
    heap: &Heap,
    container: &mut Value,
    index: &Value,
    increment: i64,
) -> Result<(), IndexAddFault> {
    let Some(dict) = container.as_dict_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized indexed add has a dict container") }
    };
    let key = match index.transparent() {
        ValueView::Int(key) => KeyRef::Int(*key),
        ValueView::Bool(key) => KeyRef::Bool(*key),
        ValueView::String(key) => KeyRef::String(key),
        ValueView::ShortString(key) => KeyRef::ShortString(*key),
        other => {
            return Err(IndexAddFault::Collection(CollectionFault::type_error(
                format!(
                    "a dict key must be int, bool, or string, {} given",
                    other.kind_name()
                ),
            )));
        }
    };
    let slot = dict.make_mut().get_mut_ref(key).ok_or_else(|| {
        IndexAddFault::Collection(CollectionFault::out_of_bounds(format!(
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

/// Replaces an element already proven to exist and returns its old value.
/// Unlike [`index_set`], a dict key absent from the container is an
/// out-of-bounds error rather than an insertion.
pub(in crate::vm) fn index_replace_existing(
    container: &mut Value,
    index: &Value,
    value: Value,
) -> Result<Value, CollectionFault> {
    if let Some(vec) = container.as_vec_mut() {
        let position = vec_position(index, vec.len())?;
        return vec.make_mut().set(position, value).ok_or_else(|| {
            CollectionFault::out_of_bounds("the vec index is not present".to_string())
        });
    }
    if let Some(dict) = container.as_dict_mut() {
        let key = dict_key(index)?;
        return dict.make_mut().insert(key, value).ok_or_else(|| {
            CollectionFault::out_of_bounds("the dict key is not present".to_string())
        });
    }
    match container.transparent() {
        ValueView::Tuple(_) => Err(CollectionFault::type_error(
            "a tuple element cannot be written".to_string(),
        )),
        ValueView::String(_) | ValueView::ShortString(_) => Err(CollectionFault::type_error(
            "a string byte cannot be written".to_string(),
        )),
        _ => Err(bad_container(container)),
    }
}

/// `vec[...$s]` and `dict[...$s]`: spreads every element of `value` into the
/// literal under construction, following the container's kind.
pub(in crate::vm) fn spread_into(
    container: &mut Value,
    value: &Value,
) -> Result<(), CollectionFault> {
    let value = value.transparent();
    if let Some(vec) = container.as_vec_mut() {
        let elements: &[Value] = match value {
            ValueView::Vec(source) => source.as_slice(),
            ValueView::Tuple(source) => source.as_slice(),
            other => {
                return Err(CollectionFault::type_error(format!(
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
                    target.insert(Key::Int(index as i64), element.clone());
                }
                Ok(())
            }
            ValueView::Tuple(source) => {
                target.reserve_hint(source.len());
                for (index, element) in source.iter().enumerate() {
                    target.insert(Key::Int(index as i64), element.clone());
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
            other => Err(CollectionFault::type_error(format!(
                "a dict literal spreads a vec, a tuple, or a dict, {} given",
                other.kind_name()
            ))),
        }
    } else {
        Err(CollectionFault::type_error(format!(
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
) -> Result<Value, CollectionFault> {
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
    Err(CollectionFault::type_error(format!(
        "remove!() accepts a vec or dict, {} given",
        container.kind_name()
    )))
}

/// `swap_remove!($v, $i)`: removes an element without preserving vec order.
pub(in crate::vm) fn swap_remove_entry(
    container: &mut Value,
    index: &Value,
) -> Result<Value, CollectionFault> {
    if let Some(vec) = container.as_vec_mut() {
        let position = vec_position(index, vec.len())?;
        return match vec.make_mut().swap_remove(position) {
            Some(removed) => Ok(removed),
            // SAFETY: the surrounding invariant makes this path unreachable.
            None => unsafe { unreachable_invariant("the position check bounds the removal") },
        };
    }

    Err(CollectionFault::type_error(format!(
        "swap_remove!() accepts a vec, {} given",
        container.kind_name()
    )))
}

/// `remove_first!`/`remove_last!` over a vec.
pub(in crate::vm) fn remove_end(
    container: &mut Value,
    first: bool,
) -> Result<Value, CollectionFault> {
    if let Some(vec) = container.as_vec_mut() {
        let removed = if first {
            vec.make_mut().remove_first()
        } else {
            vec.make_mut().remove_last()
        };
        return match removed {
            Some(value) => Ok(value),
            None => Err(CollectionFault::out_of_bounds(
                "cannot remove from an empty vec".to_string(),
            )),
        };
    }
    Err(CollectionFault::type_error(format!(
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
        return Some((Value::int(position as i64), value));
    }
    if let Some((tuple, index)) = cursor.as_tuple_cursor_mut() {
        let position = index_value(index);
        if position == tuple.len() {
            return None;
        }
        // SAFETY: the surrounding invariant keeps this index in bounds.
        let value = unsafe { tuple.as_slice().get_unchecked(position) }.clone();
        *index = next_index(position);
        return Some((Value::int(position as i64), value));
    }
    advance_dict_cursor(cursor)
}

/// Advances a cursor whose vec shape was proven by bytecode type flow.
pub(in crate::vm) fn advance_vec_cursor(cursor: &mut Value) -> Option<(Value, Value)> {
    let Some((vec, index)) = cursor.as_vec_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec cursor traverses a vec") }
    };
    let position = index_value(index);
    if position == vec.len() {
        return None;
    }
    // SAFETY: the surrounding invariant keeps this index in bounds.
    let value = unsafe { vec.get_unchecked(position) }.clone();
    *index = next_index(position);
    Some((Value::int(position as i64), value))
}

/// Advances a proven vec cursor whose elements are all integers.
pub(in crate::vm) fn advance_vec_int_cursor(cursor: &mut Value) -> Option<(i64, i64)> {
    let Some((vec, index)) = cursor.as_vec_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized vec cursor traverses a vec") }
    };
    let position = index_value(index);
    if position == vec.len() {
        return None;
    }
    // SAFETY: the value's tag proves this projection is valid.
    let value = unsafe { vec.get_unchecked(position).as_int_unchecked() };
    *index = next_index(position);
    Some((position as i64, value))
}

/// Advances a cursor whose dict shape was proven by bytecode type flow.
pub(in crate::vm) fn advance_dict_cursor(cursor: &mut Value) -> Option<(Value, Value)> {
    let Some((dict, cursor_position)) = cursor.as_dict_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict cursor traverses a dict") }
    };
    let mut position = index_value(cursor_position);
    loop {
        match dict.entry_at_slot(position)? {
            None => position += 1,
            Some((key, value)) => {
                let key = match key {
                    KeyRef::Int(value) => Value::int(value),
                    KeyRef::Bool(value) => Value::bool(value),
                    KeyRef::String(value) => Value::string(value.clone()),
                    KeyRef::ShortString(value) => Value::short_string(value),
                };
                let value = value.clone();
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
) -> Option<(Option<Value>, i64)> {
    let Some((dict, cursor_position)) = cursor.as_dict_cursor_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a specialized dict cursor traverses a dict") }
    };
    let mut position = index_value(cursor_position);
    loop {
        match dict.entry_at_slot(position)? {
            None => position += 1,
            Some((key, value)) => {
                let key = include_key.then(|| match key {
                    KeyRef::Int(value) => Value::int(value),
                    KeyRef::Bool(value) => Value::bool(value),
                    KeyRef::String(value) => Value::string(value.clone()),
                    KeyRef::ShortString(value) => Value::short_string(value),
                });
                // SAFETY: the value's tag proves this projection is valid.
                let value = unsafe { value.as_int_unchecked() };
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
    unsafe {
        unwrap_result_invariant(
            u32::try_from(index + 1),
            "a collection cursor index fits u32",
        )
    }
}
