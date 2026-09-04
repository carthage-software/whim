//! Set proofs for composed string-length types.

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::limits::MAX_TYPE_DEPTH;
use crate::optimizer::type_flow::STRING;
use crate::optimizer::type_flow::descriptors::descriptor_mask;

#[derive(Clone, Copy)]
struct Range {
    min: i64,
    max: Option<i64>,
}

#[derive(Clone)]
struct Set {
    ranges: Vec<Range>,
}

impl Set {
    fn empty() -> Self {
        Self { ranges: Vec::new() }
    }

    fn all() -> Self {
        Self::one(0, None)
    }

    fn one(min: i64, max: Option<i64>) -> Self {
        let min = min.max(0);
        if max.is_some_and(|max| max < min) {
            return Self::empty();
        }

        Self {
            ranges: vec![Range { min, max }],
        }
    }

    fn union(mut self, other: Self) -> Self {
        self.ranges.extend(other.ranges);
        self.normalize();
        self
    }

    fn intersection(&self, other: &Self) -> Self {
        let mut intersection = Self::empty();
        for left in &self.ranges {
            for right in &other.ranges {
                let min = left.min.max(right.min);
                let max = minimum_upper_bound(left.max, right.max);
                if max.is_none_or(|max| min <= max) {
                    intersection.ranges.push(Range { min, max });
                }
            }
        }
        intersection.normalize();
        intersection
    }

    fn complement(&self) -> Self {
        let mut complement = Self::empty();
        let mut min = 0;
        for range in &self.ranges {
            if min < range.min {
                complement.ranges.push(Range {
                    min,
                    max: Some(range.min - 1),
                });
            }

            let Some(max) = range.max else {
                return complement;
            };
            let Some(next) = max.checked_add(1) else {
                return complement;
            };
            min = next;
        }
        complement.ranges.push(Range { min, max: None });
        complement
    }

    fn is_subset_of(&self, other: &Self) -> bool {
        self.ranges.iter().all(|actual| {
            other.ranges.iter().any(|expected| {
                expected.min <= actual.min && upper_bound_contains(expected.max, actual.max)
            })
        })
    }

    fn normalize(&mut self) {
        self.ranges.sort_unstable_by_key(|range| range.min);
        let mut normalized: Vec<Range> = Vec::with_capacity(self.ranges.len());
        for range in self.ranges.drain(..) {
            let Some(previous) = normalized.last_mut() else {
                normalized.push(range);
                continue;
            };
            if previous
                .max
                .is_none_or(|max| range.min <= max.saturating_add(1))
            {
                previous.max = maximum_upper_bound(previous.max, range.max);
            } else {
                normalized.push(range);
            }
        }
        self.ranges = normalized;
    }
}

struct Summary {
    possible: Set,
    guaranteed: Set,
}

impl Summary {
    fn empty() -> Self {
        Self {
            possible: Set::empty(),
            guaranteed: Set::empty(),
        }
    }

    fn all() -> Self {
        Self {
            possible: Set::all(),
            guaranteed: Set::all(),
        }
    }

    fn exact(set: Set) -> Self {
        Self {
            possible: set.clone(),
            guaranteed: set,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            possible: self.possible.union(other.possible),
            guaranteed: self.guaranteed.union(other.guaranteed),
        }
    }

    fn intersection(self, other: &Self) -> Self {
        Self {
            possible: self.possible.intersection(&other.possible),
            guaranteed: self.guaranteed.intersection(&other.guaranteed),
        }
    }

    fn complement(self) -> Self {
        Self {
            possible: self.guaranteed.complement(),
            guaranteed: self.possible.complement(),
        }
    }
}

pub(super) fn string_lengths_prove(
    actual: &TypeDescriptor,
    expected: &TypeDescriptor,
    depth: usize,
) -> bool {
    if depth > MAX_TYPE_DEPTH
        || !is_composed(actual) && !is_composed(expected)
        || descriptor_mask(actual) != Some(STRING)
    {
        return false;
    }

    let Some(actual) = summarize(actual, depth + 1) else {
        return false;
    };
    let Some(expected) = summarize(expected, depth + 1) else {
        return false;
    };
    actual.possible.is_subset_of(&expected.guaranteed)
}

