//! Unstable array primitives exposed to the Whim standard library.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::iter::Enumerate;
use std::mem;
use std::slice;

use hashbrown::HashTable;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::dict::DictIter;
use crate::value::dict::DictObject;
use crate::value::dict::keys::Key;
use crate::value::dict::keys::KeyRef;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::ops;
use crate::value::vec::VecObject;

enum ArrayEntriesInner<'value> {
    Indexed(Enumerate<slice::Iter<'value, Value>>),
    Dict(DictIter<'value>),
}

struct ArrayEntries<'value> {
    inner: ArrayEntriesInner<'value>,
    remaining: usize,
}

impl Iterator for ArrayEntries<'_> {
    type Item = (Value, Value);

    fn next(&mut self) -> Option<Self::Item> {
        let entry = match &mut self.inner {
            ArrayEntriesInner::Indexed(values) => values.next().map(|(index, value)| {
                // SAFETY: the surrounding invariant proves this result is successful.
                let index = unsafe {
                    unwrap_result_invariant(
                        i64::try_from(index),
                        "an array index fits in a Whim integer",
                    )
                };
                (Value::int(index), value.clone())
            }),
            ArrayEntriesInner::Dict(values) => values
                .next()
                .map(|(key, value)| (key.to_value(), value.clone())),
        };
        self.remaining -= usize::from(entry.is_some());
        entry
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for ArrayEntries<'_> {}

fn array_entries(value: &Value) -> Option<ArrayEntries<'_>> {
    match value.transparent() {
        ValueView::Vec(vec) => Some(ArrayEntries {
            inner: ArrayEntriesInner::Indexed(vec.iter().enumerate()),
            remaining: vec.len(),
        }),
        ValueView::Dict(dict) => Some(ArrayEntries {
            inner: ArrayEntriesInner::Dict(dict.iter()),
            remaining: dict.len(),
        }),
        ValueView::Tuple(tuple) => Some(ArrayEntries {
            inner: ArrayEntriesInner::Indexed(tuple.iter().enumerate()),
            remaining: tuple.len(),
        }),
        _ => None,
    }
}

#[whim_function(
    "Whim\\_Private\\array_entries(array<_, _> $values): vec<(string|int|bool, mixed)>"
)]
pub(crate) fn entries<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.local(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let entries = unsafe {
        unwrap_option_invariant(
            array_entries(&value),
            "a validated array argument provides array entries",
        )
    };

    context.vec(entries.map(|entry| context.tuple(<[Value; 2]>::from(entry))))
}

fn ordering_of(
    context: &mut Context<'_, '_, '_>,
    comparator: Option<&Value>,
    left: &Value,
    right: &Value,
) -> Result<i64, Throw> {
    match comparator {
        None => match ops::compare(left, right) {
            Ok(Some(Ordering::Less)) => Ok(-1),
            Ok(Some(Ordering::Equal)) => Ok(0),
            Ok(Some(Ordering::Greater)) => Ok(1),
            Ok(None) | Err(_) => Err(context.type_error("the values cannot be ordered with <=>")),
        },
        Some(callable) => {
            let result = context
                .vm
                .call_function_value(callable, &[left.clone(), right.clone()])?;
            result
                .as_int()
                .ok_or_else(|| context.type_error("a comparator must return an int"))
        }
    }
}

fn vec_element(values: &ManagedRef<VecObject>, index: usize) -> &Value {
    // SAFETY: callers derive the index from this vector's length.
    unsafe { unwrap_option_invariant(values.get(index), "a derived vec index is in bounds") }
}

#[whim_function("Whim\\_Private\\vec_from_array(array<_, _> $values): vec<mixed>")]
pub(crate) fn vec_from_array<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.local(0);
    if let ValueView::Vec(vec) = value.transparent() {
        return Value::vec(vec.clone());
    }

    // SAFETY: the surrounding invariant proves this option contains a value.
    let entries = unsafe {
        unwrap_option_invariant(
            array_entries(&value),
            "a validated array argument provides array entries",
        )
    };

    context.vec(entries.map(|(_, value)| value))
}

