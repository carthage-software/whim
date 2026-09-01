//! The side-table payload types a chunk references: literals, type
//! descriptors, and the inline-cache, switch, preset, and catch entries.

use serde::Deserialize;
use serde::Serialize;
use serde_seeded::DeserializeSeeded;
use xxhash_rust::xxh3::xxh3_64;

use crate::bytecode::chunk::Atom;
use crate::bytecode::chunk::Comparison;
use crate::bytecode::chunk::ConstantIndex;
use crate::bytecode::chunk::DescriptorIndex;
use crate::bytecode::chunk::Register;
use crate::bytecode::instruction::operands::PropertySlot;
use crate::bytecode::instruction::operands::PropertyValueMode;
use crate::unreachable_invariant;
use crate::value::Value;
use crate::value::dict::keys::KeyRef;
use crate::value::heap::Heap;
use crate::value::string::ByteStringObject;

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) enum Literal {
    Null,
    Bool(#[seeded(with(serde_seeded::unseeded))] bool),
    Int(#[seeded(with(serde_seeded::unseeded))] i64),
    Float(#[seeded(with(serde_seeded::unseeded))] f64),
    String(Atom),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum LiteralKey {
    Null,
    Bool(bool),
    Int(i64),
    Float(u64),
    String(*const u8),
}

pub(crate) fn literal_key(literal: &Literal) -> LiteralKey {
    match literal {
        Literal::Null => LiteralKey::Null,
        Literal::Bool(value) => LiteralKey::Bool(*value),
        Literal::Int(value) => LiteralKey::Int(*value),
        Literal::Float(value) => LiteralKey::Float(value.to_bits()),
        Literal::String(atom) => LiteralKey::String(atom.as_bytes().as_ptr()),
    }
}

pub(crate) type DictionaryTypeDescriptor = (Box<TypeDescriptor>, Box<TypeDescriptor>);

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) enum ShapeKey {
    Int(#[seeded(with(serde_seeded::unseeded))] i64),
    String(Atom),
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
#[expect(
    clippy::use_self,
    reason = "the seeded derive requires the concrete recursive type"
)]
pub(crate) enum TypeDescriptor {
    Wildcard,
    Mixed,
    /// Returns normally without a value.
    Void,
    /// Never returns normally.
    Never,
    Null,
    Bool,
    Int,
    Float,
    String,
    Object,
    TrueLiteral,
    FalseLiteral,
    IntLiteral(#[seeded(with(serde_seeded::unseeded))] i64),
    IntRange {
        #[seeded(with(serde_seeded::unseeded))]
        min: Option<i64>,
        #[seeded(with(serde_seeded::unseeded))]
        max: Option<i64>,
    },
    FloatLiteral(#[seeded(with(serde_seeded::unseeded))] f64),
    StringLiteral(Atom),
    /// A top-level symbol by fully qualified name.
    /// `None` preserves a bare reference so the linker can supply
    /// declaration defaults; `Some` holds the arguments as written, or the
    /// full arguments of an already-specialized type.
    Named {
        name: Atom,
        arguments: Option<Vec<TypeDescriptor>>,
        /// Marks the named knot retained while expanding a recursive alias.
        #[seeded(with(serde_seeded::unseeded))]
        recursive: bool,
    },
    Member {
        class: Atom,
        class_arguments: Option<Vec<TypeDescriptor>>,
        member: Atom,
        member_arguments: Option<Vec<TypeDescriptor>>,
    },
    Parameter(Atom),
    StaticClass,
    /// A read-only view over a vec, dict, or tuple, optionally constrained by
    /// key and value type.
    Array(
        #[seeded(with(crate::bytecode::decode::pairs::optional))] Option<DictionaryTypeDescriptor>,
    ),
    Vector(Option<Box<TypeDescriptor>>),
    /// A vec with fixed leading positions and an optional homogeneous tail.
    VectorShape {
        elements: Vec<TypeDescriptor>,
        rest: Option<Box<TypeDescriptor>>,
    },
    Dictionary(
        #[seeded(with(crate::bytecode::decode::pairs::optional))] Option<DictionaryTypeDescriptor>,
    ),
    DictionaryShape {
        #[seeded(with(crate::bytecode::decode::pairs))]
        entries: Vec<(ShapeKey, TypeDescriptor)>,
        #[seeded(with(crate::bytecode::decode::pairs::optional))]
        rest: Option<DictionaryTypeDescriptor>,
    },
    Callable(Option<FunctionTypeDescriptor>),
    Classname(Box<TypeDescriptor>),
    Tuple(Vec<TypeDescriptor>),
    /// A tuple with fixed leading positions and any number of trailing values
    /// satisfying one homogeneous descriptor.
    TupleRest {
        elements: Vec<TypeDescriptor>,
        rest: Box<TypeDescriptor>,
    },
    TupleAny,
    Union(Vec<TypeDescriptor>),
    Intersection(Vec<TypeDescriptor>),
    /// The complement of a runtime-checkable type relative to `mixed`.
    Negated(Box<TypeDescriptor>),
}

impl TypeDescriptor {
    #[must_use]
    pub(crate) fn integer_range(min: Option<i64>, max: Option<i64>) -> Self {
        if min.zip(max).is_some_and(|(min, max)| min > max) {
            Self::Never
        } else if min.is_none_or(|min| min == i64::MIN) && max.is_none_or(|max| max == i64::MAX) {
            Self::Int
        } else if let (Some(min), Some(max)) = (min, max)
            && min == max
        {
            Self::IntLiteral(min)
        } else {
            Self::IntRange { min, max }
        }
    }

    /// Rebuilds this descriptor after transforming each direct child.
    pub(crate) fn map_children(&self, mut map: impl FnMut(&Self) -> Self) -> Self {
        match self {
            Self::Named {
                name,
                arguments,
                recursive,
            } => Self::Named {
                name: name.clone(),
                arguments: arguments
                    .as_ref()
                    .map(|arguments| arguments.iter().map(&mut map).collect()),
                recursive: *recursive,
            },
            Self::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => Self::Member {
                class: class.clone(),
                class_arguments: class_arguments
                    .as_ref()
                    .map(|arguments| arguments.iter().map(&mut map).collect()),
                member: member.clone(),
                member_arguments: member_arguments
                    .as_ref()
                    .map(|arguments| arguments.iter().map(&mut map).collect()),
            },
            Self::Array(arguments) => Self::Array(
                arguments
                    .as_ref()
                    .map(|(key, value)| (Box::new(map(key)), Box::new(map(value)))),
            ),
            Self::Vector(element) => {
                Self::Vector(element.as_ref().map(|element| Box::new(map(element))))
            }
            Self::VectorShape { elements, rest } => Self::VectorShape {
                elements: elements.iter().map(&mut map).collect(),
                rest: rest.as_ref().map(|rest| Box::new(map(rest))),
            },
            Self::Dictionary(arguments) => Self::Dictionary(
                arguments
                    .as_ref()
                    .map(|(key, value)| (Box::new(map(key)), Box::new(map(value)))),
            ),
            Self::DictionaryShape { entries, rest } => Self::DictionaryShape {
                entries: entries
                    .iter()
                    .map(|(key, value)| (key.clone(), map(value)))
                    .collect(),
                rest: rest
                    .as_ref()
                    .map(|(key, value)| (Box::new(map(key)), Box::new(map(value)))),
            },
            Self::Callable(signature) => Self::Callable(signature.as_ref().map(|signature| {
                FunctionTypeDescriptor {
                    parameters: signature
                        .parameters
                        .iter()
                        .map(|parameter| FunctionTypeParameterDescriptor {
                            r#type: map(&parameter.r#type),
                            optional: parameter.optional,
                        })
                        .collect(),
                    return_type: Box::new(map(&signature.return_type)),
                }
            })),
            Self::Classname(inner) => Self::Classname(Box::new(map(inner))),
            Self::Tuple(members) => Self::Tuple(members.iter().map(&mut map).collect()),
            Self::TupleRest { elements, rest } => Self::TupleRest {
                elements: elements.iter().map(&mut map).collect(),
                rest: Box::new(map(rest)),
            },
            Self::Union(members) => Self::Union(members.iter().map(&mut map).collect()),
            Self::Intersection(members) => {
                Self::Intersection(members.iter().map(&mut map).collect())
            }
            Self::Negated(inner) => Self::Negated(Box::new(map(inner))),
            _ => self.clone(),
        }
    }

    /// Conservative: false only when the descriptor is statically limited to
    /// immediate scalar values, letting the VM skip inspecting those
    /// parameters.
    #[must_use]
    pub(crate) fn may_hold_reference(&self) -> bool {
        match self {
            Self::Void
            | Self::Never
            | Self::Null
            | Self::Bool
            | Self::Int
            | Self::Float
            | Self::TrueLiteral
            | Self::FalseLiteral
            | Self::IntLiteral(_)
            | Self::IntRange { .. }
            | Self::FloatLiteral(_) => false,
            Self::Union(members) => members.iter().any(Self::may_hold_reference),
            Self::Wildcard
            | Self::Mixed
            | Self::String
            | Self::Object
            | Self::StringLiteral(_)
            | Self::Named { .. }
            | Self::Member { .. }
            | Self::Parameter(_)
            | Self::StaticClass
            | Self::Array(_)
            | Self::Vector(_)
            | Self::VectorShape { .. }
            | Self::Dictionary(_)
            | Self::DictionaryShape { .. }
            | Self::Callable(_)
            | Self::Classname(_)
            | Self::Tuple(_)
            | Self::TupleRest { .. }
            | Self::TupleAny
            | Self::Intersection(_)
            | Self::Negated(_) => true,
        }
    }
}

#[must_use]
pub(crate) fn descriptor_is_trivial(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::Object
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Array(None)
        | TypeDescriptor::Vector(None)
        | TypeDescriptor::Dictionary(None)
        | TypeDescriptor::Callable(None)
        | TypeDescriptor::TupleAny => true,
        TypeDescriptor::Array(Some((key, value)))
        | TypeDescriptor::Dictionary(Some((key, value))) => {
            matches!(key.as_ref(), TypeDescriptor::Wildcard)
                && matches!(value.as_ref(), TypeDescriptor::Wildcard)
        }
        TypeDescriptor::Vector(Some(element)) => {
            matches!(element.as_ref(), TypeDescriptor::Wildcard)
        }
        TypeDescriptor::Negated(element) => descriptor_is_trivial(element),
        TypeDescriptor::VectorShape { elements, rest } => {
            elements.iter().all(descriptor_is_trivial)
                && rest.as_deref().is_none_or(descriptor_is_trivial)
        }
        TypeDescriptor::DictionaryShape { entries, rest } => {
            entries
                .iter()
                .all(|(_, value)| descriptor_is_trivial(value))
                && rest.as_ref().is_none_or(|(key, value)| {
                    descriptor_is_trivial(key) && descriptor_is_trivial(value)
                })
        }
        TypeDescriptor::Tuple(elements)
        | TypeDescriptor::Union(elements)
        | TypeDescriptor::Intersection(elements) => elements.iter().all(descriptor_is_trivial),
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.iter().all(descriptor_is_trivial) && descriptor_is_trivial(rest)
        }
        TypeDescriptor::Named { .. }
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::StaticClass
        | TypeDescriptor::Callable(Some(_))
        | TypeDescriptor::Classname(_) => false,
    }
}

fn check_vector_shape(
    elements: &[TypeDescriptor],
    rest: Option<&TypeDescriptor>,
    value: &Value,
) -> Option<bool> {
    let Some(vector) = value.as_vec() else {
        return Some(false);
    };
    if vector.len() < elements.len() || rest.is_none() && vector.len() != elements.len() {
        return Some(false);
    }
    for (descriptor, value) in elements.iter().zip(vector.iter()) {
        if !check_trivial_descriptor(descriptor, value)? {
            return Some(false);
        }
    }
    if let Some(rest) = rest {
        for value in vector.iter().skip(elements.len()) {
            if !check_trivial_descriptor(rest, value)? {
                return Some(false);
            }
        }
    }

    Some(true)
}

fn shape_key_matches(expected: &ShapeKey, actual: KeyRef<'_>) -> bool {
    match (expected, actual) {
        (ShapeKey::Int(expected), KeyRef::Int(actual)) => *expected == actual,
        (ShapeKey::String(expected), KeyRef::String(actual)) => {
            expected.as_bytes() == ByteStringObject::handle_bytes(actual)
        }
        (ShapeKey::String(expected), KeyRef::ShortString(actual)) => {
            expected.as_bytes() == actual.as_bytes()
        }
        _ => false,
    }
}

fn check_dictionary_shape(
    entries: &[(ShapeKey, TypeDescriptor)],
    rest: Option<&DictionaryTypeDescriptor>,
    value: &Value,
) -> Option<bool> {
    let Some(dictionary) = value.as_dict() else {
        return Some(false);
    };
    if dictionary.len() < entries.len() || rest.is_none() && dictionary.len() != entries.len() {
        return Some(false);
    }
    for (key, descriptor) in entries {
        let value = match key {
            ShapeKey::Int(key) => dictionary.get_int(*key),
            ShapeKey::String(key) => dictionary.get_string(key.as_handle()),
        };
        let Some(value) = value else {
            return Some(false);
        };
        if !check_trivial_descriptor(descriptor, value)? {
            return Some(false);
        }
    }
    let Some((key_descriptor, value_descriptor)) = rest else {
        return Some(true);
    };
    for (key, value) in dictionary.iter() {
        if entries
            .iter()
            .any(|(expected, _)| shape_key_matches(expected, key))
        {
            continue;
        }
        let key = match key {
            KeyRef::Int(key) => Value::int(key),
            KeyRef::Bool(key) => Value::bool(key),
            KeyRef::String(key) => Value::string(key.clone()),
            KeyRef::ShortString(key) => Value::short_string(key),
        };
        if !check_trivial_descriptor(key_descriptor, &key)?
            || !check_trivial_descriptor(value_descriptor, value)?
        {
            return Some(false);
        }
    }

    Some(true)
}

fn check_tuple_shape(
    elements: &[TypeDescriptor],
    rest: Option<&TypeDescriptor>,
    value: &Value,
) -> Option<bool> {
    let Some(tuple) = value.as_tuple() else {
        return Some(false);
    };
    if tuple.len() < elements.len() || rest.is_none() && tuple.len() != elements.len() {
        return Some(false);
    }
    for (descriptor, value) in elements.iter().zip(tuple.iter()) {
        if !check_trivial_descriptor(descriptor, value)? {
            return Some(false);
        }
    }
    if let Some(rest) = rest {
        for value in tuple.iter().skip(elements.len()) {
            if !check_trivial_descriptor(rest, value)? {
                return Some(false);
            }
        }
    }

    Some(true)
}

fn check_descriptor_members(
    members: &[TypeDescriptor],
    value: &Value,
    short_circuit: bool,
) -> Option<bool> {
    let mut undecided = false;
    for member in members {
        match check_trivial_descriptor(member, value) {
            Some(result) if result == short_circuit => return Some(result),
            Some(_) => {}
            None => undecided = true,
        }
    }
    if undecided {
        None
    } else {
        Some(!short_circuit)
    }
}

#[expect(
    clippy::inline_always,
    reason = "trivial type checks sit on every checked VM boundary"
)]
#[inline(always)]
pub(crate) fn check_trivial_descriptor(descriptor: &TypeDescriptor, value: &Value) -> Option<bool> {
    Some(match descriptor {
        TypeDescriptor::Wildcard | TypeDescriptor::Mixed => true,
        TypeDescriptor::Void | TypeDescriptor::Never => false,
        TypeDescriptor::Null => value.is_null(),
        TypeDescriptor::Bool => value.is_bool(),
        TypeDescriptor::Int => value.is_int(),
        TypeDescriptor::Float => value.is_float(),
        TypeDescriptor::String => value.is_string(),
        TypeDescriptor::Object => value.is_object(),
        TypeDescriptor::TrueLiteral => value.as_bool() == Some(true),
        TypeDescriptor::FalseLiteral => value.as_bool() == Some(false),
        TypeDescriptor::IntLiteral(expected) => value.as_int() == Some(*expected),
        TypeDescriptor::IntRange { min, max } => value.as_int().is_some_and(|value| {
            min.is_none_or(|min| value >= min) && max.is_none_or(|max| value <= max)
        }),
        TypeDescriptor::FloatLiteral(expected) => value.as_float() == Some(*expected),
        TypeDescriptor::StringLiteral(expected) => value
            .as_string_bytes()
            .is_some_and(|string| string == expected.as_bytes()),
        TypeDescriptor::Array(None) => value.is_vec() || value.is_dict() || value.is_tuple(),
        TypeDescriptor::Array(Some((key, element)))
            if matches!(key.as_ref(), TypeDescriptor::Wildcard)
                && matches!(element.as_ref(), TypeDescriptor::Wildcard) =>
        {
            value.is_vec() || value.is_dict() || value.is_tuple()
        }
        TypeDescriptor::Array(Some(_)) => {
            let empty = value
                .as_vec()
                .map(|value| value.is_empty())
                .or_else(|| value.as_dict().map(|value| value.is_empty()))
                .or_else(|| value.as_tuple().map(|value| value.len() == 0));
            return match empty {
                Some(true) => Some(true),
                Some(false) => None,
                None => Some(false),
            };
        }
        TypeDescriptor::Vector(None) => value.is_vec(),
        TypeDescriptor::Vector(Some(element))
            if matches!(element.as_ref(), TypeDescriptor::Wildcard) =>
        {
            value.is_vec()
        }
        TypeDescriptor::Vector(Some(_)) => {
            return match value.as_vec() {
                Some(value) if value.is_empty() => Some(true),
                Some(_) => None,
                None => Some(false),
            };
        }
        TypeDescriptor::VectorShape { elements, rest } => {
            return check_vector_shape(elements, rest.as_deref(), value);
        }
        TypeDescriptor::Dictionary(None) => value.is_dict(),
        TypeDescriptor::Dictionary(Some((key, element)))
            if matches!(key.as_ref(), TypeDescriptor::Wildcard)
                && matches!(element.as_ref(), TypeDescriptor::Wildcard) =>
        {
            value.is_dict()
        }
        TypeDescriptor::Dictionary(Some(_)) => {
            return match value.as_dict() {
                Some(value) if value.is_empty() => Some(true),
                Some(_) => None,
                None => Some(false),
            };
        }
        TypeDescriptor::DictionaryShape { entries, rest } => {
            return check_dictionary_shape(entries, rest.as_ref(), value);
        }
        TypeDescriptor::Callable(None) => value.is_function(),
        TypeDescriptor::TupleAny => value.is_tuple(),
        TypeDescriptor::Tuple(elements) => return check_tuple_shape(elements, None, value),
        TypeDescriptor::TupleRest { elements, rest } => {
            return check_tuple_shape(elements, Some(rest), value);
        }
        TypeDescriptor::Negated(inner) => {
            return check_trivial_descriptor(inner, value).map(|matches| !matches);
        }
        TypeDescriptor::Union(members) => return check_descriptor_members(members, value, true),
        TypeDescriptor::Intersection(members) => {
            return check_descriptor_members(members, value, false);
        }
        _ => return None,
    })
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) struct FunctionTypeDescriptor {
    /// Parameters in call order.
    pub parameters: Vec<FunctionTypeParameterDescriptor>,
    pub return_type: Box<TypeDescriptor>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) struct FunctionTypeParameterDescriptor {
    pub r#type: TypeDescriptor,
    #[seeded(with(serde_seeded::unseeded))]
    pub optional: bool,
}

/// The shape of a call passing named arguments: named values follow the
/// positionals in the register window, in descriptor order; the VM maps
/// names to parameters.
#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) struct CallDescriptor {
    #[seeded(with(serde_seeded::unseeded))]
    pub positional: u8,
    pub named: Vec<Atom>,
}

