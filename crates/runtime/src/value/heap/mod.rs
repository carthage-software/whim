//! Heap allocation, reference counting, and cycle collection.

use std::alloc::Layout;
use std::alloc::alloc;
use std::alloc::dealloc;
use std::alloc::handle_alloc_error;
use std::array;
use std::cell::Cell;
use std::cell::RefCell;
use std::cell::UnsafeCell;
use std::io;
use std::mem;
use std::process;
use std::ptr;
use std::ptr::NonNull;
use std::rc::Rc;

use hashbrown::DefaultHashBuilder;
use hashbrown::HashMap;
use hashbrown::HashTable;

use crate::unreachable_invariant;
use crate::value::Value;
use crate::value::atom::AtomBox;
use crate::value::dict::DictObject;
use crate::value::function::FunctionObject;
use crate::value::gc;
use crate::value::hash::HashState;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::Color;
use crate::value::heap::metadata::Header;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;
use crate::value::heap::queue::Erased;
use crate::value::iterator::IteratorObject;
use crate::value::object::BuiltInHooks;
use crate::value::object::BuiltInState;
use crate::value::object::InstanceObject;
use crate::value::string::ByteStringObject;
use crate::value::tuple::TupleObject;
use crate::value::tuple::tuple_layout;
use crate::value::vec::VecObject;
use crate::value::weak::WeakMapObject;
use crate::value::weak::WeakReference;

pub(in crate::value) mod bytes;
pub(crate) mod handle;
pub(crate) mod metadata;
pub(crate) mod queue;

const TYPE_TAG_MASK: u32 = 0b1111;
const IMMORTAL_BIT: u32 = 1 << 4;
const BUFFERED_BIT: u32 = 1 << 5;
const COLOR_SHIFT: u32 = 6;
const COLOR_MASK: u32 = 0b11 << COLOR_SHIFT;
/// Marks an interned byte string.
const INTERNED_BIT: u32 = 1 << 8;
const ROOT_INDEX_SHIFT: u32 = 9;
const ROOT_INDEX_MAX: u32 = 0x0007_FFFF;
const ROOT_INDEX_MASK: u32 = ROOT_INDEX_MAX << ROOT_INDEX_SHIFT;
const TUPLE_LENGTH_SHIFT: u32 = 28;
const TUPLE_LENGTH_MASK: u32 = 0b1111 << TUPLE_LENGTH_SHIFT;

const _: () = assert!(size_of::<Header>() == 16);

/// An object's payload, built-in state, and property layout.
pub(in crate::value::heap) struct ObjectLayout {
    layout: Layout,
    slots_offset: usize,
}

fn extend_built_in_layout(layout: Layout, hooks: &BuiltInHooks) -> Layout {
    let built_in_layout = Layout::new::<BuiltInState>()
        .extend(hooks.layout)
        .unwrap_or_else(|_| allocation_failure(hooks.layout.size()))
        .0;
    layout
        .extend(built_in_layout)
        .unwrap_or_else(|_| allocation_failure(hooks.layout.size()))
        .0
}

pub(in crate::value::heap) fn object_layout(
    slot_count: usize,
    built_in_hooks: &[&BuiltInHooks],
) -> ObjectLayout {
    let slots = Layout::array::<Cell<Value>>(slot_count)
        .unwrap_or_else(|_| allocation_failure(slot_count.saturating_mul(size_of::<Value>())));
    let mut layout = Layout::new::<HeapBox<InstanceObject>>();
    for hooks in built_in_hooks {
        layout = extend_built_in_layout(layout, hooks);
    }
    let (layout, slots_offset) = layout
        .extend(slots)
        .unwrap_or_else(|_| allocation_failure(slots.size()));
    ObjectLayout {
        layout: layout.pad_to_align(),
        slots_offset,
    }
}

fn object_layout_from_state_chain(
    slot_count: usize,
    mut built_in: Option<NonNull<BuiltInState>>,
) -> ObjectLayout {
    let slots = Layout::array::<Cell<Value>>(slot_count)
        .unwrap_or_else(|_| allocation_failure(slot_count.saturating_mul(size_of::<Value>())));
    let mut layout = Layout::new::<HeapBox<InstanceObject>>();
    while let Some(state) = built_in {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let state = unsafe { state.as_ref() };
        layout = extend_built_in_layout(layout, state.hooks());
        built_in = state.next();
    }
    let (layout, slots_offset) = layout
        .extend(slots)
        .unwrap_or_else(|_| allocation_failure(slots.size()));
    ObjectLayout {
        layout: layout.pad_to_align(),
        slots_offset,
    }
}

const _: () = assert!(size_of::<ManagedRef<ByteStringObject>>() == 8);
const _: () = assert!(size_of::<Option<ManagedRef<ByteStringObject>>>() == 8);

