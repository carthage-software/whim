//! Reference-counted handles to heap values.

use std::alloc::Layout;
use std::cell::Cell;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;

use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::heap::Heap;
use crate::value::heap::metadata::CowClone;
use crate::value::heap::metadata::Header;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::object_layout;
use crate::value::object::BuiltInHooks;
use crate::value::object::BuiltInState;
use crate::value::object::InstanceObject;

#[repr(transparent)]
pub(crate) struct ManagedRef<T: Trace>(NonNull<HeapBox<T>>, PhantomData<HeapBox<T>>);

impl<T: Trace> ManagedRef<T> {
    #[must_use]
    pub(crate) fn new_in(heap: &Heap, payload: T) -> Self {
        let boxed = T::allocate_box(heap);
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe {
            boxed.as_ptr().write(HeapBox {
                header: Header::new(T::type_tag(), NonNull::from(heap).cast()),
                payload,
            });
        }

        Self(boxed, PhantomData)
    }

    /// Adopts an owned reference without changing its count.
    ///
    /// # Safety
    ///
    /// `boxed` must point to a live `T` box. The caller must either transfer an
    /// owned reference to the handle or prevent the handle from being dropped.
    #[must_use]
    pub(crate) const unsafe fn from_raw(boxed: NonNull<HeapBox<T>>) -> Self {
        Self(boxed, PhantomData)
    }

    /// Retains a live box and returns the new reference.
    ///
    /// # Safety
    ///
    /// `boxed` must point to a live `T` box.
    #[must_use]
    pub(crate) unsafe fn retain_raw(boxed: NonNull<HeapBox<T>>) -> Self {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let header = unsafe { &boxed.as_ref().header };
        if !header.is_immortal() {
            header.increment();
        }

        Self(boxed, PhantomData)
    }

    #[must_use]
    pub(crate) const fn raw_box(&self) -> NonNull<HeapBox<T>> {
        self.0
    }

    /// The heap that owns this box, recovered from the header back-pointer.
    pub(crate) const fn heap_ref(&self) -> &Heap {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { self.header().heap_ptr().cast::<Heap>().as_ref() }
    }

    /// Whether this is the only reference to a box that is neither immortal
    /// nor interned; both kinds are permanently shared and must not be
    /// mutated.
    #[must_use]
    pub(crate) const fn is_unique(&self) -> bool {
        let header = self.header();
        !header.is_immortal() && !header.is_interned() && header.reference_count() == 1
    }

    #[must_use]
    pub(crate) const fn has_other_strong_references(&self) -> bool {
        let header = self.header();
        !header.is_immortal() && header.reference_count() > 1
    }

    #[must_use]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }

    #[must_use]
    pub(crate) const fn get_mut(&mut self) -> Option<&mut T> {
        if self.is_unique() {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            Some(unsafe { &mut self.0.as_mut().payload })
        } else {
            None
        }
    }

