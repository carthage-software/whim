//! The vec collection: a growable, hole-free vector of values.

use std::mem;
use std::ptr::NonNull;
use std::slice;

use crate::value::Value;
use crate::value::array::ArrayTypeCheck;
use crate::value::array::ArrayTypeCheckCache;
use crate::value::array::ArrayTypeCheckId;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::CowClone;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;

pub(crate) struct VecObject {
    elements: Vec<Value>,
    type_check: ArrayTypeCheckCache,
}

impl VecObject {
    #[must_use]
    pub(crate) fn new(heap: &Heap) -> ManagedRef<Self> {
        ManagedRef::new_in(
            heap,
            Self {
                elements: Vec::new(),
                type_check: ArrayTypeCheckCache::new(),
            },
        )
    }

    #[must_use]
    pub(crate) fn with_elements(
        heap: &Heap,
        elements: impl IntoIterator<Item = Value>,
    ) -> ManagedRef<Self> {
        let storage = elements.into_iter().collect();
        ManagedRef::new_in(
            heap,
            Self {
                elements: storage,
                type_check: ArrayTypeCheckCache::new(),
            },
        )
    }

    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.elements.len()
    }

    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    #[must_use]
    pub(crate) fn get(&self, index: usize) -> Option<&Value> {
        self.elements.get(index)
    }

    /// Returns the element without checking that `index` is in bounds.
    ///
    /// # Safety
    ///
    /// `index` must be less than [`Self::len`].
    #[must_use]
    pub(crate) unsafe fn get_unchecked(&self, index: usize) -> &Value {
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { self.elements.get_unchecked(index) }
    }

    pub(crate) fn set(&mut self, index: usize, value: Value) -> Option<Value> {
        let slot = self.elements.get_mut(index)?;
        let previous = mem::replace(slot, value);
        self.type_check.note_mutation(index);
        Some(previous)
    }

    #[inline]
    pub(crate) fn push(&mut self, value: Value) {
        let index = self.elements.len();
        self.elements.push(value);
        self.type_check.note_mutation(index);
    }

    #[expect(
        clippy::inline_always,
        reason = "the VM uses this after proving the cache is already invalid"
    )]
    #[inline(always)]
    pub(crate) fn push_after_type_check_invalidation(&mut self, value: Value) {
        self.elements.push(value);
    }

    pub(crate) fn reserve_hint(&mut self, additional: usize) {
        self.elements.reserve(additional);
    }

    pub(crate) fn remove(&mut self, index: usize) -> Option<Value> {
        if index < self.elements.len() {
            let value = self.elements.remove(index);
            self.type_check.note_removal();
            Some(value)
        } else {
            None
        }
    }

    pub(crate) fn swap_remove(&mut self, index: usize) -> Option<Value> {
        if index < self.elements.len() {
            let value = self.elements.swap_remove(index);
            self.type_check.note_removal();
            Some(value)
        } else {
            None
        }
    }

    pub(crate) fn remove_first(&mut self) -> Option<Value> {
        self.remove(0)
    }

    pub(crate) fn remove_last(&mut self) -> Option<Value> {
        let value = self.elements.pop()?;
        self.type_check.note_removal();
        Some(value)
    }

    pub(crate) fn iter(&self) -> slice::Iter<'_, Value> {
        self.elements.iter()
    }

    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Value] {
        &self.elements
    }

    #[must_use]
    pub(crate) fn as_mut_slice(&mut self) -> &mut [Value] {
        self.type_check.invalidate();
        &mut self.elements
    }

    #[must_use]
    #[expect(
        clippy::inline_always,
        reason = "array checks query this cache in the VM hot path"
    )]
    #[inline(always)]
    pub(crate) const fn type_check(&self, id: ArrayTypeCheckId) -> ArrayTypeCheck {
        self.type_check.get(id)
    }

    #[expect(
        clippy::inline_always,
        reason = "array checks update this cache in the VM hot path"
    )]
    #[inline(always)]
    pub(crate) fn mark_type_checked(&self, id: ArrayTypeCheckId) {
        self.type_check.mark_checked(id);
    }

    #[expect(
        clippy::inline_always,
        reason = "broad mutations invalidate this cache in the VM hot path"
    )]
    #[inline(always)]
    pub(crate) fn invalidate_type_check(&self) {
        self.type_check.invalidate();
    }
}

impl CowClone for VecObject {
    fn cow_clone(&self) -> Self {
        Self {
            elements: self.elements.clone(),
            type_check: self.type_check.clone(),
        }
    }
}

impl Trace for VecObject {
    fn type_tag() -> TypeTag {
        TypeTag::Vec
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        queue: &mut DropQueue,
        mode: TeardownMode,
    ) {
        let elements = mem::take(&mut self.elements);
        for element in elements {
            queue.release_value(element, mode);
        }
    }

