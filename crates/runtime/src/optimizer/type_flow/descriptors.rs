//! Descriptor-level subtyping, equality, and substitution helpers.

use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::string_length_matches;
use crate::limits::MAX_TYPE_DEPTH;
use crate::optimizer::type_flow::ALL;
use crate::optimizer::type_flow::BOOL;
use crate::optimizer::type_flow::CALLABLE;
use crate::optimizer::type_flow::CompiledTypeParameter;
use crate::optimizer::type_flow::DICTIONARY;
use crate::optimizer::type_flow::FLOAT;
use crate::optimizer::type_flow::INT;
use crate::optimizer::type_flow::IndexedUnit;
use crate::optimizer::type_flow::Literal;
use crate::optimizer::type_flow::NULL;
use crate::optimizer::type_flow::OBJECT;
use crate::optimizer::type_flow::STRING;
use crate::optimizer::type_flow::TUPLE;
use crate::optimizer::type_flow::TypeDescriptor;
use crate::optimizer::type_flow::VECTOR;
use crate::optimizer::type_flow::same_atom;

pub(in crate::optimizer) fn substitute_parameters(
    descriptor: &TypeDescriptor,
    parameters: &[CompiledTypeParameter],
    arguments: Option<&[TypeDescriptor]>,
    depth: usize,
) -> TypeDescriptor {
    if depth > MAX_TYPE_DEPTH {
        return descriptor.clone();
    }
    match descriptor {
        TypeDescriptor::Parameter(name) => arguments
            .and_then(|arguments| {
                parameters
                    .iter()
                    .position(|parameter| same_atom(&parameter.name, name))
                    .and_then(|position| arguments.get(position))
            })
            .cloned()
            .unwrap_or_else(|| descriptor.clone()),
        _ => descriptor
            .map_children(|child| substitute_parameters(child, parameters, arguments, depth + 1)),
    }
}

pub(in crate::optimizer) fn descriptor_mask(descriptor: &TypeDescriptor) -> Option<u16> {
    match descriptor {
        TypeDescriptor::Wildcard | TypeDescriptor::Mixed | TypeDescriptor::Negated(_) => Some(ALL),
        TypeDescriptor::Void | TypeDescriptor::Null => Some(NULL),
        TypeDescriptor::Never => Some(0),
        TypeDescriptor::Bool | TypeDescriptor::TrueLiteral | TypeDescriptor::FalseLiteral => {
            Some(BOOL)
        }
        TypeDescriptor::Int | TypeDescriptor::IntLiteral(_) | TypeDescriptor::IntRange { .. } => {
            Some(INT)
        }
        TypeDescriptor::Float | TypeDescriptor::FloatLiteral(_) => Some(FLOAT),
        TypeDescriptor::String
        | TypeDescriptor::StringLength { .. }
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Classname(_) => Some(STRING),
        TypeDescriptor::Object | TypeDescriptor::StaticClass => Some(OBJECT),
        TypeDescriptor::Named { .. }
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_) => None,
        TypeDescriptor::Array(_) => Some(VECTOR | DICTIONARY | TUPLE),
        TypeDescriptor::Vector(_) | TypeDescriptor::VectorShape { .. } => Some(VECTOR),
        TypeDescriptor::Dictionary(_) | TypeDescriptor::DictionaryShape { .. } => Some(DICTIONARY),
        TypeDescriptor::Callable(_) => Some(CALLABLE),
        TypeDescriptor::Tuple(_) | TypeDescriptor::TupleRest { .. } | TypeDescriptor::TupleAny => {
            Some(TUPLE)
        }
        TypeDescriptor::Union(members) => {
            let mut mask = 0;
            for member in members {
                mask |= descriptor_mask(member)?;
            }
            Some(mask)
        }
        TypeDescriptor::Intersection(members) => {
            let mut members = members.iter();
            let mut mask = descriptor_mask(members.next()?)?;
            for member in members {
                mask &= descriptor_mask(member)?;
            }
            Some(mask)
        }
    }
}