    pub(crate) fn make_mut(&mut self) -> &mut T
    where
        T: CowClone,
    {
        if !self.is_unique() {
            let separated = (**self).cow_clone();
            let fresh = Self::new_in(self.heap_ref(), separated);
            *self = fresh;
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { &mut self.0.as_mut().payload }
    }

    pub(in crate::value::heap) const fn header(&self) -> &Header {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { &self.0.as_ref().header }
    }

    pub(crate) const fn erased(&self) -> NonNull<HeapBox<()>> {
        self.0.cast()
    }

    /// The erased box pointer when the payload participates in cycle
    /// collection, for inline built-in state visit hooks; the mirror of
    /// [`Value::collectable_box`](crate::value::Value::collectable_box) for typed
    /// handles.
    #[must_use]
    pub(crate) fn collectable_box(&self) -> Option<NonNull<HeapBox<()>>> {
        if T::type_tag().is_collectable() {
            Some(self.erased())
        } else {
            None
        }
    }
}

impl ManagedRef<InstanceObject> {
    /// Allocates an object and its fixed property slots in one contiguous
    /// block. The trailing slots avoid a second allocator round trip for
    /// every ordinary language object.
    pub(crate) fn new_object_in(
        heap: &Heap,
        slot_count: usize,
        initial_values: Option<&[Value]>,
        payload: impl FnOnce(NonNull<Cell<Value>>) -> InstanceObject,
    ) -> Self {
        debug_assert!(
            initial_values.is_none_or(|values| values.len() == slot_count),
            "an object template must cover every property slot"
        );

        let layout = object_layout(slot_count, &[]);
        let allocation = heap.allocate_object_box(slot_count, &layout, true);
        let boxed = allocation.cast::<HeapBox<InstanceObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let slots = unsafe { allocation.add(layout.slots_offset) }.cast::<Cell<Value>>();
        for index in 0..slot_count {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            let value = initial_values.map_or_else(Value::uninitialized, |values| unsafe {
                values.get_unchecked(index).clone_inline_scalar()
            });

            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe {
                slots.add(index).as_ptr().write(Cell::new(value));
            }
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe {
            boxed.as_ptr().write(HeapBox {
                header: Header::new(TypeTag::Object, NonNull::from(heap).cast()),
                payload: payload(slots),
            });
        }

        Self(boxed, PhantomData)
    }

    #[inline(always)]
    #[expect(
        clippy::inline_always,
        reason = "ordinary object allocation is a hot VM path"
    )]
    pub(crate) fn new_initialized_object_in(
        heap: &Heap,
        slot_count: usize,
        mut initialize: impl FnMut(usize) -> Value,
        payload: impl FnOnce(NonNull<Cell<Value>>) -> InstanceObject,
    ) -> Self {
        let layout = object_layout(slot_count, &[]);
        let allocation = heap.allocate_object_box(slot_count, &layout, true);
        let boxed = allocation.cast::<HeapBox<InstanceObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let slots = unsafe { allocation.add(layout.slots_offset) }.cast::<Cell<Value>>();
        for index in 0..slot_count {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe {
                slots
                    .add(index)
                    .as_ptr()
                    .write(Cell::new(initialize(index)));
            }
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe {
            boxed.as_ptr().write(HeapBox {
                header: Header::new(TypeTag::Object, NonNull::from(heap).cast()),
                payload: payload(slots),
            });
        }

        Self(boxed, PhantomData)
    }

    pub(crate) fn new_initialized_finalizable_object_in(
        heap: &Heap,
        slot_count: usize,
        mut initialize: impl FnMut(usize) -> Value,
        payload: impl FnOnce(NonNull<Cell<Value>>) -> InstanceObject,
    ) -> Self {
        let layout = object_layout(slot_count, &[]);
        let allocation = heap.allocate_object_box(slot_count, &layout, true);
        let boxed = allocation.cast::<HeapBox<InstanceObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let slots = unsafe { allocation.add(layout.slots_offset) }.cast::<Cell<Value>>();
        for index in 0..slot_count {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe {
                slots
                    .add(index)
                    .as_ptr()
                    .write(Cell::new(initialize(index)));
            }
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe {
            boxed.as_ptr().write(HeapBox {
                header: Header::new(TypeTag::FinalizableObject, NonNull::from(heap).cast()),
                payload: payload(slots),
            });
        }
        heap.register_finalizable(boxed);
        Self(boxed, PhantomData)
    }

    /// Allocates an object whose destructor must run before ordinary teardown.
    pub(crate) fn new_finalizable_object_in(
        heap: &Heap,
        slot_count: usize,
        initial_values: Option<&[Value]>,
        payload: impl FnOnce(NonNull<Cell<Value>>) -> InstanceObject,
    ) -> Self {
        debug_assert!(
            initial_values.is_none_or(|values| values.len() == slot_count),
            "an object template must cover every property slot"
        );

        let layout = object_layout(slot_count, &[]);
        let allocation = heap.allocate_object_box(slot_count, &layout, true);
        let boxed = allocation.cast::<HeapBox<InstanceObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let slots = unsafe { allocation.add(layout.slots_offset) }.cast::<Cell<Value>>();
        for index in 0..slot_count {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            let value = initial_values.map_or_else(Value::uninitialized, |values| unsafe {
                values.get_unchecked(index).clone_inline_scalar()
            });

            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe {
                slots.add(index).as_ptr().write(Cell::new(value));
            }
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe {
            boxed.as_ptr().write(HeapBox {
                header: Header::new(TypeTag::FinalizableObject, NonNull::from(heap).cast()),
                payload: payload(slots),
            });
        }
        heap.register_finalizable(boxed);
        Self(boxed, PhantomData)
    }

    pub(crate) fn new_built_in_states_object_in<E>(
        heap: &Heap,
        slot_count: usize,
        initial_values: Option<&[Value]>,
        hooks: &[&'static BuiltInHooks],
        initialize: impl FnMut(usize, NonNull<()>) -> Result<(), E>,
        payload: impl FnOnce(NonNull<Cell<Value>>, NonNull<BuiltInState>) -> InstanceObject,
    ) -> Result<Self, E> {
        Self::new_built_in_states_object_with_tag(
            heap,
            slot_count,
            initial_values,
            hooks,
            initialize,
            payload,
            TypeTag::Object,
        )
    }

    /// Allocates an inline built-in object whose destructor must run before
    /// ordinary teardown.
    pub(crate) fn new_finalizable_built_in_states_object_in<E>(
        heap: &Heap,
        slot_count: usize,
        initial_values: Option<&[Value]>,
        hooks: &[&'static BuiltInHooks],
        initialize: impl FnMut(usize, NonNull<()>) -> Result<(), E>,
        payload: impl FnOnce(NonNull<Cell<Value>>, NonNull<BuiltInState>) -> InstanceObject,
    ) -> Result<Self, E> {
        Self::new_built_in_states_object_with_tag(
            heap,
            slot_count,
            initial_values,
            hooks,
            initialize,
            payload,
            TypeTag::FinalizableObject,
        )
    }

    fn new_built_in_states_object_with_tag<E>(
        heap: &Heap,
        slot_count: usize,
        initial_values: Option<&[Value]>,
        hooks: &[&'static BuiltInHooks],
        mut initialize: impl FnMut(usize, NonNull<()>) -> Result<(), E>,
        payload: impl FnOnce(NonNull<Cell<Value>>, NonNull<BuiltInState>) -> InstanceObject,
        tag: TypeTag,
    ) -> Result<Self, E> {
        debug_assert!(
            initial_values.is_none_or(|values| values.len() == slot_count),
            "an object template must cover every property slot"
        );
        debug_assert!(
            !hooks.is_empty(),
            "a built-in object carries built-in state"
        );

        let layout = object_layout(slot_count, hooks);
        let allocation = heap.allocate_object_box(slot_count, &layout, false);
        let boxed = allocation.cast::<HeapBox<InstanceObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let slots = unsafe { allocation.add(layout.slots_offset) }.cast::<Cell<Value>>();

        let mut prefix = Layout::new::<HeapBox<InstanceObject>>();
        let mut built_in_head: Option<NonNull<BuiltInState>> = None;
        let mut built_in_tail: Option<NonNull<BuiltInState>> = None;
        for (index, hooks) in hooks.iter().copied().enumerate() {
            // SAFETY: the surrounding invariant proves this result is successful.
            let component = unsafe {
                unwrap_result_invariant(
                    Layout::new::<BuiltInState>().extend(hooks.layout),
                    "the validated built-in state layout remains valid",
                )
            }
            .0;
            // SAFETY: the surrounding invariant proves this result is successful.
            let (extended, header_offset) = unsafe {
                unwrap_result_invariant(
                    prefix.extend(component),
                    "the validated object layout remains valid",
                )
            };
            prefix = extended;

            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let built_in = unsafe { allocation.add(header_offset) }.cast::<BuiltInState>();
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe { built_in.as_ptr().write(BuiltInState::new(hooks, None)) };
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let data = unsafe { built_in.as_ref() }.data();
            if let Err(error) = initialize(index, data) {
                let mut initialized = built_in_head;
                while let Some(state) = initialized {
                    // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                    let state_ref = unsafe { state.as_ref() };
                    initialized = state_ref.next();
                    // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                    unsafe { (state_ref.hooks().drop_in_place)(state_ref.data()) };
                }

                // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                unsafe { heap.deallocate_object_box(boxed, slot_count, layout, false) };
                return Err(error);
            }

            if let Some(mut tail) = built_in_tail {
                // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                unsafe { tail.as_mut() }.set_next(built_in);
            } else {
                built_in_head = Some(built_in);
            }

            built_in_tail = Some(built_in);
        }

        // SAFETY: the surrounding invariant proves this option contains a value.
        let built_in = unsafe {
            unwrap_option_invariant(built_in_head, "a built-in object carries built-in state")
        };
        for index in 0..slot_count {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            let value = initial_values.map_or_else(Value::uninitialized, |values| unsafe {
                values.get_unchecked(index).clone_inline_scalar()
            });

            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe {
                slots.add(index).as_ptr().write(Cell::new(value));
            }
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe {
            boxed.as_ptr().write(HeapBox {
                header: Header::new(tag, NonNull::from(heap).cast()),
                payload: payload(slots, built_in),
            });
        }

        if tag == TypeTag::FinalizableObject {
            heap.register_finalizable(boxed);
        }

        Ok(Self(boxed, PhantomData))
    }
}

impl<T: Trace> Deref for ManagedRef<T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { &self.0.as_ref().payload }
    }
}

impl<T: Trace> Clone for ManagedRef<T> {
    fn clone(&self) -> Self {
        let header = self.header();
        if !header.is_immortal() {
            header.increment();
        }

        Self(self.0, PhantomData)
    }
}

impl<T: Trace> Drop for ManagedRef<T> {
    fn drop(&mut self) {
        if self.header().is_immortal() {
            return;
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let heap = unsafe { self.header().heap_ptr().cast::<Heap>().as_ref() };
        heap.release_erased(self.erased());
    }
}
