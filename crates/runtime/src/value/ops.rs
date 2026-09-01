//! Language-semantics operations on values: equality, ordering, rendering.

#![expect(
    clippy::float_cmp,
    clippy::unnested_or_patterns,
    reason = "these operations implement Whim's exact numeric semantics and measured hot paths"
)]

use std::cmp::Ordering;
use std::hash::Hasher;
use std::iter::Zip;
use std::slice::Iter;

use crate::value::Value;
use crate::value::ValueView;
use crate::value::dict::DictIter;
use crate::value::dict::DictObject;
use crate::value::dict::keys::KeyRef;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::string::ByteStringObject;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Incomparable;

/// The language's `==`: total, never converting, deep for collections.
#[must_use]
#[inline(never)]
pub(crate) fn equals(a: &Value, b: &Value) -> bool {
    match (a.transparent(), b.transparent()) {
        (ValueView::Uninitialized, _)
        | (_, ValueView::Uninitialized)
        | (ValueView::Iter(_), _)
        | (_, ValueView::Iter(_)) => {
            debug_assert!(false, "whim-runtime: an internal value reached equals");
            return false;
        }
        (ValueView::Null, ValueView::Null) => return true,
        (ValueView::Bool(left), ValueView::Bool(right)) => return left == right,
        (ValueView::Int(left), ValueView::Int(right)) => return left == right,
        (ValueView::Float(left), ValueView::Float(right)) => return left == right,
        (left, right) if left.is_string() && right.is_string() => {
            return left.as_string_bytes() == right.as_string_bytes();
        }
        (ValueView::Function(left), ValueView::Function(right)) => return left.ptr_eq(right),
        (ValueView::Object(left), ValueView::Object(right)) => return left.ptr_eq(right),
        (ValueView::Vec(_), ValueView::Vec(_))
        | (ValueView::Dict(_), ValueView::Dict(_))
        | (ValueView::Tuple(_), ValueView::Tuple(_)) => {}
        _ => return false,
    }

    equals_collections(a, b)
}

#[inline(never)]
fn equals_collections(a: &Value, b: &Value) -> bool {
    let mut cursors = Vec::new();
    let mut pair = Some((a, b));
    loop {
        if let Some((left, right)) = pair.take()
            && !equals_shallow(left, right, &mut cursors)
        {
            return false;
        }

        loop {
            let Some(cursor) = cursors.last_mut() else {
                return true;
            };
            match cursor.next() {
                EqualityStep::Pair(left, right) => {
                    pair = Some((left, right));
                    break;
                }
                EqualityStep::Done => {
                    cursors.pop();
                }
                EqualityStep::Mismatch => return false,
            }
        }
    }
}

const HASH_NULL: u64 = 0x9a5f_7341_00c6_58ad;
const HASH_FLOAT: u64 = 0xf8c2_3401_377c_65eb;
const HASH_FUNCTION: u64 = 0x95f4_73a6_82d2_ba1f;
const HASH_OBJECT: u64 = 0x3f71_4ae9_b272_958d;
const HASH_VEC: u8 = 1;
const HASH_DICT: u8 = 2;
const HASH_TUPLE: u8 = 3;
const HASH_PAIR: u8 = 4;
const HASH_INTERNAL: u64 = 0x2184_fea8_1b49_7d6b;

