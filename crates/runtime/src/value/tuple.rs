//! The tuple: an immutable, fixed-arity sequence of values.

use std::alloc::Layout;
use std::cell::Cell;
use std::ptr::NonNull;
use std::slice;

use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::array::ArrayTypeCheck;
use crate::value::array::ArrayTypeCheckId;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::Header;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;

pub(crate) struct TupleObject;

pub(crate) fn tuple_layout(len: usize) -> Layout {
    if len > 12 {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a tuple has at most twelve elements") }
    }

    let size = size_of::<HeapBox<TupleObject>>()
        + len * size_of::<Value>()
        + size_of::<Cell<Option<ArrayTypeCheckId>>>();
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            Layout::from_size_align(size, align_of::<HeapBox<TupleObject>>()),
            "a tuple layout is always valid",
        )
    }
}

impl TupleObject {
    #[must_use]
    pub(crate) fn with_pair(heap: &Heap, first: Value, second: Value) -> ManagedRef<Self> {
        let boxed = heap.allocate_tuple_box(2);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let destination = unsafe { boxed.as_ptr().add(1).cast::<Value>() };
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            destination.write(first);
            destination.add(1).write(second);
            destination
                .add(2)
                .cast::<Cell<Option<ArrayTypeCheckId>>>()
                .write(Cell::new(None));
        }
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { ManagedRef::from_raw(boxed) }
    }

    #[must_use]
    pub(crate) fn with_elements(
        heap: &Heap,
        elements: impl IntoIterator<Item = Value, IntoIter: ExactSizeIterator>,
    ) -> ManagedRef<Self> {
        let mut elements = elements.into_iter();
        let len = elements.len();
        if len > 12 {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the compiler limits tuples to twelve elements") }
        }

        let boxed = heap.allocate_tuple_box(len);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let destination = unsafe { boxed.as_ptr().add(1).cast::<Value>() };
        for index in 0..len {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let value = unsafe {
                unwrap_option_invariant(
                    elements.next(),
                    "an exact-size tuple iterator yields its declared length",
                )
            };
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            unsafe { destination.add(index).write(value) };
        }
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            destination
                .add(len)
                .cast::<Cell<Option<ArrayTypeCheckId>>>()
                .write(Cell::new(None));
        }
        #[cfg(debug_assertions)]
        {
            let exhausted = elements.next().is_none();
            debug_assert!(exhausted);
        }
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { ManagedRef::from_raw(boxed) }
    }
}

#[expect(
    clippy::inline_always,
    reason = "tuple type-check caching runs inside structural type checks"
)]
impl ManagedRef<TupleObject> {
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        // SAFETY: the managed handle points to a live tuple box.
        unsafe { self.raw_box().as_ref() }
            .header_ref()
            .tuple_length()
    }

    #[must_use]
    pub(crate) fn get(&self, index: usize) -> Option<&Value> {
        self.as_slice().get(index)
    }

    pub(crate) fn iter(&self) -> slice::Iter<'_, Value> {
        self.as_slice().iter()
    }

    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Value] {
        // SAFETY: the managed handle proves the allocation and tuple length.
        unsafe { slice::from_raw_parts(tuple_elements(self.raw_box()), self.len()) }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) fn type_check(&self, id: ArrayTypeCheckId) -> ArrayTypeCheck {
        if self.type_check_cache().get() == Some(id) {
            ArrayTypeCheck::Clean(id)
        } else {
            ArrayTypeCheck::Unknown
        }
    }

    #[inline(always)]
    pub(crate) fn mark_type_checked(&self, id: ArrayTypeCheckId) {
        self.type_check_cache().set(Some(id));
    }

    fn type_check_cache(&self) -> &Cell<Option<ArrayTypeCheckId>> {
        let offset = size_of::<HeapBox<TupleObject>>() + self.len() * size_of::<Value>();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            &*self
                .raw_box()
                .cast::<u8>()
                .add(offset)
                .cast::<Cell<Option<ArrayTypeCheckId>>>()
                .as_ptr()
        }
    }
}

impl Trace for TupleObject {
    fn type_tag() -> TypeTag {
        TypeTag::Tuple
    }

    fn enqueue_children(
        &mut self,
        allocation: NonNull<HeapBox<()>>,
        queue: &DropQueue,
        mode: TeardownMode,
    ) {
        let boxed = allocation.cast::<HeapBox<Self>>();
        // SAFETY: the type tag proves this is a live tuple box.
        let len = unsafe { boxed.as_ref() }.header_ref().tuple_length();
        // SAFETY: the type tag proves this is a live tuple box.
        let elements = unsafe { tuple_elements(boxed) }.cast_mut();
        for index in 0..len {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            queue.release_value(unsafe { elements.add(index).read() }, mode);
        }
    }

    fn visit_children(&self, allocation: NonNull<HeapBox<()>>, visitor: &mut TraceVisitor<'_>) {
        let boxed = allocation.cast::<HeapBox<Self>>();
        // SAFETY: the type tag proves this is a live tuple box.
        let len = unsafe { boxed.as_ref() }.header_ref().tuple_length();
        // SAFETY: the type tag proves the allocation and tuple length.
        let elements = unsafe { slice::from_raw_parts(tuple_elements(boxed), len) };
        for element in elements {
            if let Some(child) = element.collectable_box() {
                visitor.visit(child);
            }
        }
    }
}

const unsafe fn tuple_elements(boxed: NonNull<HeapBox<TupleObject>>) -> *const Value {
    // SAFETY: tuple elements start directly after their heap box.
    unsafe {
        boxed
            .cast::<u8>()
            .as_ptr()
            .add(size_of::<HeapBox<TupleObject>>())
    }
    .cast()
}

const _: () = assert!(size_of::<HeapBox<TupleObject>>() == size_of::<Header>());
const _: () = assert!(align_of::<HeapBox<TupleObject>>() >= align_of::<Value>());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_elements_after_the_header() {
        let heap = Heap::new();
        let tuple = TupleObject::with_elements(&heap, [Value::int(1), Value::int(2)]);
        let allocation = tuple.raw_box().as_ptr().cast::<u8>();
        let elements = tuple.as_slice().as_ptr().cast::<u8>();

        assert_eq!(tuple.len(), 2);
        assert_eq!(tuple.get(0).and_then(Value::as_int), Some(1));
        assert_eq!(tuple.get(1).and_then(Value::as_int), Some(2));
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        assert_eq!(unsafe { elements.offset_from(allocation) }, 16);
        assert_eq!(tuple_layout(2).size(), 56);
    }

    #[test]
    fn remembers_one_successful_structural_check() {
        let heap = Heap::new();
        let tuple = TupleObject::with_pair(&heap, Value::int(1), Value::int(2));
        let first = ArrayTypeCheckId::new(7);
        let second = ArrayTypeCheckId::new(8);

        assert_eq!(tuple.type_check(first), ArrayTypeCheck::Unknown);
        tuple.mark_type_checked(first);
        assert_eq!(tuple.type_check(first), ArrayTypeCheck::Clean(first));
        assert_eq!(tuple.type_check(second), ArrayTypeCheck::Unknown);

        tuple.mark_type_checked(second);
        assert_eq!(tuple.type_check(first), ArrayTypeCheck::Unknown);
        assert_eq!(tuple.type_check(second), ArrayTypeCheck::Clean(second));
    }
}