pub(in crate::optimizer::type_flow) fn exact_descriptor_mask(
    descriptor: &TypeDescriptor,
) -> Option<u16> {
    match descriptor {
        TypeDescriptor::Wildcard | TypeDescriptor::Mixed => Some(ALL),
        TypeDescriptor::Void | TypeDescriptor::Null => Some(NULL),
        TypeDescriptor::Never => Some(0),
        TypeDescriptor::Bool => Some(BOOL),
        TypeDescriptor::Int => Some(INT),
        TypeDescriptor::Float => Some(FLOAT),
        TypeDescriptor::String => Some(STRING),
        TypeDescriptor::Object => Some(OBJECT),
        TypeDescriptor::Array(None) => Some(VECTOR | DICTIONARY | TUPLE),
        TypeDescriptor::Vector(None) => Some(VECTOR),
        TypeDescriptor::Dictionary(None) => Some(DICTIONARY),
        TypeDescriptor::Callable(None) => Some(CALLABLE),
        TypeDescriptor::TupleAny => Some(TUPLE),
        TypeDescriptor::Union(members) => {
            let mut mask = 0;
            for member in members {
                mask |= exact_descriptor_mask(member)?;
            }
            Some(mask)
        }
        _ => None,
    }
}

/// Whether releasing a value satisfying `descriptor` may invoke user code or
/// make an object weak reference observe an earlier death.
pub(in crate::optimizer::type_flow) fn descriptor_may_release_observably(
    descriptor: &TypeDescriptor,
) -> bool {
    match descriptor {
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Object
        | TypeDescriptor::Named { .. }
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::StaticClass
        | TypeDescriptor::Array(None)
        | TypeDescriptor::Vector(None)
        | TypeDescriptor::Dictionary(None)
        | TypeDescriptor::Callable(_)
        | TypeDescriptor::TupleAny
        | TypeDescriptor::Negated(_) => true,
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::StringLength { .. }
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Classname(_) => false,
        TypeDescriptor::Array(Some((_, value))) | TypeDescriptor::Dictionary(Some((_, value))) => {
            descriptor_may_release_observably(value)
        }
        TypeDescriptor::Vector(Some(element)) => descriptor_may_release_observably(element),
        TypeDescriptor::VectorShape { elements, rest } => {
            elements.iter().any(descriptor_may_release_observably)
                || rest
                    .as_ref()
                    .is_some_and(|rest| descriptor_may_release_observably(rest))
        }
        TypeDescriptor::DictionaryShape { entries, rest } => {
            entries
                .iter()
                .any(|(_, value)| descriptor_may_release_observably(value))
                || rest
                    .as_ref()
                    .is_some_and(|(_, value)| descriptor_may_release_observably(value))
        }
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.iter().any(descriptor_may_release_observably)
                || descriptor_may_release_observably(rest)
        }
        TypeDescriptor::Tuple(members) | TypeDescriptor::Union(members) => {
            members.iter().any(descriptor_may_release_observably)
        }
        TypeDescriptor::Intersection(members) => {
            members.iter().all(descriptor_may_release_observably)
        }
    }
}

