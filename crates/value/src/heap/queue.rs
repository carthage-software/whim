//! Iterative teardown for nested heap values.

use std::cell::Cell;
use std::cell::UnsafeCell;
use std::mem;
use std::ptr::NonNull;

use crate::Value;
use crate::heap::DeferredBuiltInDrop;
use crate::heap::Heap;
use crate::heap::bytes::HeapBytes;
use crate::heap::handle::ManagedRef;
use crate::heap::metadata::HeapBox;
use crate::heap::metadata::TeardownMode;
use crate::heap::metadata::Trace;
use crate::heap::metadata::TypeTag;

pub(in crate::heap) struct Erased {
    pub(in crate::heap) box_pointer: NonNull<HeapBox<()>>,
    pub(in crate::heap) tag: TypeTag,
}

/// Child teardown can re-enter this queue; mutable borrows stay inside push/pop.
pub struct DropQueue {
    pending: UnsafeCell<Vec<Erased>>,
    /// One string buffer deferred until child enumeration finishes.
    released_bytes: Cell<Option<HeapBytes>>,
    /// One built-in state drop deferred until child enumeration finishes.
    deferred_built_in: Cell<Option<DeferredBuiltInDrop>>,
}

impl DropQueue {
    pub(in crate::heap) const fn new() -> Self {
        Self {
            pending: UnsafeCell::new(Vec::new()),
            released_bytes: Cell::new(None),
            deferred_built_in: Cell::new(None),
        }
    }

    pub(in crate::heap) fn push(&self, entry: Erased) {
        // SAFETY: this single-threaded borrow ends before any child teardown can re-enter.
        unsafe { &mut *self.pending.get() }.push(entry);
    }

    pub(in crate::heap) fn pop(&self) -> Option<Erased> {
        // SAFETY: this single-threaded borrow ends before any child teardown can re-enter.
        unsafe { &mut *self.pending.get() }.pop()
    }

    #[cfg(test)]
    pub(in crate::heap) fn capacity(&self) -> usize {
        // SAFETY: tests inspect this single-threaded queue between pending-buffer operations.
        unsafe { &*self.pending.get() }.capacity()
    }

    /// Defers built-in state destruction until child enumeration finishes.
    pub(crate) fn defer_built_in_drop(&self, data: NonNull<()>, drop_data: unsafe fn(NonNull<()>)) {
        debug_assert!(
            self.deferred_built_in.get().is_none(),
            "an object defers one built-in state chain drop per teardown"
        );
        self.deferred_built_in.set(Some((data, drop_data)));
    }

    pub(in crate::heap) fn take_deferred_built_in(&self) -> Option<DeferredBuiltInDrop> {
        self.deferred_built_in.take()
    }

    pub(crate) fn release_bytes(&self, bytes: HeapBytes) {
        let previous = self.released_bytes.replace(Some(bytes));
        debug_assert!(
            previous.is_none(),
            "a payload defers at most one flat buffer per teardown"
        );
    }

    pub(in crate::heap) fn take_released_bytes(&self) -> Option<HeapBytes> {
        self.released_bytes.take()
    }

    fn release<T: Trace>(&self, child: ManagedRef<T>) {
        let erased = child.erased();
        if child.header().is_immortal() {
            mem::forget(child);
            return;
        }

        let heap_ptr = child.header().heap_ptr().cast::<Heap>();
        mem::forget(child);
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let heap = unsafe { heap_ptr.as_ref() };
        if let Some(entry) = heap.release_reference(erased) {
            self.push(entry);
        }
    }

    /// Releases a child unless cycle collection already decremented it.
    pub fn release_child<T: Trace>(&self, child: ManagedRef<T>, mode: TeardownMode) {
        if mode == TeardownMode::CycleMember && T::type_tag().is_collectable() {
            mem::forget(child);
        } else {
            self.release(child);
        }
    }

    pub fn release_value(&self, value: Value, mode: TeardownMode) {
        if mode == TeardownMode::CycleMember && value.collectable_box().is_some() {
            mem::forget(value);
        } else {
            drop(value);
        }
    }

    /// Releases an object value while avoiding needless cycle work.
    pub fn release_object_value(&self, value: Value, mode: TeardownMode) {
        if !value.is_object() {
            self.release_value(value, mode);
            return;
        }

        // SAFETY: the value's tag proves this projection is valid.
        let object = unsafe { value.into_object_unchecked() };
        if mode != TeardownMode::Full || object.cycle_possible() {
            self.release_child(object, mode);
            return;
        }

        let header = object.header();
        if header.is_immortal() {
            mem::forget(object);
            return;
        }

        let box_pointer = object.erased();
        let remaining = header.decrement();
        mem::forget(object);
        if remaining == 0 {
            self.push(Erased {
                box_pointer,
                tag: TypeTag::Object,
            });
        }
    }
}
