//! The dict collection: an insertion-ordered map with int and string keys.

use std::iter::Enumerate;
use std::mem;
use std::ptr::NonNull;
use std::slice::Iter;
use std::vec::Vec;

use hashbrown::HashTable;
use hashbrown::hash_table::Entry;

use crate::unreachable_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::array::ArrayTypeCheck;
use crate::value::array::ArrayTypeCheckCache;
use crate::value::array::ArrayTypeCheckId;
use crate::value::hash::HashState;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::CowClone;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;
use crate::value::string::ByteStringObject;
use crate::value::string::short::ShortString;

mod insertion;
pub(crate) mod keys;
mod slots;

use crate::value::dict::keys::Key;
use crate::value::dict::keys::KeyRef;
use crate::value::dict::slots::Slot;
use crate::value::dict::slots::slot_hash;
use crate::value::dict::slots::slot_matches;
use crate::value::dict::slots::slot_matches_ref;
use crate::value::dict::slots::slot_matches_short_string;
use crate::value::dict::slots::slot_matches_string;

pub(crate) struct DictObject {
    hash_state: NonNull<HashState>,
    packed: Option<Vec<Value>>,
    entries: Vec<Slot>,
    index: HashTable<IndexEntry>,
    live: usize,
    type_check: ArrayTypeCheckCache,
}

#[derive(Clone, Copy)]
struct IndexEntry {
    slot: u32,
    fingerprint: u32,
}

impl IndexEntry {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the fingerprint deliberately keeps the low 32 hash bits"
    )]
    #[expect(
        clippy::inline_always,
        reason = "fingerprint creation is part of the dictionary insertion fast path"
    )]
    #[inline(always)]
    const fn new(slot: u32, hash: u64) -> Self {
        Self {
            slot,
            fingerprint: hash as u32,
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the fingerprint deliberately compares the low 32 hash bits"
    )]
    #[expect(
        clippy::inline_always,
        reason = "fingerprint checks run inside every indexed dictionary probe"
    )]
    #[inline(always)]
    const fn matches(self, hash: u64) -> bool {
        self.fingerprint == hash as u32
    }
}

/// An insertion-order iterator over packed or indexed dict storage.
pub(crate) struct DictIter<'a> {
    inner: DictIterInner<'a>,
}

enum DictIterInner<'a> {
    Packed(Enumerate<Iter<'a, Value>>),
    Indexed(Iter<'a, Slot>),
}

impl<'a> Iterator for DictIter<'a> {
    type Item = (KeyRef<'a>, &'a Value);

    #[expect(
        clippy::cast_possible_wrap,
        reason = "packed dictionaries cannot exceed the u32 slot limit"
    )]
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            DictIterInner::Packed(values) => {
                let (index, value) = values.next()?;
                Some((KeyRef::Int(index as i64), value))
            }
            DictIterInner::Indexed(slots) => loop {
                match slots.next()? {
                    Slot::Occupied { key, value, .. } => {
                        return Some((KeyRef::from(key), value));
                    }
                    Slot::Vacant => {}
                }
            },
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "dictionary storage is capped at u32 slots"
)]
#[expect(
    clippy::inline_always,
    reason = "these methods form the VM dictionary fast path"
)]
impl DictObject {
    #[must_use]
    pub(crate) fn new(heap: &Heap) -> ManagedRef<Self> {
        ManagedRef::new_in(
            heap,
            Self {
                hash_state: NonNull::from(heap.hash_state()),
                packed: Some(Vec::new()),
                entries: Vec::new(),
                index: HashTable::new(),
                live: 0,
                type_check: ArrayTypeCheckCache::new(),
            },
        )
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.packed.as_ref().map_or(self.live, Vec::len)
    }

    /// Returns the hash state owned by the heap that outlives this dict.
    #[inline(always)]
    const fn hash_state(&self) -> &HashState {
        // SAFETY: the heap-owned hash state outlives the dict.
        unsafe { self.hash_state.as_ref() }
    }

    #[must_use]
    pub(crate) fn packed_values(&self) -> Option<&[Value]> {
        self.packed.as_deref()
    }

    /// The mutable packed storage for a pinned writer. Pin writes bypass
    /// per-slot mutation notes, so the structural type-check cache is
    /// conservatively invalidated up front.
    pub(crate) fn packed_values_for_pin(&mut self) -> Option<&mut [Value]> {
        self.packed.as_ref()?;
        self.type_check.invalidate();
        self.packed.as_deref_mut()
    }

