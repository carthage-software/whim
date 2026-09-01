//! Heap headers and tracing contracts.

use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::unreachable_invariant;
use crate::value::heap::BUFFERED_BIT;
use crate::value::heap::COLOR_MASK;
use crate::value::heap::COLOR_SHIFT;
use crate::value::heap::IMMORTAL_BIT;
use crate::value::heap::INTERNED_BIT;
use crate::value::heap::ROOT_INDEX_MASK;
use crate::value::heap::ROOT_INDEX_MAX;
use crate::value::heap::ROOT_INDEX_SHIFT;
use crate::value::heap::TUPLE_LENGTH_MASK;
use crate::value::heap::TUPLE_LENGTH_SHIFT;
use crate::value::heap::TYPE_TAG_MASK;
use crate::value::heap::queue::DropQueue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum TypeTag {
    ByteString = 0,
    Vec = 1,
    Dict = 2,
    Tuple = 3,
    Function = 4,
    Object = 5,
    Weak = 6,
    WeakMap = 7,
    Iterator = 8,
    FinalizableObject = 9,
}

impl TypeTag {
    fn from_bits(bits: u32) -> Self {
        match bits & TYPE_TAG_MASK {
            0 => Self::ByteString,
            1 => Self::Vec,
            2 => Self::Dict,
            3 => Self::Tuple,
            4 => Self::Function,
            5 => Self::Object,
            6 => Self::Weak,
            7 => Self::WeakMap,
            8 => Self::Iterator,
            9 => Self::FinalizableObject,
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("a header carries a known type tag") },
        }
    }

    /// Whether this type can form reference cycles.
    pub(crate) const fn is_collectable(self) -> bool {
        matches!(
            self,
            Self::Vec
                | Self::Dict
                | Self::Tuple
                | Self::Function
                | Self::Object
                | Self::FinalizableObject
                | Self::WeakMap
        )
    }
}

/// A cycle collector mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Color {
    Black = 0,
    Gray = 1,
    White = 2,
    /// A possible cycle root.
    Purple = 3,
}

impl Color {
    fn from_bits(bits: u32) -> Self {
        match (bits & COLOR_MASK) >> COLOR_SHIFT {
            0 => Self::Black,
            1 => Self::Gray,
            2 => Self::White,
            3 => Self::Purple,
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("a colour occupies exactly two bits") },
        }
    }
}

#[repr(C)]
pub(crate) struct Header {
    rc: Cell<u32>,
    info: Cell<u32>,
    /// The type-erased back-pointer to the owning
    /// [`Heap`](crate::value::heap::Heap), or a dangling sentinel for immortal
    /// boxes, which never reach the heap.
    heap: Cell<NonNull<()>>,
}

