//! Dict keys: the owned and borrowed key representations, their hashing,
//! and their equality.

use crate::Value;
use crate::ValueView;
use crate::hash::HashState;
use crate::heap::handle::ManagedRef;
use crate::newtype::NewtypeValueId;
use crate::string::ByteStringObject;
use crate::string::short::ShortString;

pub enum Key {
    Int(i64),
    Uint(u64),
    Bool(bool),
    String(ManagedRef<ByteStringObject>),
    ShortString(ShortString),
    NewtypeInt(i64, NewtypeValueId),
    NewtypeUint(u64, NewtypeValueId),
    NewtypeBool(bool, NewtypeValueId),
    NewtypeString(ManagedRef<ByteStringObject>, NewtypeValueId),
    NewtypeShortString(ShortString, NewtypeValueId),
    Fresh(u64),
    NewtypeFresh(u64, NewtypeValueId),
}

const _: () = assert!(size_of::<Key>() == 16);

impl Clone for Key {
    fn clone(&self) -> Self {
        match self {
            Self::Int(value) => Self::Int(*value),
            Self::Uint(value) => Self::Uint(*value),
            Self::Fresh(value) => Self::Fresh(*value),
            Self::Bool(value) => Self::Bool(*value),
            Self::String(string) => Self::String(string.clone()),
            Self::ShortString(string) => Self::ShortString(*string),
            Self::NewtypeInt(value, tag) => Self::NewtypeInt(*value, *tag),
            Self::NewtypeUint(value, tag) => Self::NewtypeUint(*value, *tag),
            Self::NewtypeFresh(value, tag) => Self::NewtypeFresh(*value, *tag),
            Self::NewtypeBool(value, tag) => Self::NewtypeBool(*value, *tag),
            Self::NewtypeString(string, tag) => Self::NewtypeString(string.clone(), *tag),
            Self::NewtypeShortString(string, tag) => Self::NewtypeShortString(*string, *tag),
        }
    }
}

#[expect(
    clippy::inline_always,
    reason = "key conversion runs on every dictionary lookup and insertion"
)]
impl Key {
    #[must_use]
    #[inline(always)]
    pub fn from_value(value: &Value) -> Option<Self> {
        KeyRef::from_value(value).map(KeyRef::to_owned)
    }

    /// Converts an owned array-key value without cloning its common forms.
    #[must_use]
    #[inline(always)]
    pub fn from_owned_value(value: Value) -> Option<Self> {
        if value.newtype_id().is_some() {
            return Self::from_value(&value);
        }
        if let Some(key) = value.as_int() {
            return Some(Self::Int(key));
        }
        if let Some(key) = value.as_uint() {
            return Some(Self::Uint(key));
        }
        if let Some(key) = value.as_bool() {
            return Some(Self::Bool(key));
        }
        if let Some(key) = value.as_short_string() {
            return Some(Self::ShortString(key));
        }
        if value.is_string() {
            // SAFETY: the value's tag proves this projection is valid.
            return Some(Self::String(unsafe { value.into_string_unchecked() }));
        }

        if let Some(key) = value.as_fresh() {
            return Some(Self::Fresh(key));
        }

        None
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn hash64(&self, state: &HashState) -> u64 {
        match self {
            Self::Int(value) => state.hash_int(*value),
            Self::Uint(value) => state.hash_uint(*value),
            Self::Fresh(value) => state.hash_fresh(*value),
            Self::Bool(value) => state.hash_bool(*value),
            Self::String(string) => string.hash64(state),
            Self::ShortString(string) => string.hash64(state),
            Self::NewtypeInt(value, tag) => state.hash_newtype(state.hash_int(*value), *tag),
            Self::NewtypeUint(value, tag) => state.hash_newtype(state.hash_uint(*value), *tag),
            Self::NewtypeFresh(value, tag) => state.hash_newtype(state.hash_fresh(*value), *tag),
            Self::NewtypeBool(value, tag) => state.hash_newtype(state.hash_bool(*value), *tag),
            Self::NewtypeString(string, tag) => state.hash_newtype(string.hash64(state), *tag),
            Self::NewtypeShortString(string, tag) => state.hash_newtype(string.hash64(state), *tag),
        }
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => left == right,
            (Self::Uint(left), Self::Uint(right)) | (Self::Fresh(left), Self::Fresh(right)) => {
                left == right
            }
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::String(left), Self::String(right)) => left.eq_bytes(right),
            (Self::ShortString(left), Self::ShortString(right)) => left == right,
            (Self::NewtypeInt(left, a), Self::NewtypeInt(right, b)) => a == b && left == right,
            (Self::NewtypeUint(left, a), Self::NewtypeUint(right, b))
            | (Self::NewtypeFresh(left, a), Self::NewtypeFresh(right, b)) => {
                a == b && left == right
            }
            (Self::NewtypeBool(left, a), Self::NewtypeBool(right, b)) => a == b && left == right,
            (Self::NewtypeString(left, a), Self::NewtypeString(right, b)) => {
                a == b && left.eq_bytes(right)
            }
            (Self::NewtypeShortString(left, a), Self::NewtypeShortString(right, b)) => {
                a == b && left == right
            }
            (Self::NewtypeString(left, a), Self::NewtypeShortString(right, b))
            | (Self::NewtypeShortString(right, b), Self::NewtypeString(left, a)) => {
                a == b && ByteStringObject::handle_bytes(left) == right.as_bytes()
            }
            (Self::String(left), Self::ShortString(right))
            | (Self::ShortString(right), Self::String(left)) => {
                ByteStringObject::handle_bytes(left) == right.as_bytes()
            }
            _ => false,
        }
    }
}