    fn visit_children(&self, _allocation: NonNull<HeapBox<()>>, visitor: &mut TraceVisitor<'_>) {
        for element in &self.elements {
            if let Some(child) = element.collectable_box() {
                visitor.visit(child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::value::Value;
    use crate::value::ValueView;
    use crate::value::array::ArrayTypeCheck;
    use crate::value::array::ArrayTypeCheckCache;
    use crate::value::array::ArrayTypeCheckId;
    use crate::value::heap::Heap;
    use crate::value::heap::metadata::CowClone;
    use crate::value::vec::VecObject;

    fn vector() -> VecObject {
        VecObject {
            elements: vec![Value::int(1), Value::int(2)],
            type_check: ArrayTypeCheckCache::new(),
        }
    }

    const fn id(value: u32) -> ArrayTypeCheckId {
        ArrayTypeCheckId::new(value)
    }

    #[test]
    fn owned_elements_reuse_storage_and_preserve_cow_lifetimes() {
        let heap = Heap::new();
        let child = Value::from_string_bytes(&heap, b"owned vector child with heap storage");
        let mut elements = Vec::with_capacity(16);
        elements.push(child.clone());
        elements.push(Value::int(7));
        let pointer = elements.as_ptr();
        let vector = VecObject::with_elements(&heap, elements);
        assert_eq!(vector.as_slice().as_ptr(), pointer);

        let mut copy = vector.clone();
        drop(copy.make_mut().set(0, Value::int(42)));
        assert_eq!(copy.get(0).and_then(Value::as_int), Some(42));
        assert_eq!(
            vector.get(0).and_then(Value::as_string_bytes),
            child.as_string_bytes()
        );
        drop(copy);
        drop(vector);

        let ValueView::String(string) = child.transparent() else {
            panic!("the long child string must use heap storage");
        };
        assert!(string.is_unique());
    }

    #[test]
    fn mutations_bound_the_cached_check_to_one_slot() {
        let mut vector = vector();
        vector.mark_type_checked(id(7));
        assert_eq!(vector.type_check(id(7)), ArrayTypeCheck::Clean(id(7)));

        drop(vector.set(1, Value::int(3)));
        assert_eq!(
            vector.type_check(id(7)),
            ArrayTypeCheck::Dirty { id: id(7), slot: 1 }
        );

        drop(vector.set(1, Value::int(4)));
        assert_eq!(vector.type_check(id(7)), ArrayTypeCheck::Unknown);

        vector.mark_type_checked(id(7));
        vector.push(Value::int(5));
        assert_eq!(
            vector.type_check(id(7)),
            ArrayTypeCheck::Dirty { id: id(7), slot: 2 }
        );
    }

    #[test]
    fn removals_preserve_clean_checks_but_broad_mutations_invalidate_them() {
        let mut vector = vector();
        vector.mark_type_checked(id(7));
        drop(vector.remove(0));
        assert_eq!(vector.type_check(id(7)), ArrayTypeCheck::Clean(id(7)));

        vector.mark_type_checked(id(7));
        drop(vector.remove_last());
        assert_eq!(vector.type_check(id(7)), ArrayTypeCheck::Clean(id(7)));

        vector.mark_type_checked(id(7));
        let _ = vector.as_mut_slice();
        assert_eq!(vector.type_check(id(7)), ArrayTypeCheck::Unknown);
    }

    #[test]
    fn cache_keys_and_cow_clones_are_independent() {
        let vector = vector();
        vector.mark_type_checked(id(7));
        vector.mark_type_checked(id(8));
        assert_eq!(vector.type_check(id(7)), ArrayTypeCheck::Clean(id(7)));
        assert_eq!(vector.type_check(id(8)), ArrayTypeCheck::Clean(id(8)));
        assert_eq!(vector.type_check(id(9)), ArrayTypeCheck::Unknown);

        let mut copy = vector.cow_clone();
        drop(copy.set(0, Value::int(3)));
        assert_eq!(vector.type_check(id(7)), ArrayTypeCheck::Clean(id(7)));
        assert_eq!(vector.type_check(id(8)), ArrayTypeCheck::Clean(id(8)));
        assert_eq!(
            copy.type_check(id(7)),
            ArrayTypeCheck::Dirty { id: id(7), slot: 0 }
        );
        assert_eq!(copy.type_check(id(8)), ArrayTypeCheck::Unknown);

        copy.mark_type_checked(id(7));
        drop(copy.set(1, Value::int(4)));
        assert_eq!(copy.type_check(id(8)), ArrayTypeCheck::Unknown);
        assert_eq!(
            copy.type_check(id(7)),
            ArrayTypeCheck::Dirty { id: id(7), slot: 1 }
        );
    }
}
