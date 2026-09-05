//! Object instances: class ids, property slots, and inline built-in Rust state.

use std::alloc::Layout;
use std::any::TypeId;
use std::cell::Cell;
use std::ptr;
use std::ptr::NonNull;

use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub(crate) struct ClassId(pub(crate) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub(crate) struct TypeEnvironmentId(pub(crate) u32);

type BuiltInVisitChildren = for<'visit> unsafe fn(NonNull<()>, &mut TraceVisitor<'visit>);

/// The layout, teardown, and tracing operations a built-in class provides.
pub(crate) struct BuiltInHooks {
    pub(crate) state_type: TypeId,
    pub(crate) layout: Layout,
    /// Drops the inline state after its children have been released.
    pub(crate) drop_in_place: unsafe fn(NonNull<()>),
    /// Releases values held by the inline state.
    pub(crate) enqueue_children: Option<unsafe fn(NonNull<()>, &DropQueue, TeardownMode)>,
    /// Visits collectable boxes held by the inline state.
    pub(crate) visit_children: Option<BuiltInVisitChildren>,
}

/// Drops one initialized inline built-in state without freeing its surrounding
/// object allocation.
///
/// # Safety
///
/// `data` must point to a live `T` and this function may be called only once.
pub(crate) unsafe fn drop_built_in_state<T>(data: NonNull<()>) {
    // SAFETY: the source and target ranges are valid.
    unsafe { ptr::drop_in_place(data.cast::<T>().as_ptr()) };
}

unsafe fn drop_built_in_state_chain(data: NonNull<()>) {
    let mut current = Some(data.cast::<BuiltInState>());
    while let Some(built_in) = current {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let built_in = unsafe { built_in.as_ref() };
        current = built_in.next();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { (built_in.hooks.drop_in_place)(built_in.data()) };
    }
}

pub(crate) struct BuiltInState {
    hooks: &'static BuiltInHooks,
    next: Option<NonNull<Self>>,
}

impl BuiltInState {
    #[must_use]
    pub(in crate::value) const fn new(
        hooks: &'static BuiltInHooks,
        next: Option<NonNull<Self>>,
    ) -> Self {
        Self { hooks, next }
    }

    #[must_use]
    pub(in crate::value) fn data_offset(hooks: &BuiltInHooks) -> usize {
        // SAFETY: the surrounding invariant proves this result is successful.
        unsafe {
            unwrap_result_invariant(
                Layout::new::<Self>().extend(hooks.layout),
                "a built-in state layout must fit in addressable memory",
            )
        }
        .1
    }

    #[must_use]
    pub(in crate::value) fn data(&self) -> NonNull<()> {
        let pointer = NonNull::from(self).cast::<u8>();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { pointer.add(Self::data_offset(self.hooks)) }.cast()
    }

    #[must_use]
    fn is<T: 'static>(&self) -> bool {
        self.hooks.state_type == TypeId::of::<T>()
    }

    #[must_use]
    fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        if !self.is::<T>() {
            return None;
        }
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        Some(unsafe { self.data().cast::<T>().as_ref() })
    }

    #[must_use]
    pub(in crate::value) const fn hooks(&self) -> &'static BuiltInHooks {
        self.hooks
    }

    #[must_use]
    pub(in crate::value) const fn next(&self) -> Option<NonNull<Self>> {
        self.next
    }

    pub(in crate::value) const fn set_next(&mut self, next: NonNull<Self>) {
        self.next = Some(next);
    }
}

pub(crate) struct InstanceObject {
    class: ClassId,
    type_environment: TypeEnvironmentId,
    slots: NonNull<Cell<Value>>,
    slot_count: u32,
    slots_are_acyclic: bool,
    cycle_possible: Cell<bool>,
    built_in: Option<NonNull<BuiltInState>>,
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the linker caps class layouts at u32 property slots"
)]
#[expect(
    clippy::inline_always,
    reason = "object allocation and property access are VM hot paths"
)]
impl InstanceObject {
    #[inline(always)]
    const fn slots(&self) -> NonNull<Cell<Value>> {
        self.slots
    }

    #[must_use]
    pub(crate) fn new(heap: &Heap, class: ClassId, slot_count: usize) -> ManagedRef<Self> {
        Self::new_typed(heap, class, slot_count, TypeEnvironmentId::default())
    }