pub(in crate::value::heap) fn allocate_box<T>() -> NonNull<HeapBox<T>> {
    let layout = Layout::new::<HeapBox<T>>();
    // SAFETY: this layout matches the allocation.
    let pointer = unsafe { alloc(layout) };
    NonNull::new(pointer)
        .unwrap_or_else(|| handle_alloc_error(layout))
        .cast()
}

/// # Safety
///
/// `box_pointer` must come from [`allocate_box`] and its payload must be
/// dropped.
unsafe fn deallocate_box<T>(box_pointer: NonNull<HeapBox<T>>) {
    // SAFETY: this layout matches the allocation.
    unsafe {
        dealloc(
            box_pointer.cast::<u8>().as_ptr(),
            Layout::new::<HeapBox<T>>(),
        );
    }
}

pub(in crate::value::heap) fn allocate_bytes(len: usize) -> NonNull<u8> {
    let Ok(layout) = Layout::array::<u8>(len) else {
        allocation_failure(len)
    };
    // SAFETY: this layout matches the allocation.
    let pointer = unsafe { alloc(layout) };
    NonNull::new(pointer).unwrap_or_else(|| handle_alloc_error(layout))
}

/// # Safety
///
/// `pointer` must come from [`allocate_bytes`] with the same `len` and must not
/// be used again.
pub(in crate::value::heap) unsafe fn deallocate_bytes(pointer: NonNull<u8>, len: usize) {
    let Ok(layout) = Layout::array::<u8>(len) else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a buffer's layout was valid when it was allocated") }
    };
    // SAFETY: this layout matches the allocation.
    unsafe { dealloc(pointer.as_ptr(), layout) };
}

/// A built-in state drop deferred until the queue borrow ends.
type DeferredBuiltInDrop = (NonNull<()>, unsafe fn(NonNull<()>));

/// Maximum recursive object teardown depth.
const TEARDOWN_DEPTH_LIMIT: u32 = 32;

const DEFAULT_CYCLE_THRESHOLD: usize = 10_001;
const CYCLE_THRESHOLD_STEP: usize = 10_000;
const CYCLE_THRESHOLD_TRIGGER: usize = 100;
/// Keep at most 8 KiB of root slots after a burst on 64-bit targets.
const ROOT_BUFFER_RETAIN_LIMIT: usize = 1_024;
/// Number of pooled fixed-slot object layouts.
const OBJECT_POOL_CLASSES: usize = 16;
const OBJECT_POOL_BYTE_LIMIT: usize = 16 * 1024 * 1024;

pub(in crate::value) type Roots = Vec<NonNull<HeapBox<()>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalizerOrigin {
    ReferenceCount,
    /// Found in an unreachable cycle.
    Cycle,
    CycleInCoroutine,
    Shutdown,
}

pub(crate) struct PendingFinalizer {
    pub(crate) object: ManagedRef<InstanceObject>,
    pub(crate) origin: FinalizerOrigin,
}

type WeakRegistry = HashMap<usize, Vec<NonNull<HeapBox<()>>>, DefaultHashBuilder>;
type FinalizableObjects = HashMap<NonNull<HeapBox<InstanceObject>>, u64, DefaultHashBuilder>;

type Interner = HashTable<AtomBox>;

pub(crate) struct Heap {
    hash_state: HashState,
    byte_strings: RefCell<[Option<ManagedRef<ByteStringObject>>; 256]>,
    /// A weak table of interned strings.
    interner: RefCell<Interner>,
    roots: UnsafeCell<Roots>,
    cycle_threshold: Cell<usize>,
    collecting: Cell<bool>,
    /// Active coroutine resumptions, used to choose finalizer dispatch.
    coroutine_depth: Cell<usize>,
    pending_collection: Cell<bool>,
    weak_registry: RefCell<WeakRegistry>,
    has_weak_dependents: Cell<bool>,
    drop_queue: UnsafeCell<DropQueue>,
    /// Live finalizable objects and their allocation order.
    finalizable_objects: UnsafeCell<FinalizableObjects>,
    finalizable_sequence: Cell<u64>,
    pending_finalizers: UnsafeCell<Vec<PendingFinalizer>>,
    /// Free object blocks by property count.
    object_free_lists: UnsafeCell<[Vec<NonNull<u8>>; OBJECT_POOL_CLASSES]>,
    object_pool_bytes: Cell<usize>,
    draining: Cell<bool>,
    teardown_depth: Cell<u32>,
}