/// A match jump table; each target is an
/// instruction offset relative to the switch instruction, like
/// [`JumpOffset`](crate::bytecode::instruction::operands::JumpOffset).
#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) enum SwitchTable {
    Int {
        #[seeded(with(serde_seeded::unseeded))]
        base: i64,
        #[seeded(with(serde_seeded::unseeded))]
        targets: Vec<i32>,
        #[seeded(with(serde_seeded::unseeded))]
        default: i32,
    },
    String {
        #[seeded(with(crate::bytecode::decode::atom_i32_pairs))]
        arms: Vec<(Atom, i32)>,
        #[seeded(with(serde_seeded::unseeded))]
        buckets: Vec<u32>,
        #[seeded(with(serde_seeded::unseeded))]
        default: i32,
    },
    StringByte {
        #[seeded(with(serde_seeded::unseeded))]
        base: u8,
        #[seeded(with(serde_seeded::unseeded))]
        targets: Vec<i32>,
        #[seeded(with(serde_seeded::unseeded))]
        default: i32,
    },
    Pattern {
        descriptors: Vec<TypeDescriptor>,
        #[seeded(with(serde_seeded::unseeded))]
        targets: Vec<i32>,
        #[seeded(with(serde_seeded::unseeded))]
        default: i32,
    },
    DictionaryShape {
        keys: Vec<ShapeKey>,
        patterns: Vec<Vec<TypeDescriptor>>,
        #[seeded(with(serde_seeded::unseeded))]
        targets: Vec<i32>,
        #[seeded(with(serde_seeded::unseeded))]
        default: i32,
    },
    Bool {
        #[seeded(with(serde_seeded::unseeded))]
        targets: Vec<i32>,
        #[seeded(with(serde_seeded::unseeded))]
        default: i32,
    },
    Float {
        #[seeded(with(serde_seeded::unseeded))]
        values: Vec<f64>,
        #[seeded(with(serde_seeded::unseeded))]
        targets: Vec<i32>,
        #[seeded(with(serde_seeded::unseeded))]
        default: i32,
    },
}