#[whim_function(
    "Whim\\_Private\\dict_from_array(array<_, _> $values): dict<string|int|bool, mixed>"
)]
pub(crate) fn dict_from_array<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.local(0);
    if let ValueView::Dict(dict) = value.transparent() {
        return Value::dict(dict.clone());
    }

    // SAFETY: the surrounding invariant proves this option contains a value.
    let entries = unsafe {
        unwrap_option_invariant(
            array_entries(&value),
            "a validated array argument provides array entries",
        )
    };

    context.dict(entries)
}

#[whim_function(
    "Whim\\_Private\\dict_from_entries(array<_, (string|int|bool, _)> $entries): dict<string|int|bool, mixed>"
)]
pub(crate) fn dict_from_entries<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.local(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let entries = unsafe {
        unwrap_option_invariant(
            array_entries(&value),
            "a validated array argument provides array entries",
        )
    };

    let mut result = DictObject::new(context.vm.heap());
    for (_, element) in entries {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let pair = unsafe {
            unwrap_option_invariant(
                element.as_tuple(),
                "validated dictionary entries are tuples",
            )
        };
        // SAFETY: the surrounding invariant proves this result is successful.
        let pair = unsafe {
            unwrap_result_invariant(
                <&[Value; 2]>::try_from(pair.as_slice()),
                "validated dictionary entry tuples contain two elements",
            )
        };
        let [key, entry] = pair;
        // SAFETY: the surrounding invariant proves this option contains a value.
        let key = unsafe {
            unwrap_option_invariant(
                Key::from_value(key),
                "validated dictionary entry keys are array keys",
            )
        };

        result.make_mut().insert(key, entry.clone());
    }

    Value::dict(result)
}

#[whim_function(
    "Whim\\_Private\\dict_merge(dict<_, _> $first, dict<_, _> $second): dict<string|int|bool, mixed>"
)]
pub(crate) fn dict_merge(arguments: Arguments<'_>) -> Value {
    let first = arguments.dict(0);
    let second = arguments.dict(1);
    if second.is_empty() {
        return Value::dict(first);
    }

    let mut result = first;
    for (key, value) in second.iter() {
        result.make_mut().insert(key.to_owned(), value.clone());
    }

    Value::dict(result)
}

#[whim_function(
    "Whim\\_Private\\dict_flip(array<_, string|int|bool> $values): dict<string|int|bool, mixed>"
)]
pub(crate) fn dict_flip<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.local(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let entries = unsafe {
        unwrap_option_invariant(
            array_entries(&value),
            "a validated array argument provides array entries",
        )
    };

    let mut flipped = DictObject::new(context.vm.heap());
    for (key, entry) in entries {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let entry = unsafe {
            unwrap_option_invariant(
                Key::from_owned_value(entry),
                "validated dictionary values are array keys",
            )
        };

        flipped.make_mut().insert(entry, key);
    }

    Value::dict(flipped)
}

#[whim_function(
    "Whim\\_Private\\dict_flatten(array<_, _> $values): null|dict<string|int|bool, mixed>"
)]
pub(crate) fn dict_flatten<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.local(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let groups = unsafe {
        unwrap_option_invariant(
            array_entries(&value),
            "a validated array argument provides array entries",
        )
    };

    let mut result = DictObject::new(context.vm.heap());
    for (_, group) in groups {
        let Some(inner) = array_entries(&group) else {
            return Value::null();
        };

        for (key, value) in inner {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let key = unsafe {
                unwrap_option_invariant(
                    Key::from_owned_value(key),
                    "array iteration produces only array keys",
                )
            };
            result.make_mut().insert(key, value);
        }
    }

    Value::dict(result)
}

#[whim_function(
    "Whim\\_Private\\dict_count_values(array<_, string|int|bool> $values): dict<string|int|bool, int>"
)]
pub(crate) fn dict_count_values<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let value = arguments.local(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let entries = unsafe {
        unwrap_option_invariant(
            array_entries(&value),
            "a validated array argument provides array entries",
        )
    };

    let mut counts = DictObject::new(context.vm.heap());
    for (_, value) in entries {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let key = unsafe {
            unwrap_option_invariant(
                Key::from_value(&value),
                "validated dictionary values are array keys",
            )
        };

        let previous = counts.make_mut().insert(key.clone(), Value::int(1));
        if let Some(count) = previous.as_ref().and_then(Value::as_int) {
            counts.make_mut().insert(key, Value::int(count + 1));
        }
    }

    Value::dict(counts)
}