impl Heap {
    #[must_use]
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            hash_state: HashState::new(),
            byte_strings: RefCell::new(array::from_fn(|_| None)),
            interner: RefCell::new(HashTable::new()),
            roots: UnsafeCell::new(Vec::new()),
            cycle_threshold: Cell::new(DEFAULT_CYCLE_THRESHOLD),
            collecting: Cell::new(false),
            coroutine_depth: Cell::new(0),
            pending_collection: Cell::new(false),
            weak_registry: RefCell::new(HashMap::new()),
            has_weak_dependents: Cell::new(false),
            drop_queue: UnsafeCell::new(DropQueue::new()),
            finalizable_objects: UnsafeCell::new(HashMap::new()),
            finalizable_sequence: Cell::new(0),
            pending_finalizers: UnsafeCell::new(Vec::new()),
            object_free_lists: UnsafeCell::new(array::from_fn(|_| Vec::new())),
            object_pool_bytes: Cell::new(0),
            draining: Cell::new(false),
            teardown_depth: Cell::new(0),
        })
    }

    pub(in crate::value) fn register_finalizable(&self, object: NonNull<HeapBox<InstanceObject>>) {
        let sequence = self.finalizable_sequence.get();
        self.finalizable_sequence.set(sequence.wrapping_add(1));
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let replaced = unsafe { &mut *self.finalizable_objects.get() }.insert(object, sequence);
        debug_assert!(replaced.is_none());
    }

    /// Returns a live finalizer's allocation order.
    pub(in crate::value) fn finalizer_sequence(
        &self,
        object: NonNull<HeapBox<InstanceObject>>,
    ) -> Option<u64> {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { &*self.finalizable_objects.get() }
            .get(&object)
            .copied()
    }

    pub(in crate::value) fn schedule_finalizer(
        &self,
        object: NonNull<HeapBox<InstanceObject>>,
        origin: FinalizerOrigin,
    ) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let header = unsafe { &object.as_ref().header };
        debug_assert_eq!(header.type_tag(), TypeTag::FinalizableObject);
        header.increment();
        header.set_type_tag(TypeTag::Object);
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let removed = unsafe { &mut *self.finalizable_objects.get() }.remove(&object);
        debug_assert!(removed.is_some());
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { &mut *self.pending_finalizers.get() }.push(PendingFinalizer {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            object: unsafe { ManagedRef::from_raw(object) },
            origin,
        });
    }

    /// Queues all remaining destructors for shutdown.
    pub(crate) fn schedule_shutdown_finalizers(&self) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let objects = mem::take(unsafe { &mut *self.finalizable_objects.get() });
        let mut objects = objects.into_iter().collect::<Vec<_>>();
        objects.sort_unstable_by_key(|(_, sequence)| *sequence);
        for (object, _) in objects {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let header = unsafe { &object.as_ref().header };
            if header.type_tag() != TypeTag::FinalizableObject {
                continue;
            }
            if header.is_buffered() {
                self.remove_root(object.cast(), header);
            }
            header.increment();
            header.set_type_tag(TypeTag::Object);
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe { &mut *self.pending_finalizers.get() }.push(PendingFinalizer {
                // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                object: unsafe { ManagedRef::from_raw(object) },
                origin: FinalizerOrigin::Shutdown,
            });
        }
    }

    pub(crate) fn take_pending_finalizers(&self) -> Vec<PendingFinalizer> {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        mem::take(unsafe { &mut *self.pending_finalizers.get() })
    }

    /// Restores finalizers after a destructor throws.
    pub(crate) fn return_pending_finalizers(
        &self,
        finalizers: impl IntoIterator<Item = PendingFinalizer>,
    ) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { &mut *self.pending_finalizers.get() }.extend(finalizers);
    }

    #[must_use]
    pub(crate) fn has_pending_finalizers(&self) -> bool {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        !unsafe { &*self.pending_finalizers.get() }.is_empty()
    }

    #[must_use]
    pub(crate) fn has_finalizable_objects(&self) -> bool {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        !unsafe { &*self.finalizable_objects.get() }.is_empty()
    }

    pub(crate) fn abandon_finalizers(&self) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let objects = mem::take(unsafe { &mut *self.finalizable_objects.get() });
        for (object, _) in objects {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let header = unsafe { &object.as_ref().header };
            if header.type_tag() == TypeTag::FinalizableObject {
                header.set_type_tag(TypeTag::Object);
            }
        }
    }

    #[must_use]
    pub(crate) fn byte_string(&self, byte: u8) -> ManagedRef<ByteStringObject> {
        let mut strings = self.byte_strings.borrow_mut();
        strings[usize::from(byte)]
            .get_or_insert_with(|| ByteStringObject::from_bytes(self, &[byte]))
            .clone()
    }

    pub(in crate::value) const fn hash_state(&self) -> &HashState {
        &self.hash_state
    }

    pub(in crate::value) const fn interner(&self) -> &RefCell<Interner> {
        &self.interner
    }

    pub(in crate::value) const fn empty_interner() -> Interner {
        HashTable::new()
    }

    pub(crate) fn configure_cycle_threshold(&self, threshold: Option<usize>) {
        self.cycle_threshold
            .set(threshold.unwrap_or(DEFAULT_CYCLE_THRESHOLD));
    }

    pub(crate) fn enter_coroutine(&self) {
        self.coroutine_depth.set(self.coroutine_depth.get() + 1);
    }

    pub(crate) fn leave_coroutine(&self) {
        let depth = self.coroutine_depth.get();
        debug_assert!(depth != 0);
        self.coroutine_depth.set(depth - 1);
    }

    pub(in crate::value) const fn cycle_finalizer_origin(&self) -> FinalizerOrigin {
        if self.coroutine_depth.get() == 0 {
            FinalizerOrigin::Cycle
        } else {
            FinalizerOrigin::CycleInCoroutine
        }
    }

    /// Collects cycles and returns the number of freed boxes.
    pub(crate) fn collect_cycles(&self) -> usize {
        gc::collect(self)
    }

    /// Adapts the threshold to the last collection's yield.
    fn collect_cycles_automatically(&self) {
        let freed = self.collect_cycles();
        let threshold = self.cycle_threshold.get();
        let threshold = if freed < CYCLE_THRESHOLD_TRIGGER {
            threshold
                .saturating_add(CYCLE_THRESHOLD_STEP)
                .min(ROOT_INDEX_MAX as usize)
        } else {
            threshold
                .saturating_sub(CYCLE_THRESHOLD_STEP)
                .max(DEFAULT_CYCLE_THRESHOLD)
        };

        self.cycle_threshold.set(threshold);
    }

    pub(in crate::value) fn take_roots(&self) -> Roots {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        mem::take(unsafe { &mut *self.roots.get() })
    }

    /// Reuses the collector's emptied root storage after it clears buffered flags.
    pub(in crate::value) fn recycle_roots(&self, mut roots: Roots) {
        debug_assert!(self.is_collecting());
        roots.clear();
        trim_empty_roots(&mut roots);
        // SAFETY: collection excludes buffering new roots, and the collector no
        // longer borrows either root vector when returning this empty storage.
        let buffer = unsafe { &mut *self.roots.get() };
        debug_assert!(buffer.is_empty());
        *buffer = roots;
    }

    pub(in crate::value) const fn is_collecting(&self) -> bool {
        self.collecting.get()
    }

    pub(in crate::value) fn set_collecting(&self, collecting: bool) {
        self.collecting.set(collecting);
    }

    pub(in crate::value) fn allocate_tuple_box(&self, len: usize) -> NonNull<HeapBox<TupleObject>> {
        let layout = tuple_layout(len);
        // SAFETY: this layout matches the allocation.
        let allocation = unsafe { alloc(layout) };
        let boxed = NonNull::new(allocation)
            .unwrap_or_else(|| handle_alloc_error(layout))
            .cast::<HeapBox<TupleObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe {
            boxed.as_ptr().write(HeapBox {
                header: Header::new_tuple(len, NonNull::from(self).cast()),
                payload: TupleObject,
            });
        }
        boxed
    }

    /// # Safety
    ///
    /// `box_pointer` must be a dead object allocated with `object_layout`.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "passing the small layout by value keeps object teardown scalar"
    )]
    unsafe fn deallocate_object_box(
        &self,
        box_pointer: NonNull<HeapBox<InstanceObject>>,
        slot_count: usize,
        object_layout: ObjectLayout,
        poolable: bool,
    ) {
        if poolable
            && slot_count < OBJECT_POOL_CLASSES
            && self.object_pool_bytes.get() + object_layout.layout.size() <= OBJECT_POOL_BYTE_LIMIT
        {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let lists = unsafe { &mut *self.object_free_lists.get() };
            lists[slot_count].push(box_pointer.cast());
            self.object_pool_bytes
                .set(self.object_pool_bytes.get() + object_layout.layout.size());
            return;
        }
        // SAFETY: this layout matches the allocation.
        unsafe {
            dealloc(box_pointer.cast::<u8>().as_ptr(), object_layout.layout);
        }
    }

    fn allocate_object_box(
        &self,
        slot_count: usize,
        object_layout: &ObjectLayout,
        poolable: bool,
    ) -> NonNull<u8> {
        if poolable && slot_count < OBJECT_POOL_CLASSES {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let lists = unsafe { &mut *self.object_free_lists.get() };
            if let Some(allocation) = lists[slot_count].pop() {
                self.object_pool_bytes
                    .set(self.object_pool_bytes.get() - object_layout.layout.size());
                return allocation;
            }
        }

        // SAFETY: this layout matches the allocation.
        let allocation = unsafe { alloc(object_layout.layout) };
        NonNull::new(allocation).unwrap_or_else(|| handle_alloc_error(object_layout.layout))
    }

    pub(in crate::value) fn release_erased(&self, box_pointer: NonNull<HeapBox<()>>) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let header = unsafe { &box_pointer.as_ref().header };
        if header.is_immortal() {
            return;
        }

        if header.decrement() == 0 {
            self.release_last_reference(box_pointer, header);
        } else if header.type_tag().is_collectable() && !header.is_buffered() {
            self.maybe_buffer_root(box_pointer, header);
        }
    }

    #[inline(never)]
    fn release_last_reference(&self, box_pointer: NonNull<HeapBox<()>>, header: &Header) {
        if header.is_interned() {
            self.unintern(box_pointer.cast::<HeapBox<ByteStringObject>>());
        }

        if header.is_buffered() {
            self.remove_root(box_pointer, header);
        }

        let tag = header.type_tag();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        if tag == TypeTag::Object && unsafe { self.try_teardown_shallow_object(box_pointer.cast()) }
        {
            return;
        }

        if tag == TypeTag::FinalizableObject {
            self.schedule_finalizer(box_pointer.cast(), FinalizerOrigin::ReferenceCount);
            return;
        }

        self.drain_from(Erased { box_pointer, tag });
    }

    /// # Safety
    ///
    /// `box_pointer` must be an unreferenced object owned by this heap.
    unsafe fn try_teardown_shallow_object(
        &self,
        box_pointer: NonNull<HeapBox<InstanceObject>>,
    ) -> bool {
        if self.has_weak_dependents.get() {
            return false;
        }

        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let payload = unsafe { &mut (*box_pointer.as_ptr()).payload };
        if payload.has_built_in() {
            return false;
        }

        let slot_count = payload.slot_count();
        if slot_count == 0 {
            // SAFETY: the source and target ranges are valid.
            unsafe { ptr::drop_in_place(payload) };
            let layout = object_layout(0, &[]);
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe { self.deallocate_object_box(box_pointer, 0, layout, true) };
            return true;
        }

        let depth = self.teardown_depth.get();
        if depth >= TEARDOWN_DEPTH_LIMIT {
            return false;
        }

        self.teardown_depth.set(depth + 1);
        for index in 0..slot_count {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            drop(unsafe { payload.take_slot_unchecked(index) });
        }

        self.teardown_depth.set(depth);
        // SAFETY: the source and target ranges are valid.
        unsafe { ptr::drop_in_place(payload) };
        let layout = object_layout(slot_count, &[]);
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { self.deallocate_object_box(box_pointer, slot_count, layout, true) };
        true
    }

    /// Decrements a box and returns it when teardown must begin.
    pub(in crate::value::heap) fn release_reference(
        &self,
        box_pointer: NonNull<HeapBox<()>>,
    ) -> Option<Erased> {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let header = unsafe { &box_pointer.as_ref().header };
        if header.is_immortal() {
            return None;
        }

        if header.decrement() == 0 {
            if header.is_interned() {
                self.unintern(box_pointer.cast::<HeapBox<ByteStringObject>>());
            }

            if header.is_buffered() {
                self.remove_root(box_pointer, header);
            }

            if header.type_tag() == TypeTag::FinalizableObject {
                self.schedule_finalizer(box_pointer.cast(), FinalizerOrigin::ReferenceCount);
                return None;
            }

            Some(Erased {
                box_pointer,
                tag: header.type_tag(),
            })
        } else {
            self.maybe_buffer_root(box_pointer, header);
            None
        }
    }

    /// Buffers a possible cycle root and collects at the threshold.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the root buffer is bounded by the header's index field"
    )]
    fn maybe_buffer_root(&self, box_pointer: NonNull<HeapBox<()>>, header: &Header) {
        if !header.type_tag().is_collectable() || header.is_buffered() {
            return;
        }

        if matches!(
            header.type_tag(),
            TypeTag::Object | TypeTag::FinalizableObject
        ) {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let object = unsafe {
                &box_pointer
                    .cast::<HeapBox<InstanceObject>>()
                    .as_ref()
                    .payload
            };

            if !object.cycle_possible() {
                return;
            }
        }

        if self.is_collecting() {
            return;
        }

        let should_collect = {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let roots = unsafe { &mut *self.roots.get() };
            if roots.len() > ROOT_INDEX_MAX as usize {
                return;
            }

            header.set_color(Color::Purple);
            header.set_buffered(true);
            header.set_root_index(roots.len() as u32);
            roots.push(box_pointer);
            roots.len() >= self.cycle_threshold.get()
        };

        if should_collect {
            if self.draining.get() {
                self.pending_collection.set(true);
            } else {
                self.collect_cycles_automatically();
            }
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "every root slot fits the header's index field"
    )]
    fn remove_root(&self, box_pointer: NonNull<HeapBox<()>>, header: &Header) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let roots = unsafe { &mut *self.roots.get() };
        let slot = header.root_index() as usize;
        debug_assert!(
            slot < roots.len() && roots[slot] == box_pointer,
            "whim-runtime: a buffered box's recorded root slot is stale"
        );

        roots.swap_remove(slot);
        if slot < roots.len() {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let moved = unsafe { roots[slot].as_ref() };
            moved.header_ref().set_root_index(slot as u32);
        }

        header.set_buffered(false);
        if roots.is_empty() {
            trim_empty_roots(roots);
        }
    }

    fn drain_from(&self, first: Erased) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { &mut *self.drop_queue.get() }.pending.push(first);
        self.drain_pending();
    }

    /// Drains teardown and runs a collection deferred during it.
    pub(in crate::value) fn drain_pending(&self) {
        if self.draining.replace(true) {
            return;
        }

        loop {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let entry = unsafe { &mut *self.drop_queue.get() }.pending.pop();
            let Some(entry) = entry else { break };
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe { self.teardown_in_mode(entry.box_pointer, entry.tag, TeardownMode::Full) };
        }

        self.draining.set(false);
        if self.pending_collection.replace(false) {
            self.collect_cycles_automatically();
        }
    }

    /// # Safety
    ///
    /// `box_pointer` must be an unreferenced live box owned by this heap with
    /// the given `tag`.
    pub(in crate::value) unsafe fn teardown_in_mode(
        &self,
        box_pointer: NonNull<HeapBox<()>>,
        tag: TypeTag,
        mode: TeardownMode,
    ) {
        match tag {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            TypeTag::ByteString => unsafe {
                self.teardown_box::<ByteStringObject>(box_pointer, mode);
            },
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            TypeTag::Vec => unsafe { self.teardown_box::<VecObject>(box_pointer, mode) },
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            TypeTag::Dict => unsafe { self.teardown_box::<DictObject>(box_pointer, mode) },
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            TypeTag::Tuple => unsafe { self.teardown_tuple_box(box_pointer, mode) },
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            TypeTag::Function => unsafe { self.teardown_box::<FunctionObject>(box_pointer, mode) },
            TypeTag::Object => {
                if mode == TeardownMode::Full
                    // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                    && unsafe { self.try_teardown_shallow_object(box_pointer.cast()) }
                {
                    return;
                }

                if self.has_weak_dependents.get() {
                    self.notify_weak_dependents(box_pointer.cast());
                }

                // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                unsafe { self.teardown_object_box(box_pointer, mode) }
            }
            // SAFETY: the surrounding invariant makes this path unreachable.
            TypeTag::FinalizableObject => unsafe {
                unreachable_invariant("a finalizable object is scheduled before teardown")
            },
            TypeTag::Weak => {
                self.deregister_weak_reference(box_pointer.cast());
                // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                unsafe { self.teardown_box::<WeakReference>(box_pointer, mode) }
            }
            TypeTag::WeakMap => {
                self.deregister_weak_map_keys(box_pointer.cast());
                // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                unsafe { self.teardown_box::<WeakMapObject>(box_pointer, mode) }
            }
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            TypeTag::Iterator => unsafe { self.teardown_box::<IteratorObject>(box_pointer, mode) },
        }
    }

    pub(in crate::value) fn register_weak_dependent(
        &self,
        target_address: usize,
        dependent: NonNull<HeapBox<()>>,
    ) {
        self.has_weak_dependents.set(true);
        self.weak_registry
            .borrow_mut()
            .entry(target_address)
            .or_default()
            .push(dependent);
    }

    /// Removes a dependent that may already have died with its target.
    pub(in crate::value) fn deregister_weak_dependent(
        &self,
        target_address: usize,
        dependent: NonNull<HeapBox<()>>,
    ) {
        let mut registry = self.weak_registry.borrow_mut();
        if let Some(dependents) = registry.get_mut(&target_address) {
            dependents.retain(|&entry| entry != dependent);
            if dependents.is_empty() {
                registry.remove(&target_address);
            }
        }
        if registry.is_empty() {
            *registry = HashMap::new();
            self.has_weak_dependents.set(false);
        }
    }

    #[cold]
    #[inline(never)]
    fn notify_weak_dependents(&self, object: NonNull<HeapBox<InstanceObject>>) {
        let address = object.addr().get();
        let dependents = {
            let mut registry = self.weak_registry.borrow_mut();
            let dependents = registry.remove(&address);
            if registry.is_empty() {
                *registry = HashMap::new();
                self.has_weak_dependents.set(false);
            }
            dependents
        };
        let Some(dependents) = dependents else {
            return;
        };
        for dependent in dependents {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let boxed = unsafe { dependent.as_ref() };
            match boxed.header_ref().type_tag() {
                TypeTag::Weak => {
                    // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                    unsafe { dependent.cast::<HeapBox<WeakReference>>().as_ref() }
                        .state_ref()
                        .clear_target();
                }
                TypeTag::WeakMap => {
                    if self.is_collecting() && boxed.header_ref().is_buffered() {
                        continue;
                    }
                    let map =
                        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                        unsafe { dependent.cast::<HeapBox<WeakMapObject>>().as_ref() }.state_ref();
                    if let Some(value) = map.remove_entry_by_address(address) {
                        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
                        unsafe { &mut *self.drop_queue.get() }
                            .release_value(value, TeardownMode::Full);
                    }
                }
                // SAFETY: the surrounding invariant makes this path unreachable.
                _ => unsafe {
                    unreachable_invariant(
                        "only weak references and weak maps register as dependents",
                    )
                },
            }
        }
    }

    fn deregister_weak_reference(&self, weak: NonNull<HeapBox<WeakReference>>) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let payload = unsafe { weak.as_ref() }.state_ref();
        if let Some(target_address) = payload.target_address() {
            self.deregister_weak_dependent(target_address, weak.cast());
        }
    }

    fn deregister_weak_map_keys(&self, map: NonNull<HeapBox<WeakMapObject>>) {
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let payload = unsafe { map.as_ref() }.state_ref();
        for key_address in payload.key_addresses() {
            self.deregister_weak_dependent(key_address, map.cast());
        }
    }

    /// # Safety
    ///
    /// `box_pointer` must be an unreferenced live `HeapBox<T>` owned by this
    /// heap.
    unsafe fn teardown_box<T: Trace>(&self, box_pointer: NonNull<HeapBox<()>>, mode: TeardownMode) {
        let typed = box_pointer.cast::<HeapBox<T>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let payload = unsafe { &mut (*typed.as_ptr()).payload };
        let (released_bytes, deferred_built_in) = {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let queue = unsafe { &mut *self.drop_queue.get() };
            payload.enqueue_children(box_pointer, queue, mode);
            (queue.take_released_bytes(), queue.take_deferred_built_in())
        };
        if let Some(bytes) = released_bytes {
            bytes.release();
        }
        if let Some((data, drop_data)) = deferred_built_in {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe { drop_data(data) };
        }
        // SAFETY: the source and target ranges are valid.
        unsafe { ptr::drop_in_place(payload) };
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { deallocate_box(typed) };
    }

    /// # Safety
    ///
    /// `box_pointer` must be an unreferenced live tuple owned by this heap.
    unsafe fn teardown_tuple_box(&self, box_pointer: NonNull<HeapBox<()>>, mode: TeardownMode) {
        let typed = box_pointer.cast::<HeapBox<TupleObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let boxed = unsafe { &mut *typed.as_ptr() };
        let len = boxed.header.tuple_length();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let queue = unsafe { &mut *self.drop_queue.get() };
        boxed.payload.enqueue_children(box_pointer, queue, mode);
        // SAFETY: this layout matches the allocation.
        unsafe { dealloc(typed.cast::<u8>().as_ptr(), tuple_layout(len)) };
    }

    /// # Safety
    ///
    /// `box_pointer` must be an unreferenced live object owned by this heap.
    unsafe fn teardown_object_box(&self, box_pointer: NonNull<HeapBox<()>>, mode: TeardownMode) {
        let typed = box_pointer.cast::<HeapBox<InstanceObject>>();
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        let payload = unsafe { &mut (*typed.as_ptr()).payload };
        let slot_count = payload.slot_count();
        let built_in = payload.built_in_state_head();
        let layout = object_layout_from_state_chain(slot_count, built_in);
        let (released_bytes, deferred_built_in) = {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            let queue = unsafe { &mut *self.drop_queue.get() };
            payload.enqueue_children(box_pointer, queue, mode);
            (queue.take_released_bytes(), queue.take_deferred_built_in())
        };
        if let Some(bytes) = released_bytes {
            bytes.release();
        }
        if let Some((data, drop_data)) = deferred_built_in {
            // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
            unsafe { drop_data(data) };
        }
        // SAFETY: the source and target ranges are valid.
        unsafe { ptr::drop_in_place(payload) };
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        unsafe { self.deallocate_object_box(typed, slot_count, layout, built_in.is_none()) };
    }
}