pub(crate) fn string_switch_buckets(arms: &[(Atom, i32)]) -> Vec<u32> {
    let base = arms.len().max(1).next_power_of_two();
    let Some(width) = base.checked_mul(2) else {
        // SAFETY: the bytecode count limit keeps a switch below this size.
        unsafe { unreachable_invariant("a string switch table has room for empty buckets") }
    };
    let mut buckets = vec![0; width];
    let mask = width - 1;
    for (index, (value, _)) in arms.iter().enumerate() {
        let Ok(entry) = u32::try_from(index + 1) else {
            // SAFETY: the bytecode count limit keeps every arm index in u32.
            unsafe { unreachable_invariant("a string switch arm index fits in u32") }
        };
        let mut bucket = string_switch_bucket(value.as_bytes(), mask);
        while buckets[bucket] != 0 {
            bucket = (bucket + 1) & mask;
        }
        buckets[bucket] = entry;
    }
    buckets
}

pub(crate) fn string_switch_lookup(
    arms: &[(Atom, i32)],
    buckets: &[u32],
    value: &[u8],
) -> Option<usize> {
    let mask = buckets.len().checked_sub(1)?;
    let mut bucket = string_switch_bucket(value, mask);
    for _ in 0..buckets.len() {
        let entry = buckets[bucket];
        if entry == 0 {
            return None;
        }
        let index = usize::try_from(entry - 1).ok()?;
        if arms.get(index)?.0.as_bytes() == value {
            return Some(index);
        }
        bucket = (bucket + 1) & mask;
    }
    None
}