#[whim_function(
    "Whim\\_Private\\dict_sort(dict<_, _> $values, null|fn(_, _): int $comparator): dict<string|int|bool, mixed>"
)]
pub(crate) fn dict_sort(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let values = arguments.dict(0);
    let comparator = arguments.local(1);
    let comparator = (!comparator.is_null()).then_some(&comparator);
    let mut entries: Vec<(Value, Value)> = values
        .iter()
        .map(|(key, value)| (key.to_value(), value.clone()))
        .collect();
    if entries.len() > 1 {
        let mut error: Option<Throw> = None;
        entries.sort_by(|left, right| {
            if error.is_some() {
                return Ordering::Equal;
            }

            match ordering_of(context, comparator, &left.1, &right.1) {
                Ok(value) => value.cmp(&0),
                Err(thrown) => {
                    error = Some(thrown);
                    Ordering::Equal
                }
            }
        });
        if let Some(thrown) = error {
            return Err(thrown);
        }
    }

    Ok(context.dict(entries))
}

#[whim_function(
    "Whim\\_Private\\vec_sort(vec<_> $values, null|fn(_, _): int $comparator): vec<mixed>"
)]
pub(crate) fn vec_sort(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let values = arguments.vec(0);
    let comparator = arguments.local(1);
    let comparator = (!comparator.is_null()).then_some(&comparator);
    let mut elements: Vec<Value> = values.iter().cloned().collect();
    if elements.len() <= 1 {
        return Ok(Value::vec(values));
    }

    let mut error: Option<Throw> = None;
    elements.sort_by(|left, right| {
        if error.is_some() {
            return Ordering::Equal;
        }

        match ordering_of(context, comparator, left, right) {
            Ok(value) => value.cmp(&0),
            Err(thrown) => {
                error = Some(thrown);
                Ordering::Equal
            }
        }
    });
    if let Some(thrown) = error {
        return Err(thrown);
    }

    Ok(context.vec(elements))
}

#[whim_function(
    "Whim\\_Private\\vec_sort_by(vec<_> $values, vec<_> $keys, null|fn(_, _): int $comparator): vec<mixed>"
)]
pub(crate) fn vec_sort_by(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let values = arguments.vec(0);
    let keys = arguments.vec(1);
    debug_assert_eq!(values.len(), keys.len());
    if values.len() <= 1 {
        return Ok(Value::vec(values));
    }

    let comparator = arguments.local(2);
    let comparator = (!comparator.is_null()).then_some(&comparator);
    let mut order: Vec<_> = (0..values.len()).collect();
    let mut error = None;
    order.sort_by(|left, right| {
        if error.is_some() {
            return Ordering::Equal;
        }
        match ordering_of(
            context,
            comparator,
            vec_element(&keys, *left),
            vec_element(&keys, *right),
        ) {
            Ok(value) => value.cmp(&0),
            Err(thrown) => {
                error = Some(thrown);
                Ordering::Equal
            }
        }
    });
    if let Some(thrown) = error {
        return Err(thrown);
    }

    Ok(context.vec(
        order
            .into_iter()
            .map(|index| vec_element(&values, index).clone_inline_scalar()),
    ))
}

#[whim_function(
    "Whim\\_Private\\vec_slice(vec<_> $values, (0..) $start, null|(0..) $length): vec<mixed>"
)]
pub(crate) fn vec_slice<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    let start = usize::try_from(arguments.int(1)).unwrap_or(usize::MAX);
    if start >= values.len() {
        return context.vec([]);
    }
    let length = arguments.local(2);
    let length = length
        .as_int()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(values.len() - start)
        .min(values.len() - start);
    if start == 0 && length == values.len() {
        return Value::vec(values);
    }

    context.vec(
        values
            .iter()
            .skip(start)
            .take(length)
            .map(Value::clone_inline_scalar),
    )
}