    /// Reserves capacity for `additional` upcoming keyed inserts, leaving
    /// the packed representation only when the dict is empty.
    pub(crate) fn reserve_for_build(&mut self, additional: usize) {
        if matches!(self.packed.as_deref(), Some([])) {
            self.materialize_index();
        }
        if self.packed.is_none() {
            self.entries.reserve(additional);
            let hash_state = self.hash_state;
            // SAFETY: the heap-owned hash state outlives the dict.
            let hash_state = unsafe { hash_state.as_ref() };
            let entries = &self.entries;
            self.index.reserve(additional, |entry| {
                slot_hash(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { entries.get_unchecked(entry.slot as usize) },
                    hash_state,
                )
            });
        }
    }

    pub(crate) fn reserve_hint(&mut self, additional: usize) {
        if let Some(values) = &mut self.packed {
            values.reserve(additional);
            return;
        }

        self.entries.reserve(additional);
        let hash_state = self.hash_state;
        // SAFETY: the heap-owned hash state outlives the dict.
        let hash_state = unsafe { hash_state.as_ref() };
        let entries = &self.entries;
        self.index.reserve(additional, |entry| {
            slot_hash(
                // SAFETY: the surrounding invariant keeps this index in bounds.
                unsafe { entries.get_unchecked(entry.slot as usize) },
                hash_state,
            )
        });
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub(crate) fn get(&self, key: &Key) -> Option<&Value> {
        self.get_ref(KeyRef::from(key))
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get_int(&self, key: i64) -> Option<&Value> {
        if let Some(values) = &self.packed {
            return values.get(usize::try_from(key).ok()?);
        }

        let key = KeyRef::Int(key);
        let hash = key.hash64(self.hash_state());
        let position = self.index.find(hash, |entry| {
            entry.matches(hash)
                && slot_matches_ref(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { self.entries.get_unchecked(entry.slot as usize) },
                    key,
                )
        })?;

        // SAFETY: the surrounding invariant keeps this index in bounds.
        match unsafe { self.entries.get_unchecked(position.slot as usize) } {
            Slot::Occupied { value, .. } => Some(value),
            // SAFETY: the surrounding invariant makes this path unreachable.
            Slot::Vacant => unsafe {
                unreachable_invariant("the index never references a vacant slot")
            },
        }
    }

    #[must_use]
    pub(crate) fn get_ref(&self, key: KeyRef<'_>) -> Option<&Value> {
        if let Some(values) = &self.packed {
            let KeyRef::Int(position) = key else {
                return None;
            };
            return values.get(usize::try_from(position).ok()?);
        }

        let hash = key.hash64(self.hash_state());
        let position = self.index.find(hash, |entry| {
            entry.matches(hash)
                && slot_matches_ref(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { self.entries.get_unchecked(entry.slot as usize) },
                    key,
                )
        })?;

        // SAFETY: the surrounding invariant keeps this index in bounds.
        match unsafe { self.entries.get_unchecked(position.slot as usize) } {
            Slot::Occupied { value, .. } => Some(value),
            // SAFETY: the surrounding invariant makes this path unreachable.
            Slot::Vacant => unsafe {
                unreachable_invariant("the index never references a vacant slot")
            },
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get_string(&self, key: &ManagedRef<ByteStringObject>) -> Option<&Value> {
        if self.packed.is_some() {
            return None;
        }
        let hash = key.hash64(self.hash_state());
        let position = self.index.find(hash, |entry| {
            entry.matches(hash)
                && slot_matches_string(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { self.entries.get_unchecked(entry.slot as usize) },
                    key,
                )
        })?;
        // SAFETY: the surrounding invariant keeps this index in bounds.
        match unsafe { self.entries.get_unchecked(position.slot as usize) } {
            Slot::Occupied { value, .. } => Some(value),
            // SAFETY: the surrounding invariant makes this path unreachable.
            Slot::Vacant => unsafe {
                unreachable_invariant("the index never references a vacant slot")
            },
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn get_short_string(&self, key: ShortString) -> Option<&Value> {
        if self.packed.is_some() {
            return None;
        }
        let hash = key.hash64(self.hash_state());
        let position = self.index.find(hash, |entry| {
            entry.matches(hash)
                && slot_matches_short_string(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { self.entries.get_unchecked(entry.slot as usize) },
                    key,
                )
        })?;
        // SAFETY: the surrounding invariant keeps this index in bounds.
        match unsafe { self.entries.get_unchecked(position.slot as usize) } {
            Slot::Occupied { value, .. } => Some(value),
            // SAFETY: the surrounding invariant makes this path unreachable.
            Slot::Vacant => unsafe {
                unreachable_invariant("the index never references a vacant slot")
            },
        }
    }

    pub(crate) fn get_mut_ref(&mut self, key: KeyRef<'_>) -> Option<&mut Value> {
        if self.packed.is_some() {
            let KeyRef::Int(position) = key else {
                return None;
            };
            let position = usize::try_from(position).ok()?;
            if position >= self.packed.as_ref()?.len() {
                return None;
            }
            self.note_type_check_mutation(position as u32);
            // SAFETY: the surrounding invariant keeps this index in bounds.
            return Some(unsafe { self.packed.as_mut()?.get_unchecked_mut(position) });
        }

        let hash = key.hash64(self.hash_state());
        let position = self.index.find(hash, |entry| {
            entry.matches(hash)
                && slot_matches_ref(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { self.entries.get_unchecked(entry.slot as usize) },
                    key,
                )
        })?;
        let position = position.slot;
        self.note_type_check_mutation(position);
        // SAFETY: the surrounding invariant keeps this index in bounds.
        match unsafe { self.entries.get_unchecked_mut(position as usize) } {
            Slot::Occupied { value, .. } => Some(value),
            // SAFETY: the surrounding invariant makes this path unreachable.
            Slot::Vacant => unsafe {
                unreachable_invariant("the index never references a vacant slot")
            },
        }
    }

    #[inline(always)]
    pub(crate) fn get_int_mut_string(
        &mut self,
        key: &ManagedRef<ByteStringObject>,
    ) -> Option<&mut i64> {
        if self.packed.is_some() {
            return None;
        }
        let hash = key.hash64(self.hash_state());
        let position = self.index.find(hash, |entry| {
            entry.matches(hash)
                && slot_matches_string(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { self.entries.get_unchecked(entry.slot as usize) },
                    key,
                )
        })?;
        // SAFETY: the surrounding invariant keeps this index in bounds.
        match unsafe { self.entries.get_unchecked_mut(position.slot as usize) } {
            Slot::Occupied { value, .. } => value.as_int_mut(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            Slot::Vacant => unsafe {
                unreachable_invariant("the index never references a vacant slot")
            },
        }
    }

    #[inline(always)]
    pub(crate) fn get_int_mut_short_string(&mut self, key: ShortString) -> Option<&mut i64> {
        if self.packed.is_some() {
            return None;
        }
        let hash = key.hash64(self.hash_state());
        let position = self.index.find(hash, |entry| {
            entry.matches(hash)
                && slot_matches_short_string(
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    unsafe { self.entries.get_unchecked(entry.slot as usize) },
                    key,
                )
        })?;
        // SAFETY: the surrounding invariant keeps this index in bounds.
        match unsafe { self.entries.get_unchecked_mut(position.slot as usize) } {
            Slot::Occupied { value, .. } => value.as_int_mut(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            Slot::Vacant => unsafe {
                unreachable_invariant("the index never references a vacant slot")
            },
        }
    }

    pub(crate) fn insert(&mut self, key: Key, value: Value) -> Option<Value> {
        match key {
            Key::Int(key) => self.insert_int(key, value),
            key => self.insert_indexed(key, value),
        }
    }

    /// Inserts an optimizer-proven integer without constructing and matching
    /// the polymorphic owned-key representation on the packed hot path.
    #[inline(always)]
    pub(crate) fn insert_int(&mut self, key: i64, value: Value) -> Option<Value> {
        if let Some(values) = &mut self.packed
            && let Ok(position) = usize::try_from(key)
        {
            if position < values.len() {
                let previous = mem::replace(&mut values[position], value);
                self.note_type_check_mutation(position as u32);
                return Some(previous);
            }

            if position == values.len() {
                // SAFETY: a dict's slot count never exceeds u32::MAX, so the position fits u32.
                let position = unsafe {
                    unwrap_result_invariant(
                        u32::try_from(position),
                        "whim-runtime: a dict cannot exceed u32::MAX slots",
                    )
                };
                values.push(value);
                self.note_type_check_mutation(position);
                return None;
            }
        }

        self.insert_indexed(Key::Int(key), value)
    }

    fn insert_indexed(&mut self, key: Key, value: Value) -> Option<Value> {
        self.materialize_index();
        let hash_state = self.hash_state;
        // SAFETY: the heap-owned hash state outlives the dict.
        let hash_state = unsafe { hash_state.as_ref() };
        let hash = key.hash64(hash_state);
        // SAFETY: a dict's slot count never exceeds u32::MAX, so the entry count fits u32.
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
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    && slot_matches(unsafe { entries.get_unchecked(entry.slot as usize) }, &key)
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
                match &mut self.entries[position as usize] {
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
                self.entries.push(Slot::Occupied { key, value });
                self.live += 1;
                self.note_type_check_mutation(next_position);

                None
            }
        }
    }

    pub(crate) fn remove(&mut self, key: &Key) -> Option<Value> {
        if let Some(values) = &mut self.packed {
            let Key::Int(key) = key else {
                return None;
            };
            let Ok(position) = usize::try_from(*key) else {
                return None;
            };
            if position >= values.len() {
                return None;
            }
            if position + 1 == values.len() {
                let value = values.pop();
                self.type_check.note_removal();
                return value;
            }
        }

        self.materialize_index();
        let hash = key.hash64(self.hash_state());
        let entries = &self.entries;
        let Ok(entry) = self.index.find_entry(hash, |entry| {
            entry.matches(hash)
                // SAFETY: the surrounding invariant keeps this index in bounds.
                && slot_matches(unsafe { entries.get_unchecked(entry.slot as usize) }, key)
        }) else {
            return None;
        };
        let (position, _) = entry.remove();
        let position = position.slot;
        let slot = mem::replace(&mut self.entries[position as usize], Slot::Vacant);
        let Slot::Occupied { value, .. } = slot else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the index never references a vacant slot") };
        };
        self.live -= 1;
        self.type_check.note_removal();
        self.compact_if_sparse();
        Some(value)
    }

    /// Iterates the entries in insertion order.
    #[expect(
        clippy::option_if_let_else,
        reason = "the storage variants are clearer as an explicit match"
    )]
    pub(crate) fn iter(&self) -> DictIter<'_> {
        let inner = match &self.packed {
            Some(values) => DictIterInner::Packed(values.iter().enumerate()),
            None => DictIterInner::Indexed(self.entries.iter()),
        };
        DictIter { inner }
    }

    #[must_use]
    #[expect(
        clippy::cast_possible_wrap,
        reason = "dictionary storage is capped at u32 slots"
    )]
    #[expect(
        clippy::option_option,
        reason = "the outer option marks bounds and the inner option marks a vacant slot"
    )]
    pub(crate) fn entry_at_slot(&self, slot: usize) -> Option<Option<(KeyRef<'_>, &Value)>> {
        if let Some(values) = &self.packed {
            return values
                .get(slot)
                .map(|value| Some((KeyRef::Int(slot as i64), value)));
        }

        match self.entries.get(slot)? {
            Slot::Occupied { key, value, .. } => Some(Some((KeyRef::from(key), value))),
            Slot::Vacant => Some(None),
        }
    }

    /// The cached check state for `id`; a different descriptor has no cache.
    #[must_use]
    pub(crate) const fn type_check(&self, id: ArrayTypeCheckId) -> ArrayTypeCheck {
        self.type_check.get(id)
    }

    pub(crate) fn mark_type_checked(&self, id: ArrayTypeCheckId) {
        self.type_check.mark_checked(id);
    }

    fn note_type_check_mutation(&self, slot: u32) {
        self.type_check.note_mutation(slot as usize);
    }

    #[inline(always)]
    fn materialize_index(&mut self) {
        let Some(values) = self.packed.take() else {
            return;
        };

        self.live = values.len();
        self.entries.reserve(values.len());
        let hash_state = self.hash_state;
        // SAFETY: the heap-owned hash state outlives the dict.
        let hash_state = unsafe { hash_state.as_ref() };
        let entries = &self.entries;
        self.index.reserve(values.len(), |entry| {
            slot_hash(
                // SAFETY: the surrounding invariant keeps this index in bounds.
                unsafe { entries.get_unchecked(entry.slot as usize) },
                hash_state,
            )
        });
        for (position, value) in values.into_iter().enumerate() {
            // SAFETY: a dict's slot count never exceeds u32::MAX, so the position fits u32.
            let position = unsafe {
                unwrap_result_invariant(
                    u32::try_from(position),
                    "whim-runtime: a dict cannot exceed u32::MAX slots",
                )
            };
            let key = Key::Int(i64::from(position));
            let hash = key.hash64(hash_state);
            self.entries.push(Slot::Occupied { key, value });
            let entries = &self.entries;
            self.index
                .insert_unique(hash, IndexEntry::new(position, hash), |entry| {
                    slot_hash(
                        // SAFETY: the surrounding invariant keeps this index in bounds.
                        unsafe { entries.get_unchecked(entry.slot as usize) },
                        hash_state,
                    )
                });
        }
    }

    /// Rebuilds the slots and the index without tombstones, preserving entry
    /// order, once tombstones outnumber live entries.
    fn compact_if_sparse(&mut self) {
        if self.entries.len() - self.live <= self.live {
            return;
        }
        let old = mem::replace(&mut self.entries, Vec::with_capacity(self.live));
        self.index.clear();
        let hash_state = self.hash_state;
        // SAFETY: the heap-owned hash state outlives the dict.
        let hash_state = unsafe { hash_state.as_ref() };
        for slot in old {
            if let Slot::Occupied { key, value } = slot {
                // SAFETY: a dict's slot count never exceeds u32::MAX, so the entry count fits u32.
                let position = unsafe {
                    unwrap_result_invariant(
                        u32::try_from(self.entries.len()),
                        "whim-runtime: a dict cannot exceed u32::MAX slots",
                    )
                };
                let hash = key.hash64(hash_state);
                self.entries.push(Slot::Occupied { key, value });
                let entries = &self.entries;
                self.index
                    .insert_unique(hash, IndexEntry::new(position, hash), |entry| {
                        slot_hash(
                            // SAFETY: the surrounding invariant keeps this index in bounds.
                            unsafe { entries.get_unchecked(entry.slot as usize) },
                            hash_state,
                        )
                    });
            }
        }
    }
}

