//! Dict keys: the owned and borrowed key representations, their hashing,
//! and their equality.

use crate::value::Value;
use crate::value::ValueView;
use crate::value::hash::HashState;
use crate::value::heap::handle::ManagedRef;
use crate::value::string::ByteStringObject;
use crate::value::string::short::ShortString;

pub(crate) enum Key {
    Int(i64),
    Bool(bool),
    String(ManagedRef<ByteStringObject>),
    ShortString(ShortString),
}

impl Clone for Key {
    fn clone(&self) -> Self {
        match self {
            Self::Int(value) => Self::Int(*value),
            Self::Bool(value) => Self::Bool(*value),
            Self::String(string) => Self::String(string.clone()),
            Self::ShortString(string) => Self::ShortString(*string),
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
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        KeyRef::from_value(value).map(KeyRef::to_owned)
    }

    /// Converts an owned array-key value without cloning its common forms.
    #[must_use]
    #[inline(always)]
    pub(crate) fn from_owned_value(value: Value) -> Option<Self> {
        if let Some(key) = value.as_int() {
            return Some(Self::Int(key));
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

        None
    }

    #[must_use]
    pub(crate) fn hash64(&self, state: &HashState) -> u64 {
        match self {
            Self::Int(value) => state.hash_int(*value),
            Self::Bool(value) => state.hash_bool(*value),
            Self::String(string) => string.hash64(state),
            Self::ShortString(string) => string.hash64(state),
        }
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => left == right,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::String(left), Self::String(right)) => left.eq_bytes(right),
            (Self::ShortString(left), Self::ShortString(right)) => left == right,
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
pub(crate) enum KeyRef<'a> {
    Int(i64),
    Bool(bool),
    String(&'a ManagedRef<ByteStringObject>),
    ShortString(ShortString),
}

#[expect(
    clippy::inline_always,
    reason = "borrowed key conversion runs on every dictionary lookup"
)]
impl<'a> KeyRef<'a> {
    #[must_use]
    #[inline(always)]
    pub(crate) fn from_value(value: &'a Value) -> Option<Self> {
        match value.transparent() {
            ValueView::Int(value) => Some(Self::Int(*value)),
            ValueView::Bool(value) => Some(Self::Bool(*value)),
            ValueView::String(value) => Some(Self::String(value)),
            ValueView::ShortString(value) => Some(Self::ShortString(*value)),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn to_owned(self) -> Key {
        match self {
            Self::Int(value) => Key::Int(value),
            Self::Bool(value) => Key::Bool(value),
            Self::String(value) => Key::String(value.clone()),
            Self::ShortString(value) => Key::ShortString(value),
        }
    }

    #[must_use]
    pub(crate) fn to_value(self) -> Value {
        match self {
            Self::Int(value) => Value::int(value),
            Self::Bool(value) => Value::bool(value),
            Self::String(value) => Value::string(value.clone()),
            Self::ShortString(value) => Value::short_string(value),
        }
    }

    pub(in crate::value) fn hash64(self, state: &HashState) -> u64 {
        match self {
            Self::Int(value) => state.hash_int(value),
            Self::Bool(value) => state.hash_bool(value),
            Self::String(string) => string.hash64(state),
            Self::ShortString(string) => string.hash64(state),
        }
    }
}

impl<'a> From<&'a Key> for KeyRef<'a> {
    fn from(value: &'a Key) -> Self {
        match value {
            Key::Int(value) => Self::Int(*value),
            Key::Bool(value) => Self::Bool(*value),
            Key::String(value) => Self::String(value),
            Key::ShortString(value) => Self::ShortString(*value),
        }
    }
}
