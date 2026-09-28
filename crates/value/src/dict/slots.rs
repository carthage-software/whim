//! The insertion-ordered entry slot and the key-matching predicates the
//! hash index probes with.

#![expect(
    clippy::inline_always,
    reason = "slot probes run inside dictionary hash-table lookups"
)]

use std::mem;
use std::ptr;

use whim_base::unreachable_invariant;

use crate::Value;
use crate::dict::keys::Key;
use crate::dict::keys::KeyRef;
use crate::hash::HashState;
use crate::heap::handle::ManagedRef;
use crate::string::ByteStringObject;
use crate::string::short::ShortString;

/// One position in the insertion-ordered entry vector.
pub(in crate::dict) enum Slot {
    Occupied { key: Key, value: Value },
    Vacant,
}

impl Clone for Slot {
    fn clone(&self) -> Self {
        if let Self::Occupied { key, value } = self {
            mem::forget((key.clone(), value.clone_inline_scalar()));
        }

        // SAFETY: the clones retained both owned fields before copying the slot.
        unsafe { ptr::read(self) }
    }
}

#[inline(always)]
pub(in crate::dict) fn slot_hash(slot: &Slot, state: &HashState) -> u64 {
    match slot {
        Slot::Occupied { key, .. } => key.hash64(state),
        // SAFETY: the surrounding invariant makes this path unreachable.
        Slot::Vacant => unsafe {
            unreachable_invariant("the index never references a vacant slot")
        },
    }
}

pub(in crate::dict) fn slot_matches(slot: &Slot, key: &Key) -> bool {
    match slot {
        Slot::Occupied {
            key: occupied_key, ..
        } => occupied_key == key,
        Slot::Vacant => false,
    }
}

#[inline(always)]
pub(in crate::dict) fn slot_matches_ref(slot: &Slot, key: KeyRef<'_>) -> bool {
    match slot {
        Slot::Occupied { key: occupied, .. } => match (occupied, key) {
            (Key::Int(left), KeyRef::Int(right)) => *left == right,
            (Key::Uint(left), KeyRef::Uint(right)) => *left == right,
            (Key::Bool(left), KeyRef::Bool(right)) => *left == right,
            (Key::String(left), KeyRef::String(right)) => left.eq_bytes(right),
            (Key::ShortString(left), KeyRef::ShortString(right)) => *left == right,
            (Key::String(left), KeyRef::ShortString(right)) => {
                ByteStringObject::handle_bytes(left) == right.as_bytes()
            }
            (Key::ShortString(left), KeyRef::String(right)) => {
                left.as_bytes() == ByteStringObject::handle_bytes(right)
            }
            _ => false,
        },
        Slot::Vacant => false,
    }
}

#[inline(always)]
pub(in crate::dict) fn slot_matches_string(
    slot: &Slot,
    key: &ManagedRef<ByteStringObject>,
) -> bool {
    match slot {
        Slot::Occupied {
            key: Key::String(occupied),
            ..
        } => occupied.eq_bytes(key),
        Slot::Occupied {
            key: Key::ShortString(occupied),
            ..
        } => occupied.as_bytes() == ByteStringObject::handle_bytes(key),
        _ => false,
    }
}

#[inline(always)]
pub(in crate::dict) fn slot_matches_short_string(slot: &Slot, key: ShortString) -> bool {
    match slot {
        Slot::Occupied {
            key: Key::String(occupied),
            ..
        } => ByteStringObject::handle_bytes(occupied) == key.as_bytes(),
        Slot::Occupied {
            key: Key::ShortString(occupied),
            ..
        } => *occupied == key,
        _ => false,
    }
}