fn trim_empty_roots(roots: &mut Roots) {
    debug_assert!(roots.is_empty());
    if roots.capacity() > ROOT_BUFFER_RETAIN_LIMIT {
        *roots = Vec::new();
    }
}

/// # Safety
///
/// `box_pointer` must reference a live box whose header tag matches its
/// payload.
#[inline(always)]
#[expect(
    clippy::inline_always,
    reason = "edge traversal is the collector's innermost operation"
)]
pub(in crate::value) unsafe fn visit_children_erased<F>(
    box_pointer: NonNull<HeapBox<()>>,
    visit: &mut F,
) where
    F: FnMut(NonNull<HeapBox<()>>),
{
    let mut visitor = TraceVisitor::new(visit);
    // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
    unsafe { visit_children_erased_with(box_pointer, &mut visitor) };
}

unsafe fn visit_children_erased_with(
    box_pointer: NonNull<HeapBox<()>>,
    visitor: &mut TraceVisitor<'_>,
) {
    // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
    let boxed = unsafe { box_pointer.as_ref() };
    match boxed.header_ref().type_tag() {
        TypeTag::ByteString | TypeTag::Weak | TypeTag::Iterator => {}
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        TypeTag::Vec => unsafe { box_pointer.cast::<HeapBox<VecObject>>().as_ref() }
            .state_ref()
            .visit_children(box_pointer, visitor),
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        TypeTag::Dict => unsafe { box_pointer.cast::<HeapBox<DictObject>>().as_ref() }
            .state_ref()
            .visit_children(box_pointer, visitor),
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        TypeTag::Tuple => unsafe { box_pointer.cast::<HeapBox<TupleObject>>().as_ref() }
            .state_ref()
            .visit_children(box_pointer, visitor),
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        TypeTag::Function => unsafe { box_pointer.cast::<HeapBox<FunctionObject>>().as_ref() }
            .state_ref()
            .visit_children(box_pointer, visitor),
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        TypeTag::Object | TypeTag::FinalizableObject => unsafe {
            box_pointer
                .cast::<HeapBox<InstanceObject>>()
                .as_ref()
                .state_ref()
                .visit_children(box_pointer, visitor);
        },
        // SAFETY: the single-threaded heap owns this live allocation and serializes this access.
        TypeTag::WeakMap => unsafe { box_pointer.cast::<HeapBox<WeakMapObject>>().as_ref() }
            .state_ref()
            .visit_children(box_pointer, visitor),
    }
}