fn summarize(descriptor: &TypeDescriptor, depth: usize) -> Option<Summary> {
    if depth > MAX_TYPE_DEPTH {
        return None;
    }

    match descriptor {
        TypeDescriptor::Wildcard | TypeDescriptor::Mixed | TypeDescriptor::String => {
            Some(Summary::all())
        }
        TypeDescriptor::StringLength { min, max } => Some(Summary::exact(Set::one(*min, *max))),
        TypeDescriptor::StringLiteral(value) => {
            let length = i64::try_from(value.as_bytes().len()).ok()?;
            Some(Summary {
                possible: Set::one(length, Some(length)),
                guaranteed: if length == 0 {
                    Set::one(0, Some(0))
                } else {
                    Set::empty()
                },
            })
        }
        TypeDescriptor::Classname(_) => Some(Summary {
            possible: Set::all(),
            guaranteed: Set::empty(),
        }),
        TypeDescriptor::Union(members) => {
            let mut summary = Summary::empty();
            for member in members {
                summary = summary.union(summarize(member, depth + 1)?);
            }
            Some(summary)
        }
        TypeDescriptor::Intersection(members) => {
            let mut summary = Summary::all();
            for member in members {
                summary = summary.intersection(&summarize(member, depth + 1)?);
            }
            Some(summary)
        }
        TypeDescriptor::Negated(excluded) => Some(summarize(excluded, depth + 1)?.complement()),
        TypeDescriptor::Named { .. }
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_) => None,
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::Object
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StaticClass
        | TypeDescriptor::Array(_)
        | TypeDescriptor::Vector(_)
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::Dictionary(_)
        | TypeDescriptor::DictionaryShape { .. }
        | TypeDescriptor::Callable(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::TupleAny => Some(Summary::empty()),
    }
}

fn is_composed(descriptor: &TypeDescriptor) -> bool {
    matches!(
        descriptor,
        TypeDescriptor::Union(_) | TypeDescriptor::Intersection(_) | TypeDescriptor::Negated(_)
    )
}

fn upper_bound_contains(expected: Option<i64>, actual: Option<i64>) -> bool {
    match (expected, actual) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(expected), Some(actual)) => actual <= expected,
    }
}

fn minimum_upper_bound(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (Some(left), Some(right)) => Some(left.min(right)),
    }
}

fn maximum_upper_bound(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (None, _) | (_, None) => None,
        (Some(left), Some(right)) => Some(left.max(right)),
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::chunk::descriptors::TypeDescriptor;
    use crate::optimizer::type_flow::STRING;
    use crate::optimizer::type_flow::descriptors::descriptor_mask;
    use crate::optimizer::type_flow::descriptors::descriptor_proves;

    #[test]
    fn proves_every_composed_string_length_subset() {
        let descriptors = descriptors();
        for (actual_name, actual) in &descriptors {
            if descriptor_mask(actual) != Some(STRING) {
                continue;
            }
            for (expected_name, expected) in &descriptors {
                let subset = (0..=32).all(|length| {
                    !matches_length(actual, length) || matches_length(expected, length)
                });
                assert_eq!(
                    descriptor_proves(actual, expected, None, 0),
                    subset,
                    "{actual_name} against {expected_name}",
                );
            }
        }
    }

    fn descriptors() -> Vec<(&'static str, TypeDescriptor)> {
        let exact_four = length(4, Some(4));
        let except_four = TypeDescriptor::Negated(Box::new(exact_four.clone()));
        vec![
            ("string", TypeDescriptor::String),
            ("empty", length(0, Some(0))),
            ("one", length(1, Some(1))),
            ("four", exact_four),
            ("up to four", length(0, Some(4))),
            ("one or more", length(1, None)),
            ("four or more", length(4, None)),
            ("one through four", length(1, Some(4))),
            ("four through eight", length(4, Some(8))),
            (
                "adjacent union",
                TypeDescriptor::Union(vec![length(0, Some(3)), length(4, Some(8))]),
            ),
            (
                "gapped union",
                TypeDescriptor::Union(vec![length(0, Some(3)), length(5, Some(8))]),
            ),
            (
                "narrowing intersection",
                TypeDescriptor::Intersection(vec![length(1, Some(8)), length(4, Some(12))]),
            ),
            ("not four", except_four.clone()),
            (
                "string except four",
                TypeDescriptor::Intersection(vec![TypeDescriptor::String, except_four]),
            ),
            (
                "complement union",
                TypeDescriptor::Union(vec![length(0, Some(3)), length(5, None)]),
            ),
        ]
    }

    const fn length(min: i64, max: Option<i64>) -> TypeDescriptor {
        TypeDescriptor::StringLength { min, max }
    }

    fn matches_length(descriptor: &TypeDescriptor, length: i64) -> bool {
        match descriptor {
            TypeDescriptor::String => true,
            TypeDescriptor::StringLength { min, max } => {
                length >= *min && max.is_none_or(|max| length <= max)
            }
            TypeDescriptor::Union(members) => {
                members.iter().any(|member| matches_length(member, length))
            }
            TypeDescriptor::Intersection(members) => {
                members.iter().all(|member| matches_length(member, length))
            }
            TypeDescriptor::Negated(excluded) => !matches_length(excluded, length),
            _ => false,
        }
    }
}