impl CowClone for DictObject {
    fn cow_clone(&self) -> Self {
        Self {
            hash_state: self.hash_state,
            packed: self.packed.clone(),
            entries: self.entries.clone(),
            index: self.index.clone(),
            live: self.live,
            type_check: self.type_check.clone(),
        }
    }
}

impl Trace for DictObject {
    fn type_tag() -> TypeTag {
        TypeTag::Dict
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        queue: &mut DropQueue,
        mode: TeardownMode,
    ) {
        self.index = HashTable::new();
        if let Some(values) = self.packed.take() {
            for value in values {
                queue.release_value(value, mode);
            }
        }

        let entries = mem::take(&mut self.entries);
        for slot in entries {
            if let Slot::Occupied { key, value, .. } = slot {
                if let Key::String(string) = key {
                    queue.release_child(string, mode);
                }
                queue.release_value(value, mode);
            }
        }
    }

    fn visit_children(&self, _allocation: NonNull<HeapBox<()>>, visitor: &mut TraceVisitor<'_>) {
        if let Some(values) = &self.packed {
            for value in values {
                if let Some(child) = value.collectable_box() {
                    visitor.visit(child);
                }
            }
            return;
        }

        for slot in &self.entries {
            if let Slot::Occupied { value, .. } = slot
                && let Some(child) = value.collectable_box()
            {
                visitor.visit(child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::value::Value;
    use crate::value::dict::DictObject;
    use crate::value::dict::keys::Key;
    use crate::value::heap::Heap;
    use crate::value::heap::handle::ManagedRef;

    fn packed_dict(heap: &Heap) -> ManagedRef<DictObject> {
        let mut dict = DictObject::new(heap);
        drop(dict.make_mut().insert_int(0, Value::int(10)));
        drop(dict.make_mut().insert_int(1, Value::int(20)));
        drop(dict.make_mut().insert_int(2, Value::int(30)));
        dict
    }

    #[test]
    fn removing_the_packed_tail_preserves_packed_storage() {
        let heap = Heap::new();
        let mut dict = packed_dict(&heap);

        let removed = dict.make_mut().remove(&Key::Int(2));

        assert_eq!(removed.and_then(|value| value.as_int()), Some(30));
        assert_eq!(dict.packed_values().map(<[Value]>::len), Some(2));
    }

    #[test]
    fn missing_packed_keys_do_not_materialize_storage() {
        let heap = Heap::new();
        let mut dict = packed_dict(&heap);

        assert!(dict.make_mut().remove(&Key::Int(3)).is_none());
        assert!(dict.make_mut().remove(&Key::Bool(false)).is_none());
        assert_eq!(dict.packed_values().map(<[Value]>::len), Some(3));
    }

    #[test]
    fn removing_a_packed_hole_materializes_storage() {
        let heap = Heap::new();
        let mut dict = packed_dict(&heap);

        let removed = dict.make_mut().remove(&Key::Int(1));

        assert_eq!(removed.and_then(|value| value.as_int()), Some(20));
        assert!(dict.packed_values().is_none());
    }
}