pub(crate) fn descriptor_proves(
    actual: &TypeDescriptor,
    expected: &TypeDescriptor,
    unit: Option<&IndexedUnit<'_>>,
    depth: usize,
) -> bool {
    if depth > MAX_TYPE_DEPTH
        || matches!(expected, TypeDescriptor::Wildcard | TypeDescriptor::Mixed)
    {
        return depth <= MAX_TYPE_DEPTH;
    }
    if descriptors_equal(actual, expected, depth + 1) || matches!(actual, TypeDescriptor::Never) {
        return true;
    }
    if let TypeDescriptor::Union(members) = actual {
        return members
            .iter()
            .all(|member| descriptor_proves(member, expected, unit, depth + 1));
    }
    if let TypeDescriptor::Union(members) = expected {
        return members
            .iter()
            .any(|member| descriptor_proves(actual, member, unit, depth + 1));
    }
    if let TypeDescriptor::Intersection(members) = actual {
        return members
            .iter()
            .any(|member| descriptor_proves(member, expected, unit, depth + 1));
    }
    if let TypeDescriptor::Intersection(members) = expected {
        return members
            .iter()
            .all(|member| descriptor_proves(actual, member, unit, depth + 1));
    }
    if let TypeDescriptor::Negated(excluded) = expected {
        return descriptors_disjoint(actual, excluded, depth + 1);
    }
    match (actual, expected) {
        (TypeDescriptor::Array(_), TypeDescriptor::Array(None))
        | (TypeDescriptor::Vector(_), TypeDescriptor::Array(None))
        | (TypeDescriptor::VectorShape { .. }, TypeDescriptor::Array(None))
        | (TypeDescriptor::Dictionary(_), TypeDescriptor::Array(None))
        | (TypeDescriptor::DictionaryShape { .. }, TypeDescriptor::Array(None))
        | (TypeDescriptor::Tuple(_), TypeDescriptor::Array(None))
        | (TypeDescriptor::TupleRest { .. }, TypeDescriptor::Array(None))
        | (TypeDescriptor::TupleAny, TypeDescriptor::Array(None)) => return true,
        (
            TypeDescriptor::Array(Some((actual_key, actual_value))),
            TypeDescriptor::Array(Some((expected_key, expected_value))),
        ) => {
            return descriptor_proves(actual_key, expected_key, unit, depth + 1)
                && descriptor_proves(actual_value, expected_value, unit, depth + 1);
        }
        (
            TypeDescriptor::Vector(Some(actual_value)),
            TypeDescriptor::Array(Some((expected_key, expected_value))),
        ) => {
            return matches!(actual_value.as_ref(), TypeDescriptor::Never)
                || descriptor_proves(
                    &TypeDescriptor::integer_range(Some(0), None),
                    expected_key,
                    unit,
                    depth + 1,
                ) && descriptor_proves(actual_value, expected_value, unit, depth + 1);
        }
        (
            TypeDescriptor::VectorShape { elements, rest },
            TypeDescriptor::Array(Some((expected_key, expected_value))),
        ) => {
            return elements.is_empty() && rest.is_none()
                || descriptor_proves(
                    &TypeDescriptor::integer_range(Some(0), None),
                    expected_key,
                    unit,
                    depth + 1,
                ) && elements
                    .iter()
                    .all(|element| descriptor_proves(element, expected_value, unit, depth + 1))
                    && rest.as_deref().is_none_or(|rest| {
                        descriptor_proves(rest, expected_value, unit, depth + 1)
                    });
        }
        (
            TypeDescriptor::Dictionary(Some((actual_key, actual_value))),
            TypeDescriptor::Array(Some((expected_key, expected_value))),
        ) => {
            if matches!(actual_key.as_ref(), TypeDescriptor::Never)
                && matches!(actual_value.as_ref(), TypeDescriptor::Never)
            {
                return true;
            }

            return descriptor_proves(actual_key, expected_key, unit, depth + 1)
                && descriptor_proves(actual_value, expected_value, unit, depth + 1);
        }
        (
            TypeDescriptor::DictionaryShape { entries, rest },
            TypeDescriptor::Array(Some((expected_key, expected_value))),
        ) => {
            return entries.iter().all(|(key, value)| {
                let key = match key {
                    ShapeKey::Int(_) => TypeDescriptor::Int,
                    ShapeKey::String(_) => TypeDescriptor::String,
                };
                descriptor_proves(&key, expected_key, unit, depth + 1)
                    && descriptor_proves(value, expected_value, unit, depth + 1)
            }) && rest.as_ref().is_none_or(|(key, value)| {
                descriptor_proves(key, expected_key, unit, depth + 1)
                    && descriptor_proves(value, expected_value, unit, depth + 1)
            });
        }
        (
            TypeDescriptor::Tuple(actual),
            TypeDescriptor::Array(Some((expected_key, expected_value))),
        ) => {
            return actual.is_empty()
                || descriptor_proves(
                    &TypeDescriptor::integer_range(Some(0), Some(actual.len() as i64 - 1)),
                    expected_key,
                    unit,
                    depth + 1,
                ) && actual
                    .iter()
                    .all(|member| descriptor_proves(member, expected_value, unit, depth + 1));
        }
        _ => {}
    }
    if let Some(unit) = unit
        && matches!(
            expected,
            TypeDescriptor::Bool
                | TypeDescriptor::Int
                | TypeDescriptor::Float
                | TypeDescriptor::String
                | TypeDescriptor::Object
                | TypeDescriptor::Callable(None)
        )
        && let (Some(actual), Some(expected)) = (
            unit.descriptor_mask(actual, depth + 1),
            descriptor_mask(expected),
        )
        && actual & !expected == 0
    {
        return true;
    }

    matches!(
        (actual, expected),
        (
            TypeDescriptor::TrueLiteral | TypeDescriptor::FalseLiteral,
            TypeDescriptor::Bool
        ) | (TypeDescriptor::IntLiteral(_), TypeDescriptor::Int)
            | (TypeDescriptor::IntRange { .. }, TypeDescriptor::Int)
            | (TypeDescriptor::FloatLiteral(_), TypeDescriptor::Float)
            | (TypeDescriptor::StringLiteral(_), TypeDescriptor::String)
            | (TypeDescriptor::StringLength { .. }, TypeDescriptor::String)
            | (TypeDescriptor::StaticClass, TypeDescriptor::Object)
    ) || matches!(
        (actual, expected),
        (
            TypeDescriptor::IntLiteral(value),
            TypeDescriptor::IntRange { min, max }
        ) if min.is_none_or(|min| *value >= min) && max.is_none_or(|max| *value <= max)
    ) || matches!(
        (actual, expected),
        (
            TypeDescriptor::IntRange {
                min: actual_min,
                max: actual_max,
            },
            TypeDescriptor::IntRange {
                min: expected_min,
                max: expected_max,
            }
        ) if range_lower_contains(*expected_min, *actual_min)
            && range_upper_contains(*expected_max, *actual_max)
    ) || matches!(
        (actual, expected),
        (
            TypeDescriptor::StringLiteral(value),
            TypeDescriptor::StringLength { min, max },
        ) if string_length_matches(value.as_bytes().len(), *min, *max)
    ) || matches!(
        (actual, expected),
        (
            TypeDescriptor::StringLength {
                min: actual_min,
                max: actual_max,
            },
            TypeDescriptor::StringLength {
                min: expected_min,
                max: expected_max,
            },
        ) if actual_min >= expected_min
            && range_upper_contains(*expected_max, *actual_max)
    )
}