impl Eq for Key {}

#[derive(Clone, Copy)]
pub enum KeyRef<'a> {
    Int(i64),
    Uint(u64),
    Bool(bool),
    String(&'a ManagedRef<ByteStringObject>),
    ShortString(ShortString),
    NewtypeInt(i64, NewtypeValueId),
    NewtypeUint(u64, NewtypeValueId),
    NewtypeBool(bool, NewtypeValueId),
    NewtypeString(&'a ManagedRef<ByteStringObject>, NewtypeValueId),
    NewtypeShortString(ShortString, NewtypeValueId),
    Fresh(u64),
    NewtypeFresh(u64, NewtypeValueId),
}

const _: () = assert!(size_of::<KeyRef<'_>>() == 16);

#[expect(
    clippy::inline_always,
    reason = "borrowed key conversion runs on every dictionary lookup"
)]
impl<'a> KeyRef<'a> {
    #[must_use]
    #[inline(always)]
    pub fn from_value(value: &'a Value) -> Option<Self> {
        if let Some(tag) = value.newtype_id() {
            return Self::from_newtype_value(value, tag);
        }
        match value.transparent() {
            ValueView::Int(value) => Some(Self::Int(*value)),
            ValueView::Uint(value) => Some(Self::Uint(*value)),
            ValueView::Fresh(value) => Some(Self::Fresh(*value)),
            ValueView::Bool(value) => Some(Self::Bool(*value)),
            ValueView::String(value) => Some(Self::String(value)),
            ValueView::ShortString(value) => Some(Self::ShortString(*value)),
            _ => None,
        }
    }