enum HashTask<'value> {
    Value(&'value Value),
    Key(KeyRef<'value>),
    Sequence { domain: u8, elements: usize },
    Dict { entries: usize },
}

#[must_use]
pub(crate) fn structural_hash(value: &Value, heap: &Heap) -> u64 {
    let state = heap.hash_state();
    let mut tasks = vec![HashTask::Value(value)];
    let mut completed = Vec::new();

    while let Some(task) = tasks.pop() {
        match task {
            HashTask::Value(value) => match value.transparent() {
                ValueView::Null => completed.push(HASH_NULL),
                ValueView::Bool(value) => completed.push(state.hash_bool(*value)),
                ValueView::Int(value) => completed.push(state.hash_int(*value)),
                ValueView::Float(value) => {
                    let bits = if *value == 0.0 { 0 } else { value.to_bits() };
                    completed.push(mix_hash(HASH_FLOAT ^ bits));
                }
                ValueView::String(value) => completed.push(value.hash64(state)),
                ValueView::ShortString(value) => completed.push(value.hash64(state)),
                ValueView::Function(value) => {
                    completed.push(mix_hash(
                        HASH_FUNCTION ^ value.raw_box().addr().get() as u64,
                    ));
                }
                ValueView::Object(value) => {
                    completed.push(mix_hash(HASH_OBJECT ^ value.raw_box().addr().get() as u64));
                }
                ValueView::Vec(value) => {
                    tasks.push(HashTask::Sequence {
                        domain: HASH_VEC,
                        elements: value.len(),
                    });
                    tasks.extend(value.iter().rev().map(HashTask::Value));
                }
                ValueView::Dict(value) => {
                    tasks.push(HashTask::Dict {
                        entries: value.len(),
                    });
                    for (key, value) in value.iter() {
                        tasks.push(HashTask::Value(value));
                        tasks.push(HashTask::Key(key));
                    }
                }
                ValueView::Tuple(value) => {
                    tasks.push(HashTask::Sequence {
                        domain: HASH_TUPLE,
                        elements: value.len(),
                    });
                    tasks.extend(value.iter().rev().map(HashTask::Value));
                }
                ValueView::Uninitialized | ValueView::Iter(_) => completed.push(HASH_INTERNAL),
            },
            HashTask::Key(key) => completed.push(key.hash64(state)),
            HashTask::Sequence { domain, elements } => {
                let start = completed.len() - elements;
                let mut hasher = state.structural_hasher();
                hasher.write_u8(domain);
                hasher.write_usize(elements);
                for hash in &completed[start..] {
                    hasher.write_u64(*hash);
                }
                let hash = hasher.finish();
                completed.truncate(start);
                completed.push(hash);
            }
            HashTask::Dict { entries } => {
                let elements = entries * 2;
                let start = completed.len() - elements;
                let mut sum = 0u64;
                let mut xor = 0u64;
                let (pairs, remainder) = completed[start..].as_chunks::<2>();
                debug_assert!(remainder.is_empty());
                for pair in pairs {
                    let mut hasher = state.structural_hasher();
                    hasher.write_u8(HASH_PAIR);
                    hasher.write_u64(pair[0]);
                    hasher.write_u64(pair[1]);
                    let hash = hasher.finish();
                    sum = sum.wrapping_add(hash);
                    xor ^= hash.rotate_left((hash >> 58) as u32);
                }

                let mut hasher = state.structural_hasher();
                hasher.write_u8(HASH_DICT);
                hasher.write_usize(entries);
                hasher.write_u64(sum);
                hasher.write_u64(xor);
                let hash = hasher.finish();
                completed.truncate(start);
                completed.push(hash);
            }
        }
    }

    completed.pop().unwrap_or(HASH_INTERNAL)
}

const fn mix_hash(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn equals_shallow<'a>(
    left: &'a Value,
    right: &'a Value,
    cursors: &mut Vec<EqualityCursor<'a>>,
) -> bool {
    match (left.transparent(), right.transparent()) {
        (ValueView::Uninitialized, _)
        | (_, ValueView::Uninitialized)
        | (ValueView::Iter(_), _)
        | (_, ValueView::Iter(_)) => {
            debug_assert!(false, "whim-runtime: an internal value reached equals");
            false
        }
        (ValueView::Null, ValueView::Null) => true,
        (ValueView::Bool(left), ValueView::Bool(right)) => left == right,
        (ValueView::Int(left), ValueView::Int(right)) => left == right,
        (ValueView::Float(left), ValueView::Float(right)) => left == right,
        (left, right) if left.is_string() && right.is_string() => {
            left.as_string_bytes() == right.as_string_bytes()
        }
        (ValueView::Vec(left), ValueView::Vec(right)) => {
            if left.ptr_eq(right) {
                return true;
            }
            if left.len() != right.len() {
                return false;
            }
            if !left.is_empty() {
                cursors.push(EqualityCursor::Sequence(left.iter().zip(right.iter())));
            }
            true
        }
        (ValueView::Dict(left), ValueView::Dict(right)) => {
            if left.ptr_eq(right) {
                return true;
            }
            if left.len() != right.len() {
                return false;
            }
            if !left.is_empty() {
                cursors.push(EqualityCursor::Dict {
                    left: left.iter(),
                    right,
                });
            }
            true
        }
        (ValueView::Tuple(left), ValueView::Tuple(right)) => {
            if left.ptr_eq(right) {
                return true;
            }
            if left.len() != right.len() {
                return false;
            }
            if left.len() != 0 {
                cursors.push(EqualityCursor::Sequence(left.iter().zip(right.iter())));
            }
            true
        }
        (ValueView::Function(left), ValueView::Function(right)) => left.ptr_eq(right),
        (ValueView::Object(left), ValueView::Object(right)) => left.ptr_eq(right),
        _ => false,
    }
}

enum EqualityCursor<'a> {
    Sequence(Zip<Iter<'a, Value>, Iter<'a, Value>>),
    Dict {
        left: DictIter<'a>,
        right: &'a DictObject,
    },
}

