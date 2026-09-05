//! Weak references and weak-keyed maps.

use std::cell::Cell;
use std::cell::UnsafeCell;
use std::mem;
use std::ptr::NonNull;

use hashbrown::HashMap;

use crate::value::Value;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;
use crate::value::object::InstanceObject;

pub(crate) struct WeakReference {
    target: Cell<Option<NonNull<HeapBox<InstanceObject>>>>,
}

impl WeakReference {
    /// Allocates a weak reference on its target's heap.
    #[must_use]
    pub(crate) fn new(target: &ManagedRef<InstanceObject>) -> ManagedRef<Self> {
        let heap = target.heap_ref();
        let weak = ManagedRef::new_in(
            heap,
            Self {
                target: Cell::new(Some(target.raw_box())),
            },
        );
        heap.register_weak_dependent(target.raw_box().addr().get(), weak.erased());
        weak
    }

    #[must_use]
    pub(crate) fn upgrade(&self) -> Option<ManagedRef<InstanceObject>> {
        let target = self.target.get()?;
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { target.as_ref() }.header_ref().increment();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        Some(unsafe { ManagedRef::from_raw(target) })
    }

    pub(in crate::value) fn clear_target(&self) {
        self.target.set(None);
    }

    pub(in crate::value) fn target_address(&self) -> Option<usize> {
        self.target.get().map(|target| target.addr().get())
    }
}

impl Trace for WeakReference {
    fn type_tag() -> TypeTag {
        TypeTag::Weak
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        _queue: &DropQueue,
        _mode: TeardownMode,
    ) {
    }
}

/// Weak-keyed values owned by the single-threaded VM.
pub(crate) struct WeakMapObject {
    entries: UnsafeCell<HashMap<usize, Value>>,
}

impl WeakMapObject {
    #[must_use]
    pub(crate) fn new(heap: &Heap) -> ManagedRef<Self> {
        ManagedRef::new_in(
            heap,
            Self {
                entries: UnsafeCell::new(HashMap::new()),
            },
        )
    }

    pub(crate) fn set(
        this: &ManagedRef<Self>,
        key: &ManagedRef<InstanceObject>,
        value: Value,
    ) -> Option<Value> {
        let address = key.raw_box().addr().get();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let entries = unsafe { &mut *this.entries.get() };
        if let Some(previous) = entries.insert(address, value) {
            return Some(previous);
        }

        this.heap_ref()
            .register_weak_dependent(address, this.erased());
        None
    }

    pub(crate) fn remove(
        this: &ManagedRef<Self>,
        key: &ManagedRef<InstanceObject>,
    ) -> Option<Value> {
        let address = key.raw_box().addr().get();
        let value = this.remove_entry_by_address(address)?;
        this.heap_ref()
            .deregister_weak_dependent(address, this.erased());
        Some(value)
    }

    #[must_use]
    pub(crate) fn get(&self, key: &ManagedRef<InstanceObject>) -> Option<Value> {
        let address = key.raw_box().addr().get();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let entries = unsafe { &*self.entries.get() };
        entries.get(&address).cloned()
    }

    #[must_use]
    pub(crate) fn has(&self, key: &ManagedRef<InstanceObject>) -> bool {
        let address = key.raw_box().addr().get();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let entries = unsafe { &*self.entries.get() };
        entries.contains_key(&address)
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { &*self.entries.get() }.len()
    }

    pub(in crate::value) fn remove_entry_by_address(&self, address: usize) -> Option<Value> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let entries = unsafe { &mut *self.entries.get() };
        entries.remove(&address)
    }

    pub(in crate::value) fn key_addresses(&self) -> Vec<usize> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let entries = unsafe { &*self.entries.get() };
        entries.keys().copied().collect()
    }
}

impl Trace for WeakMapObject {
    fn type_tag() -> TypeTag {
        TypeTag::WeakMap
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        queue: &DropQueue,
        mode: TeardownMode,
    ) {
        let entries = mem::take(self.entries.get_mut());
        for value in entries.into_values() {
            queue.release_value(value, mode);
        }
    }

    fn visit_children(&self, _allocation: NonNull<HeapBox<()>>, visitor: &mut TraceVisitor<'_>) {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let entries = unsafe { &*self.entries.get() };
        for value in entries.values() {
            if let Some(child) = value.collectable_box() {
                visitor.visit(child);
            }
        }
    }
}