    #[cold]
    #[inline(never)]
    fn from_newtype_value(value: &'a Value, tag: NewtypeValueId) -> Option<Self> {
        match value.transparent() {
            ValueView::Int(value) => Some(Self::NewtypeInt(*value, tag)),
            ValueView::Uint(value) => Some(Self::NewtypeUint(*value, tag)),
            ValueView::Fresh(value) => Some(Self::NewtypeFresh(*value, tag)),
            ValueView::Bool(value) => Some(Self::NewtypeBool(*value, tag)),
            ValueView::String(value) => Some(Self::NewtypeString(value, tag)),
            ValueView::ShortString(value) => Some(Self::NewtypeShortString(*value, tag)),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_owned(self) -> Key {
        match self {
            Self::Int(value) => Key::Int(value),
            Self::Uint(value) => Key::Uint(value),
            Self::Fresh(value) => Key::Fresh(value),
            Self::Bool(value) => Key::Bool(value),
            Self::String(value) => Key::String(value.clone()),
            Self::ShortString(value) => Key::ShortString(value),
            Self::NewtypeInt(value, tag) => Key::NewtypeInt(value, tag),
            Self::NewtypeUint(value, tag) => Key::NewtypeUint(value, tag),
            Self::NewtypeFresh(value, tag) => Key::NewtypeFresh(value, tag),
            Self::NewtypeBool(value, tag) => Key::NewtypeBool(value, tag),
            Self::NewtypeString(value, tag) => Key::NewtypeString(value.clone(), tag),
            Self::NewtypeShortString(value, tag) => Key::NewtypeShortString(value, tag),
        }
    }

    #[must_use]
    #[inline(always)]
    pub fn to_value(self) -> Value {
        match self {
            Self::Int(value) => Value::int(value),
            Self::Uint(value) => Value::uint(value),
            Self::Fresh(value) => Value::fresh(value),
            Self::Bool(value) => Value::bool(value),
            Self::String(value) => Value::string(value.clone()),
            Self::ShortString(value) => Value::short_string(value),
            Self::NewtypeInt(value, tag) => Value::newtype(Value::int(value), tag),
            Self::NewtypeUint(value, tag) => Value::newtype(Value::uint(value), tag),
            Self::NewtypeFresh(value, tag) => Value::newtype(Value::fresh(value), tag),
            Self::NewtypeBool(value, tag) => Value::newtype(Value::bool(value), tag),
            Self::NewtypeString(value, tag) => Value::newtype(Value::string(value.clone()), tag),
            Self::NewtypeShortString(value, tag) => Value::newtype(Value::short_string(value), tag),
        }
    }

    #[must_use]
    pub const fn without_newtype(self) -> Self {
        match self {
            Self::NewtypeInt(value, _) => Self::Int(value),
            Self::NewtypeUint(value, _) => Self::Uint(value),
            Self::NewtypeFresh(value, _) => Self::Fresh(value),
            Self::NewtypeBool(value, _) => Self::Bool(value),
            Self::NewtypeString(value, _) => Self::String(value),
            Self::NewtypeShortString(value, _) => Self::ShortString(value),
            key => key,
        }
    }

    #[inline(always)]
    pub(crate) fn hash64(self, state: &HashState) -> u64 {
        match self {
            Self::Int(value) => state.hash_int(value),
            Self::Uint(value) => state.hash_uint(value),
            Self::Fresh(value) => state.hash_fresh(value),
            Self::Bool(value) => state.hash_bool(value),
            Self::String(string) => string.hash64(state),
            Self::ShortString(string) => string.hash64(state),
            Self::NewtypeInt(value, tag) => state.hash_newtype(state.hash_int(value), tag),
            Self::NewtypeUint(value, tag) => state.hash_newtype(state.hash_uint(value), tag),
            Self::NewtypeFresh(value, tag) => state.hash_newtype(state.hash_fresh(value), tag),
            Self::NewtypeBool(value, tag) => state.hash_newtype(state.hash_bool(value), tag),
            Self::NewtypeString(string, tag) => state.hash_newtype(string.hash64(state), tag),
            Self::NewtypeShortString(string, tag) => state.hash_newtype(string.hash64(state), tag),
        }
    }
}

impl<'a> From<&'a Key> for KeyRef<'a> {
    fn from(value: &'a Key) -> Self {
        match value {
            Key::Int(value) => Self::Int(*value),
            Key::Uint(value) => Self::Uint(*value),
            Key::Fresh(value) => Self::Fresh(*value),
            Key::Bool(value) => Self::Bool(*value),
            Key::String(value) => Self::String(value),
            Key::ShortString(value) => Self::ShortString(*value),
            Key::NewtypeInt(value, tag) => Self::NewtypeInt(*value, *tag),
            Key::NewtypeUint(value, tag) => Self::NewtypeUint(*value, *tag),
            Key::NewtypeFresh(value, tag) => Self::NewtypeFresh(*value, *tag),
            Key::NewtypeBool(value, tag) => Self::NewtypeBool(*value, *tag),
            Key::NewtypeString(value, tag) => Self::NewtypeString(value, *tag),
            Key::NewtypeShortString(value, tag) => Self::NewtypeShortString(*value, *tag),
        }
    }
}