enum EqualityStep<'a> {
    Pair(&'a Value, &'a Value),
    Done,
    Mismatch,
}

impl<'a> EqualityCursor<'a> {
    fn next(&mut self) -> EqualityStep<'a> {
        match self {
            Self::Sequence(values) => values.next().map_or(EqualityStep::Done, |(left, right)| {
                EqualityStep::Pair(left, right)
            }),
            Self::Dict { left, right } => {
                let Some((key, left)) = left.next() else {
                    return EqualityStep::Done;
                };
                right.get_ref(key).map_or(EqualityStep::Mismatch, |right| {
                    EqualityStep::Pair(left, right)
                })
            }
        }
    }
}

pub(crate) fn compare(a: &Value, b: &Value) -> Result<Option<Ordering>, Incomparable> {
    match (a.transparent(), b.transparent()) {
        (ValueView::Int(left), ValueView::Int(right)) => Ok(Some(left.cmp(right))),
        (ValueView::Float(left), ValueView::Float(right)) => Ok(left.partial_cmp(right)),
        (ValueView::Int(left), ValueView::Float(right)) => Ok(compare_int_float(*left, *right)),
        (ValueView::Float(left), ValueView::Int(right)) => {
            Ok(compare_int_float(*right, *left).map(Ordering::reverse))
        }
        (left, right) if left.is_string() && right.is_string() => Ok(Some(
            // SAFETY: the value's tag proves this projection is valid.
            unsafe { left.as_string_bytes().unwrap_unchecked() }
                // SAFETY: the value's tag proves this projection is valid.
                .cmp(unsafe { right.as_string_bytes().unwrap_unchecked() }),
        )),
        _ => Err(Incomparable),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the preceding bounds checks make float-to-int truncation defined"
)]
#[inline]
pub(crate) fn compare_int_float(integer: i64, float: f64) -> Option<Ordering> {
    const TWO_TO_63: f64 = 9_223_372_036_854_775_808.0;

    if float.is_nan() {
        return None;
    }
    if float >= TWO_TO_63 {
        return Some(Ordering::Less);
    }
    if float < -TWO_TO_63 {
        return Some(Ordering::Greater);
    }

    let truncated = float.trunc() as i64;
    Some(match integer.cmp(&truncated) {
        Ordering::Equal if float.fract() > 0.0 => Ordering::Less,
        Ordering::Equal if float.fract() < 0.0 => Ordering::Greater,
        ordering => ordering,
    })
}

#[must_use]
pub(crate) fn render_int(heap: &Heap, value: i64) -> ManagedRef<ByteStringObject> {
    let mut buffer = itoa::Buffer::new();
    ByteStringObject::from_bytes(heap, buffer.format(value).as_bytes())
}

/// Renders a float in its canonical form on `heap`: the shortest decimal that
/// reads back as the same value, always containing a `.` or an exponent so it
/// is never mistaken for an integer. The special values render as `NAN`,
/// `INF`, and `-INF`.
#[must_use]
pub(crate) fn render_float(heap: &Heap, value: f64) -> ManagedRef<ByteStringObject> {
    if value.is_nan() {
        return ByteStringObject::from_bytes(heap, b"NAN");
    }
    if value.is_infinite() {
        return ByteStringObject::from_bytes(
            heap,
            if value.is_sign_negative() {
                b"-INF"
            } else {
                b"INF"
            },
        );
    }
    let mut buffer = ryu::Buffer::new();
    ByteStringObject::from_bytes(heap, buffer.format(value).as_bytes())
}

#[must_use]
pub(crate) fn stringify_for_concat(
    heap: &Heap,
    value: &Value,
) -> Option<ManagedRef<ByteStringObject>> {
    match value.transparent() {
        ValueView::String(string) => Some(string.clone()),
        ValueView::ShortString(string) => {
            Some(ByteStringObject::from_bytes(heap, string.as_bytes()))
        }
        ValueView::Int(int) => Some(render_int(heap, *int)),
        ValueView::Float(float) => Some(render_float(heap, *float)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use crate::value::ops::compare_int_float;

    #[test]
    fn mixed_numeric_comparison_preserves_large_integer_ordering() {
        assert_eq!(
            compare_int_float(9_007_199_254_740_993, 9_007_199_254_740_992.0),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_int_float(i64::MAX, 9_223_372_036_854_775_808.0),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_int_float(i64::MIN, -9_223_372_036_854_775_808.0),
            Some(Ordering::Equal)
        );
        assert_eq!(compare_int_float(0, f64::NAN), None);
    }
}