pub(in crate::optimizer) fn descriptors_disjoint(
    left: &TypeDescriptor,
    right: &TypeDescriptor,
    depth: usize,
) -> bool {
    if depth > MAX_TYPE_DEPTH {
        return false;
    }
    if matches!(left, TypeDescriptor::Never) || matches!(right, TypeDescriptor::Never) {
        return true;
    }
    if descriptors_equal(left, right, depth + 1) {
        return false;
    }
    if let TypeDescriptor::Union(members) = left {
        return members
            .iter()
            .all(|member| descriptors_disjoint(member, right, depth + 1));
    }
    if let TypeDescriptor::Union(members) = right {
        return members
            .iter()
            .all(|member| descriptors_disjoint(left, member, depth + 1));
    }
    if let TypeDescriptor::Intersection(members) = left {
        return members
            .iter()
            .any(|member| descriptors_disjoint(member, right, depth + 1));
    }
    if let TypeDescriptor::Intersection(members) = right {
        return members
            .iter()
            .any(|member| descriptors_disjoint(left, member, depth + 1));
    }
    if let TypeDescriptor::Negated(excluded) = right {
        return descriptor_proves(left, excluded, None, depth + 1);
    }
    if let TypeDescriptor::Negated(excluded) = left {
        return descriptor_proves(right, excluded, None, depth + 1);
    }
    if descriptor_mask(left)
        .is_some_and(|left| descriptor_mask(right).is_some_and(|right| left & right == 0))
    {
        return true;
    }
    match (left, right) {
        (TypeDescriptor::TrueLiteral, TypeDescriptor::FalseLiteral)
        | (TypeDescriptor::FalseLiteral, TypeDescriptor::TrueLiteral) => true,
        (TypeDescriptor::IntLiteral(left), TypeDescriptor::IntLiteral(right)) => left != right,
        (TypeDescriptor::IntLiteral(value), TypeDescriptor::IntRange { min, max })
        | (TypeDescriptor::IntRange { min, max }, TypeDescriptor::IntLiteral(value)) => {
            min.is_some_and(|min| *value < min) || max.is_some_and(|max| *value > max)
        }
        (
            TypeDescriptor::IntRange {
                min: left_min,
                max: left_max,
            },
            TypeDescriptor::IntRange {
                min: right_min,
                max: right_max,
            },
        ) => {
            left_max.is_some_and(|max| right_min.is_some_and(|min| max < min))
                || right_max.is_some_and(|max| left_min.is_some_and(|min| max < min))
        }
        (TypeDescriptor::FloatLiteral(left), TypeDescriptor::FloatLiteral(right)) => left != right,
        (TypeDescriptor::StringLiteral(left), TypeDescriptor::StringLiteral(right)) => {
            left != right
        }
        (TypeDescriptor::StringLiteral(value), TypeDescriptor::StringLength { min, max })
        | (TypeDescriptor::StringLength { min, max }, TypeDescriptor::StringLiteral(value)) => {
            !string_length_matches(value.as_bytes().len(), *min, *max)
        }
        (
            TypeDescriptor::StringLength {
                min: left_min,
                max: left_max,
            },
            TypeDescriptor::StringLength {
                min: right_min,
                max: right_max,
            },
        ) => {
            left_max.is_some_and(|max| max < *right_min)
                || right_max.is_some_and(|max| max < *left_min)
        }
        (TypeDescriptor::Tuple(left), TypeDescriptor::Tuple(right)) => {
            left.len() != right.len()
                || left
                    .iter()
                    .zip(right)
                    .any(|(left, right)| descriptors_disjoint(left, right, depth + 1))
        }
        _ => false,
    }
}