/// Aborts after an infallible allocation fails.
#[cold]
#[inline(never)]
fn allocation_failure(bytes: usize) -> ! {
    use std::io::Write as _;

    let mut stderr = io::stderr();
    let _ = writeln!(
        stderr,
        "whim: fatal: could not allocate {bytes} bytes; aborting"
    );

    process::abort();
}

impl Drop for Heap {
    fn drop(&mut self) {
        self.collect_cycles();
        for (slot_count, free_list) in self.object_free_lists.get_mut().iter_mut().enumerate() {
            let layout = object_layout(slot_count, &[]).layout;
            for allocation in free_list.drain(..) {
                // SAFETY: this layout matches the allocation.
                unsafe { dealloc(allocation.as_ptr(), layout) };
            }
        }
        self.object_pool_bytes.set(0);
    }
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_CYCLE_THRESHOLD;
    use super::Heap;
    use super::ROOT_BUFFER_RETAIN_LIMIT;
    use crate::value::Value;
    use crate::value::vec::VecObject;

    fn root_buffer_state(heap: &Heap) -> (usize, usize) {
        // SAFETY: these single-threaded tests inspect the buffer between heap operations.
        let roots = unsafe { &*heap.roots.get() };
        (roots.len(), roots.capacity())
    }

    #[test]
    fn cycle_threshold_can_return_to_default() {
        let heap = Heap::new();
        heap.configure_cycle_threshold(Some(7));
        assert_eq!(heap.cycle_threshold.get(), 7);

        heap.configure_cycle_threshold(None);
        assert_eq!(heap.cycle_threshold.get(), DEFAULT_CYCLE_THRESHOLD);
    }