fn string_switch_bucket(value: &[u8], mask: usize) -> usize {
    let hash = xxh3_64(value);
    let hash = usize::try_from(hash).unwrap_or_else(|_| {
        let folded = (hash ^ (hash >> 32)) & u64::from(u32::MAX);
        let Ok(folded) = usize::try_from(folded) else {
            // SAFETY: every supported target has at least thirty-two-bit pointers.
            unsafe { unreachable_invariant("a folded string-switch hash fits in usize") }
        };
        folded
    });
    hash & mask
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) enum PresetSlot {
    /// A positional argument fixed when the partial was built; its value
    /// sits in the instruction's window.
    GivenPositional,
    HolePositional,
    /// A named argument fixed when the partial was built; its value sits in
    /// the window.
    GivenNamed(Atom),
    HoleNamed(Atom),
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) struct PresetDescriptor {
    /// Given values and holes in call order.
    pub slots: Vec<PresetSlot>,
    /// Whether a trailing `...` exposes every parameter not otherwise named.
    #[seeded(with(serde_seeded::unseeded))]
    pub open_remaining: bool,
    /// Explicit turbofish arguments, when written.
    pub type_arguments: Option<Vec<TypeDescriptor>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct CatchEntry {
    /// The first protected instruction index, inclusive.
    pub start: u32,
    /// The end of the protected range, exclusive.
    pub end: u32,
    pub handler: u32,
    pub type_descriptor: DescriptorIndex,
    pub temporary_floor: u16,
    /// The register the caught error binds to, when the catch names one.
    pub binding: Option<Register>,
}

/// What an inline-cache site resolves: the names the VM looks up on first
/// execution and caches in the site's slot thereafter.
#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub(crate) enum IcDescriptor {
    Member {
        name: Atom,
        /// Class type arguments for an instantiation site; absent for every
        /// other member cache.
        type_arguments: Option<Vec<TypeDescriptor>>,
    },
    ClassMember {
        class: Atom,
        member: Atom,
        /// A turbofish written on a static call; absent for every other
        /// class-member cache, and for a call that writes none.
        type_arguments: Option<Vec<TypeDescriptor>>,
    },
}

/// Type facts established once before an integer-controlled numeric loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedIntLoopDescriptor {
    pub comparison: Comparison,
    pub counter: Register,
    pub limit: Register,
    pub float_registers: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IntStepLoopDescriptor {
    pub comparison: Comparison,
    pub counter: Register,
    pub limit: Register,
    pub step: Register,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FloatSquaresSumBranchDescriptor {
    pub sum_destination: Register,
    pub first_square_destination: Register,
    pub second_square_destination: Register,
    pub first_source: Register,
    pub second_source: Register,
    pub comparison: Comparison,
    pub constant: ConstantIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FloatPairUpdateDescriptor {
    pub first_destination: Register,
    pub first_operand: Register,
    pub constant: ConstantIndex,
    pub second_destination: Register,
    pub second_operand: Register,
    pub second_addend: Register,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PropertyInitializationDescriptor {
    pub allocates: bool,
    pub entries: Vec<PropertyInitializationEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PropertyInitializationEntry {
    pub value: Register,
    pub slot: PropertySlot,
    pub value_mode: PropertyValueMode,
}