pub(crate) fn descriptors_equal(
    left: &TypeDescriptor,
    right: &TypeDescriptor,
    depth: usize,
) -> bool {
    if depth > MAX_TYPE_DEPTH {
        return false;
    }
    match (left, right) {
        (TypeDescriptor::Wildcard, TypeDescriptor::Wildcard)
        | (TypeDescriptor::Mixed, TypeDescriptor::Mixed)
        | (TypeDescriptor::Void, TypeDescriptor::Void)
        | (TypeDescriptor::Never, TypeDescriptor::Never)
        | (TypeDescriptor::Null, TypeDescriptor::Null)
        | (TypeDescriptor::Bool, TypeDescriptor::Bool)
        | (TypeDescriptor::Int, TypeDescriptor::Int)
        | (TypeDescriptor::Float, TypeDescriptor::Float)
        | (TypeDescriptor::String, TypeDescriptor::String)
        | (TypeDescriptor::Object, TypeDescriptor::Object)
        | (TypeDescriptor::TrueLiteral, TypeDescriptor::TrueLiteral)
        | (TypeDescriptor::FalseLiteral, TypeDescriptor::FalseLiteral)
        | (TypeDescriptor::StaticClass, TypeDescriptor::StaticClass)
        | (TypeDescriptor::TupleAny, TypeDescriptor::TupleAny)
        | (TypeDescriptor::Callable(None), TypeDescriptor::Callable(None)) => true,
        (TypeDescriptor::IntLiteral(left), TypeDescriptor::IntLiteral(right)) => left == right,
        (
            TypeDescriptor::StringLength {
                min: left_min,
                max: left_max,
            },
            TypeDescriptor::StringLength {
                min: right_min,
                max: right_max,
            },
        ) => left_min == right_min && left_max == right_max,
        (
            TypeDescriptor::IntRange {
                min: left_min,
                max: left_max,
            },
            TypeDescriptor::IntRange {
                min: right_min,
                max: right_max,
            },
        ) => left_min == right_min && left_max == right_max,
        (TypeDescriptor::FloatLiteral(left), TypeDescriptor::FloatLiteral(right)) => {
            left.to_bits() == right.to_bits()
        }
        (TypeDescriptor::StringLiteral(left), TypeDescriptor::StringLiteral(right))
        | (TypeDescriptor::Parameter(left), TypeDescriptor::Parameter(right)) => {
            same_atom(left, right)
        }
        (
            TypeDescriptor::Named {
                name: left_name,
                arguments: left_arguments,
                ..
            },
            TypeDescriptor::Named {
                name: right_name,
                arguments: right_arguments,
                ..
            },
        ) => {
            same_atom(left_name, right_name)
                && match (left_arguments, right_arguments) {
                    (None, None) => true,
                    (Some(left), Some(right)) => descriptor_slices_equal(left, right, depth + 1),
                    _ => false,
                }
        }
        (TypeDescriptor::Array(left), TypeDescriptor::Array(right)) => match (left, right) {
            (None, None) => true,
            (Some((left_key, left_value)), Some((right_key, right_value))) => {
                descriptors_equal(left_key, right_key, depth + 1)
                    && descriptors_equal(left_value, right_value, depth + 1)
            }
            _ => false,
        },
        (TypeDescriptor::Vector(left), TypeDescriptor::Vector(right)) => {
            descriptor_options_equal(left.as_deref(), right.as_deref(), depth + 1)
        }
        (TypeDescriptor::Dictionary(left), TypeDescriptor::Dictionary(right)) => {
            match (left, right) {
                (None, None) => true,
                (Some((left_key, left_value)), Some((right_key, right_value))) => {
                    descriptors_equal(left_key, right_key, depth + 1)
                        && descriptors_equal(left_value, right_value, depth + 1)
                }
                _ => false,
            }
        }
        (TypeDescriptor::Callable(Some(left)), TypeDescriptor::Callable(Some(right))) => {
            left.parameters.len() == right.parameters.len()
                && left
                    .parameters
                    .iter()
                    .zip(&right.parameters)
                    .all(|(left, right)| {
                        left.optional == right.optional
                            && descriptors_equal(&left.r#type, &right.r#type, depth + 1)
                    })
                && descriptors_equal(&left.return_type, &right.return_type, depth + 1)
        }
        (TypeDescriptor::Classname(left), TypeDescriptor::Classname(right))
        | (TypeDescriptor::Negated(left), TypeDescriptor::Negated(right)) => {
            descriptors_equal(left, right, depth + 1)
        }
        (TypeDescriptor::Tuple(left), TypeDescriptor::Tuple(right))
        | (TypeDescriptor::Union(left), TypeDescriptor::Union(right))
        | (TypeDescriptor::Intersection(left), TypeDescriptor::Intersection(right)) => {
            descriptor_slices_equal(left, right, depth + 1)
        }
        (
            TypeDescriptor::TupleRest {
                elements: left_elements,
                rest: left_rest,
            },
            TypeDescriptor::TupleRest {
                elements: right_elements,
                rest: right_rest,
            },
        ) => {
            descriptor_slices_equal(left_elements, right_elements, depth + 1)
                && descriptors_equal(left_rest, right_rest, depth + 1)
        }
        _ => false,
    }
}

pub(in crate::optimizer) fn descriptor_options_equal(
    left: Option<&TypeDescriptor>,
    right: Option<&TypeDescriptor>,
    depth: usize,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => descriptors_equal(left, right, depth + 1),
        _ => false,
    }
}

