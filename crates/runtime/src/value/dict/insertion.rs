//! Specialized dict insertion paths selected by proven key representation.

use std::mem;

use hashbrown::hash_table::Entry;

use crate::unreachable_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::dict::DictObject;
use crate::value::dict::IndexEntry;
use crate::value::dict::keys::Key;
use crate::value::dict::slots::Slot;
use crate::value::dict::slots::slot_hash;
use crate::value::dict::slots::slot_matches_short_string;
use crate::value::string::short::ShortString;

#[expect(
    clippy::inline_always,
    reason = "specialized insertion is part of the VM dictionary fast path"
)]
impl DictObject {
    #[inline(always)]
    pub(crate) fn insert_short_string(&mut self, key: ShortString, value: Value) -> Option<Value> {
        self.materialize_index();
        let hash_state = self.hash_state;
        // SAFETY: the heap-owned hash state outlives the dict.
        let hash_state = unsafe { hash_state.as_ref() };
        let hash = key.hash64(hash_state);
        // SAFETY: the surrounding invariant proves this result is successful.
        let next_position = unsafe {
            unwrap_result_invariant(
                u32::try_from(self.entries.len()),
                "whim-runtime: a dict cannot exceed u32::MAX slots",
            )
        };
        let entries = &self.entries;
        match self.index.entry(
            hash,
            |entry| {
                entry.matches(hash)
                    && slot_matches_short_string(
                        // SAFETY: the surrounding invariant keeps this index in bounds.
                        unsafe { entries.get_unchecked(entry.slot as usize) },
                        key,
                    )
            },
            |entry| {
                slot_hash(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { entries.get_unchecked(entry.slot as usize) },
                    hash_state,
                )
            },
        ) {
            Entry::Occupied(occupied) => {
                let position = occupied.get().slot;
                // SAFETY: the surrounding invariant keeps this index in bounds.
                match unsafe { self.entries.get_unchecked_mut(position as usize) } {
                    Slot::Occupied {
                        value: existing, ..
                    } => {
                        let previous = mem::replace(existing, value);
                        self.note_type_check_mutation(position);
                        Some(previous)
                    }
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    Slot::Vacant => unsafe {
                        unreachable_invariant("the index never references a vacant slot")
                    },
                }
            }
            Entry::Vacant(vacant) => {
                vacant.insert(IndexEntry::new(next_position, hash));
                self.entries.push(Slot::Occupied {
                    key: Key::ShortString(key),
                    value,
                });
                self.live += 1;
                self.note_type_check_mutation(next_position);
                None
            }
        }
    }
}