    #[test]
    fn root_storage_survives_empty_states_and_updates_swapped_slots() {
        let heap = Heap::new();
        let first = VecObject::with_elements(&heap, [Value::int(1)]);
        let second = VecObject::with_elements(&heap, [Value::int(2)]);
        drop(first.clone());
        drop(second.clone());
        let (length, capacity) = root_buffer_state(&heap);
        assert_eq!(length, 2);
        assert!(capacity > 0);

        drop(first);
        assert_eq!(root_buffer_state(&heap), (1, capacity));
        assert_eq!(second.header().root_index(), 0);
        drop(second);
        assert_eq!(root_buffer_state(&heap), (0, capacity));

        let next = VecObject::with_elements(&heap, [Value::int(3)]);
        drop(next.clone());
        assert_eq!(root_buffer_state(&heap), (1, capacity));
        assert_eq!(heap.collect_cycles(), 0);
        assert_eq!(root_buffer_state(&heap), (0, capacity));
        assert_eq!(next.get(0).and_then(Value::as_int), Some(3));
        drop(next.clone());
        drop(next);
        assert_eq!(root_buffer_state(&heap), (0, capacity));
    }

    #[test]
    fn root_storage_releases_large_bursts_after_destruction_or_collection() {
        let heap = Heap::new();
        heap.configure_cycle_threshold(Some(usize::MAX));
        for collect in [false, true] {
            let values: Vec<_> = (0..=ROOT_BUFFER_RETAIN_LIMIT)
                .map(|_| {
                    let value = VecObject::with_elements(&heap, [Value::int(1)]);
                    drop(value.clone());
                    value
                })
                .collect();
            assert!(root_buffer_state(&heap).1 > ROOT_BUFFER_RETAIN_LIMIT);
            if collect {
                assert_eq!(heap.collect_cycles(), 0);
                assert_eq!(root_buffer_state(&heap), (0, 0));
                assert!(
                    values
                        .iter()
                        .all(|value| value.get(0).and_then(Value::as_int) == Some(1))
                );
            }
            drop(values);
            assert_eq!(root_buffer_state(&heap), (0, 0));
        }
    }
}