impl Header {
    /// Creates a mortal header with one reference.
    #[must_use]
    pub(in crate::value::heap) const fn new(tag: TypeTag, heap: NonNull<()>) -> Self {
        Self {
            rc: Cell::new(1),
            info: Cell::new(tag as u32),
            heap: Cell::new(heap),
        }
    }

    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "tuple lengths are limited to twelve elements"
    )]
    pub(in crate::value::heap) fn new_tuple(length: usize, heap: NonNull<()>) -> Self {
        debug_assert!(length <= 12);
        Self {
            rc: Cell::new(1),
            info: Cell::new(TypeTag::Tuple as u32 | ((length as u32) << TUPLE_LENGTH_SHIFT)),
            heap: Cell::new(heap),
        }
    }

    #[must_use]
    pub(crate) const fn heap_ptr(&self) -> NonNull<()> {
        self.heap.get()
    }

    #[must_use]
    pub(crate) fn type_tag(&self) -> TypeTag {
        TypeTag::from_bits(self.info.get())
    }

    /// Replaces the runtime category while preserving every other header bit.
    pub(in crate::value::heap) fn set_type_tag(&self, tag: TypeTag) {
        let info = self.info.get() & !TYPE_TAG_MASK;
        self.info.set(info | tag as u32);
    }

    #[must_use]
    pub(crate) const fn reference_count(&self) -> u32 {
        self.rc.get()
    }

    #[must_use]
    pub(crate) const fn is_immortal(&self) -> bool {
        self.info.get() & IMMORTAL_BIT != 0
    }

    pub(crate) fn set_immortal(&self) {
        self.info.set(self.info.get() | IMMORTAL_BIT);
    }

    #[must_use]
    pub(crate) const fn is_interned(&self) -> bool {
        self.info.get() & INTERNED_BIT != 0
    }

    pub(crate) fn set_interned(&self) {
        self.info.set(self.info.get() | INTERNED_BIT);
    }

    #[must_use]
    pub(crate) const fn is_buffered(&self) -> bool {
        self.info.get() & BUFFERED_BIT != 0
    }

    pub(crate) fn set_buffered(&self, buffered: bool) {
        let info = self.info.get();
        self.info.set(if buffered {
            info | BUFFERED_BIT
        } else {
            info & !BUFFERED_BIT
        });
    }

    #[must_use]
    pub(crate) fn color(&self) -> Color {
        Color::from_bits(self.info.get())
    }

    pub(crate) fn set_color(&self, color: Color) {
        let info = self.info.get() & !COLOR_MASK;
        self.info.set(info | ((color as u32) << COLOR_SHIFT));
    }

    #[must_use]
    pub(crate) const fn root_index(&self) -> u32 {
        (self.info.get() & ROOT_INDEX_MASK) >> ROOT_INDEX_SHIFT
    }

    /// Sets the box's root-buffer index.
    pub(crate) fn set_root_index(&self, index: u32) {
        let info = self.info.get() & !ROOT_INDEX_MASK;
        self.info
            .set(info | ((index & ROOT_INDEX_MAX) << ROOT_INDEX_SHIFT));
    }

    #[must_use]
    pub(crate) fn tuple_length(&self) -> usize {
        debug_assert_eq!(self.type_tag(), TypeTag::Tuple);
        ((self.info.get() & TUPLE_LENGTH_MASK) >> TUPLE_LENGTH_SHIFT) as usize
    }

    pub(crate) fn increment(&self) {
        self.rc.set(self.rc.get() + 1);
    }

    pub(crate) fn decrement(&self) -> u32 {
        let current = self.rc.get();
        debug_assert!(
            current != 0,
            "whim-runtime: a reference count was decremented below zero"
        );
        let next = current - 1;
        self.rc.set(next);
        next
    }
}

#[repr(C)]
pub(crate) struct HeapBox<T> {
    pub(in crate::value::heap) header: Header,
    pub(in crate::value::heap) payload: T,
}

impl<T> HeapBox<T> {
    pub(crate) const fn state_ref(&self) -> &T {
        &self.payload
    }

    pub(crate) const fn header_ref(&self) -> &Header {
        &self.header
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeardownMode {
    Full,
    /// A cycle whose collectable children were already decremented.
    CycleMember,
}

/// A type-erased cycle-collector edge visitor without trait-object dispatch.
pub(crate) struct TraceVisitor<'visit> {
    state: NonNull<()>,
    visit: unsafe fn(NonNull<()>, NonNull<HeapBox<()>>),
    lifetime: PhantomData<&'visit mut ()>,
}

impl<'visit> TraceVisitor<'visit> {
    #[must_use]
    pub(crate) fn new<F>(visit: &'visit mut F) -> Self
    where
        F: FnMut(NonNull<HeapBox<()>>),
    {
        unsafe fn call<F>(state: NonNull<()>, child: NonNull<HeapBox<()>>)
        where
            F: FnMut(NonNull<HeapBox<()>>),
        {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let visit = unsafe { state.cast::<F>().as_mut() };
            visit(child);
        }

        Self {
            state: NonNull::from(visit).cast(),
            visit: call::<F>,
            lifetime: PhantomData,
        }
    }

    #[inline(always)]
    #[expect(
        clippy::inline_always,
        reason = "edge visits are the collector's innermost callback"
    )]
    pub(crate) fn visit(&mut self, child: NonNull<HeapBox<()>>) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { (self.visit)(self.state, child) };
    }
}

pub(crate) trait Trace {
    fn type_tag() -> TypeTag;

    fn enqueue_children(
        &mut self,
        allocation: NonNull<HeapBox<()>>,
        queue: &mut DropQueue,
        mode: TeardownMode,
    );

    /// Enumerates this payload's edges to collectable children without
    /// consuming them, for the cycle collector's trial deletion.
    fn visit_children(&self, _allocation: NonNull<HeapBox<()>>, _visit: &mut TraceVisitor<'_>) {}
}

/// A shallow clone for copy-on-write mutation.
pub(crate) trait CowClone {
    fn cow_clone(&self) -> Self;
}