pub(in crate::optimizer) fn descriptor_slices_equal(
    left: &[TypeDescriptor],
    right: &[TypeDescriptor],
    depth: usize,
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| descriptors_equal(left, right, depth + 1))
}

pub(in crate::optimizer) fn literal_descriptor_matches(
    literal: &Literal,
    expected: &TypeDescriptor,
) -> bool {
    match (literal, expected) {
        (Literal::Bool(true), TypeDescriptor::TrueLiteral)
        | (Literal::Bool(false), TypeDescriptor::FalseLiteral) => true,
        (Literal::Int(left), TypeDescriptor::IntLiteral(right)) => left == right,
        (Literal::Int(value), TypeDescriptor::IntRange { min, max }) => {
            min.is_none_or(|min| *value >= min) && max.is_none_or(|max| *value <= max)
        }
        (Literal::Float(left), TypeDescriptor::FloatLiteral(right)) => {
            left.to_bits() == right.to_bits()
        }
        (Literal::String(left), TypeDescriptor::StringLiteral(right)) => same_atom(left, right),
        (Literal::String(value), TypeDescriptor::StringLength { min, max }) => {
            string_length_matches(value.as_bytes().len(), *min, *max)
        }
        _ => false,
    }
}

pub(in crate::optimizer) fn literal_descriptor_disjoint(
    literal: &Literal,
    excluded: &TypeDescriptor,
) -> bool {
    let descriptor = match literal {
        Literal::Null => TypeDescriptor::Null,
        Literal::Bool(true) => TypeDescriptor::TrueLiteral,
        Literal::Bool(false) => TypeDescriptor::FalseLiteral,
        Literal::Int(value) => TypeDescriptor::IntLiteral(*value),
        Literal::Float(value) => TypeDescriptor::FloatLiteral(*value),
        Literal::String(value) => TypeDescriptor::StringLiteral(value.clone()),
    };
    descriptors_disjoint(&descriptor, excluded, 0)
}

fn range_lower_contains(expected: Option<i64>, actual: Option<i64>) -> bool {
    match (expected, actual) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(expected), Some(actual)) => actual >= expected,
    }
}

fn range_upper_contains(expected: Option<i64>, actual: Option<i64>) -> bool {
    match (expected, actual) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(expected), Some(actual)) => actual <= expected,
    }
}