    #[must_use]
    pub(crate) fn new_typed(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        type_environment: TypeEnvironmentId,
    ) -> ManagedRef<Self> {
        Self::new_typed_with_layout(heap, class, slot_count, type_environment, slot_count == 0)
    }

    #[must_use]
    pub(crate) fn new_typed_with_layout(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        type_environment: TypeEnvironmentId,
        slots_are_acyclic: bool,
    ) -> ManagedRef<Self> {
        ManagedRef::new_object_in(heap, slot_count, None, |slots| Self {
            class,
            slots,
            type_environment,
            slot_count: slot_count as u32,
            slots_are_acyclic,
            cycle_possible: Cell::new(false),
            built_in: None,
        })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn new_initialized_typed_with_layout(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        type_environment: TypeEnvironmentId,
        slots_are_acyclic: bool,
        initialize: impl FnMut(usize) -> Value,
    ) -> ManagedRef<Self> {
        ManagedRef::new_initialized_object_in(heap, slot_count, initialize, |slots| Self {
            class,
            slots,
            type_environment,
            slot_count: slot_count as u32,
            slots_are_acyclic,
            cycle_possible: Cell::new(false),
            built_in: None,
        })
    }

    #[must_use]
    pub(crate) fn new_finalizable_typed_with_layout(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        type_environment: TypeEnvironmentId,
        slots_are_acyclic: bool,
    ) -> ManagedRef<Self> {
        ManagedRef::new_finalizable_object_in(heap, slot_count, None, |slots| Self {
            class,
            slots,
            type_environment,
            slot_count: slot_count as u32,
            slots_are_acyclic,
            cycle_possible: Cell::new(true),
            built_in: None,
        })
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn new_initialized_finalizable_typed_with_layout(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        type_environment: TypeEnvironmentId,
        slots_are_acyclic: bool,
        initialize: impl FnMut(usize) -> Value,
    ) -> ManagedRef<Self> {
        ManagedRef::new_initialized_finalizable_object_in(heap, slot_count, initialize, |slots| {
            Self {
                class,
                slots,
                type_environment,
                slot_count: slot_count as u32,
                slots_are_acyclic,
                cycle_possible: Cell::new(true),
                built_in: None,
            }
        })
    }

    pub(crate) fn with_built_in_states<E>(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        hooks: &[&'static BuiltInHooks],
        initialize: impl FnMut(usize, NonNull<()>) -> Result<(), E>,
    ) -> Result<ManagedRef<Self>, E> {
        Self::with_built_in_states_typed(
            heap,
            class,
            slot_count,
            TypeEnvironmentId::default(),
            hooks,
            initialize,
        )
    }

    pub(crate) fn with_built_in_states_typed<E>(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        type_environment: TypeEnvironmentId,
        hooks: &[&'static BuiltInHooks],
        initialize: impl FnMut(usize, NonNull<()>) -> Result<(), E>,
    ) -> Result<ManagedRef<Self>, E> {
        ManagedRef::new_built_in_states_object_in(
            heap,
            slot_count,
            None,
            hooks,
            initialize,
            |slots, built_in| Self {
                class,
                slots,
                type_environment,
                slot_count: slot_count as u32,
                slots_are_acyclic: false,
                cycle_possible: Cell::new(true),
                built_in: Some(built_in),
            },
        )
    }

    pub(crate) fn with_finalizable_built_in_states_typed<E>(
        heap: &Heap,
        class: ClassId,
        slot_count: usize,
        type_environment: TypeEnvironmentId,
        hooks: &[&'static BuiltInHooks],
        initialize: impl FnMut(usize, NonNull<()>) -> Result<(), E>,
    ) -> Result<ManagedRef<Self>, E> {
        ManagedRef::new_finalizable_built_in_states_object_in(
            heap,
            slot_count,
            None,
            hooks,
            initialize,
            |slots, built_in| Self {
                class,
                slots,
                type_environment,
                slot_count: slot_count as u32,
                slots_are_acyclic: false,
                cycle_possible: Cell::new(true),
                built_in: Some(built_in),
            },
        )
    }

    #[must_use]
    pub(crate) const fn class(&self) -> ClassId {
        self.class
    }

    #[must_use]
    pub(crate) const fn type_environment(&self) -> TypeEnvironmentId {
        self.type_environment
    }

    #[must_use]
    pub(crate) const fn slot_count(&self) -> usize {
        self.slot_count as usize
    }

    /// Whether the slot at `index` has never been written.
    #[must_use]
    #[inline(never)]
    pub(crate) fn slot_is_uninitialized(&self, index: usize) -> bool {
        let slot_count = self.slot_count();
        if index >= slot_count {
            slot_out_of_range(index, slot_count);
        }
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { &*self.slots().add(index).as_ref().as_ptr() }.is_uninitialized()
    }

    #[must_use]
    #[inline(never)]
    pub(crate) fn read_slot(&self, index: usize) -> Value {
        let slot_count = self.slot_count();
        if index >= slot_count {
            slot_out_of_range(index, slot_count);
        }
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { &*self.slots().add(index).as_ref().as_ptr() }.clone()
    }

    /// Reads a slot whose index was already proven against this object's
    /// class layout.
    ///
    /// # Safety
    ///
    /// `index` must be smaller than this instance's slot count.
    #[must_use]
    #[inline(always)]
    pub(crate) unsafe fn read_slot_unchecked(&self, index: usize) -> Value {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { &*self.slots().add(index).as_ref().as_ptr() }.clone_inline_scalar()
    }

    /// Reads an integer slot without cloning or checking the proven slot index.
    ///
    /// # Safety
    ///
    /// `index` must be smaller than this instance's slot count.
    #[must_use]
    #[inline(always)]
    pub(crate) unsafe fn read_int_slot_unchecked(&self, index: usize) -> Option<i64> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let value = unsafe { &*self.slots().add(index).as_ref().as_ptr() };
        value
            .newtype_id()
            .is_none()
            .then(|| value.as_int())
            .flatten()
    }

    /// Replaces a proven integer slot without reference tracking.
    ///
    /// # Safety
    ///
    /// `index` must be smaller than this instance's slot count and its current
    /// value must be an int.
    #[inline(always)]
    pub(crate) unsafe fn write_int_slot_unchecked(&self, index: usize, value: i64) {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let slot = unsafe { self.slots().add(index).as_ref() };
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        debug_assert!(unsafe { &*slot.as_ptr() }.is_int());
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { slot.as_ptr().write(Value::int(value)) };
    }

    #[inline(never)]
    pub(crate) fn write_slot(&self, index: usize, value: Value) -> Value {
        let slot_count = self.slot_count();
        if index >= slot_count {
            slot_out_of_range(index, slot_count);
        }
        self.note_cycle_capable_write(&value, false);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { self.slots().add(index).as_ref() }.replace(value)
    }

    /// Writes a proven slot while recording whether the receiver was uniquely
    /// owned before the write.
    ///
    /// # Safety
    ///
    /// `index` must be smaller than this instance's slot count.
    pub(crate) unsafe fn write_slot_unchecked_with_unique_receiver(
        &self,
        index: usize,
        value: Value,
        receiver_was_unique: bool,
    ) -> Value {
        self.note_cycle_capable_write(&value, receiver_was_unique);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { self.slots().add(index).as_ref() }.replace(value)
    }

    /// # Safety
    ///
    /// `index` must be smaller than this instance's slot count, and user code
    /// must not yet be able to observe the instance.
    pub(crate) unsafe fn write_fresh_slot_unchecked(&self, index: usize, value: Value) {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let slot = unsafe { self.slots().add(index).as_ref() };
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        if unsafe { &*slot.as_ptr() }.is_uninitialized() {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            unsafe { slot.as_ptr().write(value) };
        } else {
            drop(slot.replace(value));
        }
    }

    fn note_cycle_capable_write(&self, value: &Value, receiver_was_unique: bool) {
        if !self.slots_are_acyclic && !receiver_was_unique && value.collectable_box().is_some() {
            self.cycle_possible.set(true);
        }
    }

    /// Whether this instance may participate in a cycle.
    pub(crate) const fn cycle_possible(&self) -> bool {
        self.cycle_possible.get()
    }

    /// Mutates one property slot without cloning its value out of the object.
    /// The callback must not re-enter Whim or otherwise access this instance.
    #[inline(never)]
    pub(crate) fn mutate_slot<R>(&self, index: usize, callback: impl FnOnce(&mut Value) -> R) -> R {
        let slot_count = self.slot_count();
        if index >= slot_count {
            slot_out_of_range(index, slot_count);
        }
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        callback(unsafe { &mut *self.slots().add(index).as_ref().as_ptr() })
    }

    #[must_use]
    pub(crate) const fn has_built_in(&self) -> bool {
        self.built_in.is_some()
    }

    #[must_use]
    pub(crate) fn built_in_ref<T: 'static>(&self) -> Option<&T> {
        let mut current = self.built_in;
        while let Some(built_in) = current {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            let built_in = unsafe { built_in.as_ref() };
            if let Some(state) = built_in.downcast_ref::<T>() {
                return Some(state);
            }
            current = built_in.next();
        }
        None
    }

    /// The first inline built-in state header, for allocation-layout recovery.
    #[must_use]
    pub(in crate::value) const fn built_in_state_head(&self) -> Option<NonNull<BuiltInState>> {
        self.built_in
    }

    /// # Safety
    ///
    /// `index` must be smaller than the fixed slot count and each slot may be
    /// taken at most once.
    pub(crate) const unsafe fn take_slot_unchecked(&self, index: usize) -> Value {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { self.slots().add(index).as_ref() }.replace(Value::uninitialized())
    }
}

/// Reports an invalid property slot outside the access hot path.
#[cold]
#[inline(never)]
fn slot_out_of_range(index: usize, count: usize) -> ! {
    panic!("whim-runtime: an instance slot index is out of range: {index} of {count}");
}

impl Trace for InstanceObject {
    fn type_tag() -> TypeTag {
        TypeTag::Object
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        queue: &DropQueue,
        mode: TeardownMode,
    ) {
        for index in 0..self.slot_count() {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            let value = unsafe { self.slots().add(index).as_ref() }.replace(Value::uninitialized());
            queue.release_object_value(value, mode);
        }
        let head = self.built_in.take();
        let mut current = head;
        while let Some(built_in) = current {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            let built_in = unsafe { built_in.as_ref() };
            current = built_in.next();
            let data = built_in.data();
            if let Some(enqueue) = built_in.hooks.enqueue_children {
                // SAFETY: the tag and managed handle prove the payload type and lifetime.
                unsafe { enqueue(data, queue, mode) };
            }
        }
        if let Some(head) = head {
            queue.defer_built_in_drop(head.cast(), drop_built_in_state_chain);
        }
    }

    fn visit_children(&self, _allocation: NonNull<HeapBox<()>>, visitor: &mut TraceVisitor<'_>) {
        for index in 0..self.slot_count() {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            let value = unsafe { &*self.slots().add(index).as_ref().as_ptr() };
            if let Some(child) = value.collectable_box() {
                visitor.visit(child);
            }
        }
        let mut current = self.built_in;
        while let Some(built_in) = current {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            let built_in = unsafe { built_in.as_ref() };
            current = built_in.next();
            if let Some(visit_hook) = built_in.hooks.visit_children {
                // SAFETY: the tag and managed handle prove the payload type and lifetime.
                unsafe { visit_hook(built_in.data(), visitor) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::value::Value;
    use crate::value::heap::Heap;
    use crate::value::object::ClassId;
    use crate::value::object::InstanceObject;
    use crate::value::object::TypeEnvironmentId;
    use crate::value::string::ByteStringObject;

    #[test]
    fn fresh_writes_release_a_runtime_default() {
        let heap = Heap::new();
        let previous = ByteStringObject::from_bytes(&heap, b"runtime default");
        let object = InstanceObject::new_initialized_typed_with_layout(
            &heap,
            ClassId(0),
            1,
            TypeEnvironmentId::default(),
            true,
            |_| Value::string(previous.clone()),
        );
        assert!(previous.has_other_strong_references());

        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { object.write_fresh_slot_unchecked(0, Value::int(42)) };

        assert!(previous.is_unique());
        assert_eq!(object.read_slot(0).as_int(), Some(42));
    }

    #[test]
    fn fresh_writes_initialize_an_empty_slot() {
        let heap = Heap::new();
        let object = InstanceObject::new(&heap, ClassId(0), 1);

        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { object.write_fresh_slot_unchecked(0, Value::int(42)) };

        assert_eq!(object.read_slot(0).as_int(), Some(42));
    }
}