#[whim_function("Whim\\_Private\\vec_chunk(vec<_> $values, (1..) $size): vec<vec<mixed>>")]
pub(crate) fn vec_chunk<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    let size = usize::try_from(arguments.int(1)).unwrap_or(usize::MAX);
    let mut chunks = Vec::with_capacity(values.len().div_ceil(size));
    let mut start = 0;
    while start < values.len() {
        let end = start.saturating_add(size).min(values.len());
        chunks.push(
            context.vec(
                values
                    .iter()
                    .skip(start)
                    .take(end - start)
                    .map(Value::clone_inline_scalar),
            ),
        );
        start = end;
    }

    context.vec(chunks)
}

#[whim_function("Whim\\_Private\\vec_reverse(vec<_> $values): vec<mixed>")]
pub(crate) fn vec_reverse<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    if values.len() <= 1 {
        return Value::vec(values);
    }

    context.vec(values.iter().rev().map(Value::clone_inline_scalar))
}

#[whim_function("Whim\\_Private\\vec_unique(vec<_> $values): vec<mixed>")]
pub(crate) fn vec_unique<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    if values.len() <= 1 {
        return Value::vec(values);
    }

    let unique = unique_values(values.iter(), context.vm.heap());
    context.vec(
        values
            .iter()
            .zip(unique)
            .filter(|(_, unique)| *unique)
            .map(|(value, _)| value.clone_inline_scalar()),
    )
}

#[whim_function("Whim\\_Private\\vec_unique_by(vec<_> $values, vec<_> $keys): vec<mixed>")]
pub(crate) fn vec_unique_by<'call>(
    context: &Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Value {
    let values = arguments.vec(0);
    let keys = arguments.vec(1);
    debug_assert_eq!(values.len(), keys.len());
    if values.len() <= 1 {
        return Value::vec(values);
    }

    let unique = unique_values(keys.iter(), context.vm.heap());
    context.vec(
        values
            .iter()
            .zip(unique)
            .filter(|(_, unique)| *unique)
            .map(|(value, _)| value.clone_inline_scalar()),
    )
}

fn unique_values<'value>(
    values: impl ExactSizeIterator<Item = &'value Value>,
    heap: &Heap,
) -> Vec<bool> {
    let length = values.len();
    let mut result = Vec::with_capacity(length);
    let mut keys = None;
    let mut floats = HashSet::new();
    let mut identities = HashSet::new();
    let mut arrays = HashTable::new();
    let mut has_null = false;

    for value in values {
        let unique = KeyRef::from_value(value).map_or_else(
            || match value.transparent() {
                ValueView::Null => !mem::replace(&mut has_null, true),
                ValueView::Float(float) if float.is_nan() => true,
                ValueView::Float(float) => {
                    let bits = if *float == 0.0 { 0 } else { float.to_bits() };
                    floats.insert(bits)
                }
                ValueView::Function(function) => identities.insert(function.raw_box().addr().get()),
                ValueView::Object(object) => identities.insert(object.raw_box().addr().get()),
                ValueView::Vec(_) | ValueView::Dict(_) | ValueView::Tuple(_) => {
                    remember_unique(&mut arrays, value, heap)
                }
                ValueView::Uninitialized | ValueView::Iter(_) => true,
                ValueView::Bool(_)
                | ValueView::Int(_)
                | ValueView::String(_)
                | ValueView::ShortString(_) => {
                    // SAFETY: these values are accepted by `KeyRef` above.
                    unsafe { crate::unreachable_invariant("an array key produces a borrowed key") }
                }
            },
            |key| {
                let keys = keys.get_or_insert_with(|| {
                    let mut keys = DictObject::new(heap);
                    keys.make_mut().reserve_for_build(length);
                    keys
                });
                if keys.get_ref(key).is_some() {
                    false
                } else {
                    keys.make_mut().insert(key.to_owned(), Value::null());
                    true
                }
            },
        );

        result.push(unique);
    }

    result
}

fn remember_unique<'value>(
    seen: &mut HashTable<(u64, &'value Value)>,
    value: &'value Value,
    heap: &Heap,
) -> bool {
    let hash = ops::structural_hash(value, heap);
    if seen
        .find(hash, |(_, existing)| ops::equals(existing, value))
        .is_some()
    {
        return false;
    }

    seen.insert_unique(hash, (hash, value), |entry| entry.0);
    true
}
