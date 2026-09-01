//! The runtime string representation: flat, rope, and slice.

use std::cell::Cell;
use std::cell::UnsafeCell;
use std::cmp::Ordering;
use std::hash::Hasher;
use std::marker::PhantomData;
use std::mem;
use std::mem::size_of;
use std::ptr;
use std::ptr::NonNull;
use std::slice;

use crate::unreachable_invariant;
use crate::value::hash::HashState;
use crate::value::heap::Heap;
use crate::value::heap::bytes::HeapBytes;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::Trace;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::queue::DropQueue;

const EAGER_FLAT_LIMIT: usize = 32;
const HASH_BLOCK_SIZE: usize = 256;

const _: () = assert!(size_of::<ShortString>() == 8);

pub(crate) mod short;

use crate::value::string::short::ShortString;

pub(crate) struct ByteStringObject {
    /// The cached 64-bit hash, or zero when it has not been computed.
    hash: Cell<u64>,
    repr: UnsafeCell<Repr>,
}

enum Repr {
    Flat(HeapBytes),
    Cons {
        left: ManagedRef<ByteStringObject>,
        right: ManagedRef<ByteStringObject>,
        /// The exact total byte length.
        len: usize,
    },
    Slice {
        /// The base string, always [`Repr::Flat`].
        base: ManagedRef<ByteStringObject>,
        offset: usize,
        /// The exact byte length.
        len: usize,
    },
}

impl ByteStringObject {
    const fn flat(bytes: HeapBytes) -> Self {
        Self {
            hash: Cell::new(0),
            repr: UnsafeCell::new(Repr::Flat(bytes)),
        }
    }

    #[must_use]
    pub(crate) fn from_bytes(heap: &Heap, bytes: &[u8]) -> ManagedRef<Self> {
        ManagedRef::new_in(heap, Self::flat(HeapBytes::from_slice(bytes)))
    }

    #[must_use]
    pub(crate) fn from_vec(heap: &Heap, bytes: Vec<u8>) -> ManagedRef<Self> {
        ManagedRef::new_in(heap, Self::flat(HeapBytes::from_vec(bytes)))
    }

    #[must_use]
    pub(crate) fn concat(
        heap: &Heap,
        left: &ManagedRef<Self>,
        right: &ManagedRef<Self>,
    ) -> ManagedRef<Self> {
        let len = left.len() + right.len();
        if len <= EAGER_FLAT_LIMIT {
            ManagedRef::new_in(
                heap,
                // SAFETY: the live string owns this payload, and the VM serializes representation access.
                Self::flat(unsafe {
                    HeapBytes::from_fragments(len, left.chunks().chain(right.chunks()))
                }),
            )
        } else {
            ManagedRef::new_in(
                heap,
                Self {
                    hash: Cell::new(0),
                    repr: UnsafeCell::new(Repr::Cons {
                        left: left.clone(),
                        right: right.clone(),
                        len,
                    }),
                },
            )
        }
    }

    /// # Safety
    ///
    /// `extra` must not overlap `string`'s own buffer: growth can free that
    /// buffer before copying `extra`.
    pub(crate) unsafe fn append_unique(string: &ManagedRef<Self>, extra: &[u8]) -> bool {
        if !string.is_unique() {
            return false;
        }
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        if let Repr::Flat(bytes) = unsafe { &*string.repr.get() } {
            debug_assert!(
                !overlaps(bytes.as_slice(), extra),
                "an append source overlaps the destination buffer"
            );
        }
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        match unsafe { &mut *string.repr.get() } {
            Repr::Flat(bytes) => {
                bytes.append(extra);
                string.hash.set(0);
                true
            }
            Repr::Cons { .. } | Repr::Slice { .. } => false,
        }
    }

    /// Appends chunks without flattening the source.
    ///
    /// # Safety
    ///
    /// `extra` and `string` must differ.
    pub(crate) unsafe fn append_unique_string(
        string: &ManagedRef<Self>,
        extra: &ManagedRef<Self>,
    ) -> bool {
        if !string.is_unique() {
            return false;
        }
        debug_assert!(
            !string.ptr_eq(extra),
            "an append source aliases the destination string"
        );
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        match unsafe { &mut *string.repr.get() } {
            Repr::Flat(bytes) => {
                // SAFETY: the live string owns this payload, and the VM serializes representation access.
                match unsafe { &*extra.repr.get() } {
                    Repr::Flat(extra) => bytes.append(extra.as_slice()),
                    Repr::Slice {
                        base, offset, len, ..
                    } => {
                        // SAFETY: the live string owns this payload, and the VM serializes representation access.
                        let Repr::Flat(base) = (unsafe { &*base.repr.get() }) else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant("a string slice always has a flat base")
                            }
                        };
                        bytes.append(&base.as_slice()[*offset..*offset + *len]);
                    }
                    Repr::Cons { .. } => {
                        for chunk in extra.chunks() {
                            bytes.append(chunk);
                        }
                    }
                }
                string.hash.set(0);
                true
            }
            Repr::Cons { .. } | Repr::Slice { .. } => false,
        }
    }

    #[must_use]
    pub(crate) fn slice(
        heap: &Heap,
        base: &ManagedRef<Self>,
        offset: usize,
        len: usize,
    ) -> ManagedRef<Self> {
        debug_assert!(
            offset.checked_add(len).is_some_and(|end| end <= base.len()),
            "whim-runtime: a slice must lie within its base: {offset} + {len} > {}",
            base.len()
        );
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        if matches!(unsafe { &*base.repr.get() }, Repr::Cons { .. }) {
            base.flatten();
        }
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        let (anchor, anchor_offset) = match unsafe { &*base.repr.get() } {
            Repr::Slice {
                base: inner,
                offset: inner_offset,
                ..
            } => (inner.clone(), *inner_offset + offset),
            _ => (base.clone(), offset),
        };
        ManagedRef::new_in(
            heap,
            Self {
                hash: Cell::new(0),
                repr: UnsafeCell::new(Repr::Slice {
                    base: anchor,
                    offset: anchor_offset,
                    len,
                }),
            },
        )
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        match unsafe { &*self.repr.get() } {
            Repr::Flat(bytes) => bytes.len(),
            Repr::Cons { len, .. } | Repr::Slice { len, .. } => *len,
        }
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn flatten(&self) -> &[u8] {
        if !self.is_flat() {
            // SAFETY: the live string owns this payload, and the VM serializes representation access.
            let buffer = unsafe { HeapBytes::from_fragments(self.len(), self.chunks()) };
            // SAFETY: the live string owns this payload, and the VM serializes representation access.
            unsafe { *self.repr.get() = Repr::Flat(buffer) };
        }
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        unsafe { self.flat_slice() }
    }

    pub(crate) fn handle_bytes(handle: &ManagedRef<Self>) -> &[u8] {
        if handle.is_flat() {
            // SAFETY: the live string owns this payload, and the VM serializes representation access.
            return unsafe { handle.flat_slice() };
        }
        handle.flatten()
    }

    /// Returns the buffer of an already-flat string.
    ///
    /// # Safety
    ///
    /// The representation must be [`Repr::Flat`].
    pub(crate) unsafe fn flat_slice(&self) -> &[u8] {
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        match unsafe { &*self.repr.get() } {
            Repr::Flat(bytes) => bytes.as_slice(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("flat_slice on a non-flat string") },
        }
    }

    /// Iterates leaf chunks without flattening or recursion.
    fn chunks(&self) -> Chunks<'_> {
        Chunks {
            stack: vec![ptr::from_ref(self)],
            _marker: PhantomData,
        }
    }

    fn contiguous_slice(&self) -> Option<&[u8]> {
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        match unsafe { &*self.repr.get() } {
            Repr::Flat(bytes) => Some(bytes.as_slice()),
            Repr::Slice { base, offset, len } => {
                // SAFETY: the live string owns this payload, and the VM serializes representation access.
                let bytes = unsafe { base.flat_slice() };
                Some(&bytes[*offset..*offset + *len])
            }
            Repr::Cons { .. } => None,
        }
    }

    pub(crate) fn hash64(&self, state: &HashState) -> u64 {
        let cached = self.hash.get();
        if cached != 0 {
            return cached;
        }

        #[expect(
            clippy::option_if_let_else,
            reason = "the contiguous path avoids constructing rope traversal state"
        )]
        let hash = if let Some(bytes) = self.contiguous_slice() {
            hash_bytes(state, bytes)
        } else {
            hash_fragments(state, self.len(), self.chunks())
        };
        self.hash.set(hash);
        hash
    }

    #[must_use]
    pub(crate) fn eq_bytes(&self, other: &Self) -> bool {
        if ptr::eq(self, other) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        self.cmp_bytes(other) == Ordering::Equal
    }

    #[must_use]
    pub(crate) fn cmp_bytes(&self, other: &Self) -> Ordering {
        if ptr::eq(self, other) {
            return Ordering::Equal;
        }
        if let (Some(left), Some(right)) = (self.contiguous_slice(), other.contiguous_slice()) {
            return left.cmp(right);
        }
        let mut left = self.chunks();
        let mut right = other.chunks();
        let mut left_chunk: &[u8] = &[];
        let mut right_chunk: &[u8] = &[];
        loop {
            while left_chunk.is_empty() {
                match left.next() {
                    Some(chunk) => left_chunk = chunk,
                    None => break,
                }
            }
            while right_chunk.is_empty() {
                match right.next() {
                    Some(chunk) => right_chunk = chunk,
                    None => break,
                }
            }
            match (left_chunk.is_empty(), right_chunk.is_empty()) {
                (true, true) => return Ordering::Equal,
                (true, false) => return Ordering::Less,
                (false, true) => return Ordering::Greater,
                (false, false) => {
                    let shared = left_chunk.len().min(right_chunk.len());
                    match left_chunk[..shared].cmp(&right_chunk[..shared]) {
                        Ordering::Equal => {
                            left_chunk = &left_chunk[shared..];
                            right_chunk = &right_chunk[shared..];
                        }
                        ordering => return ordering,
                    }
                }
            }
        }
    }

    fn is_flat(&self) -> bool {
        // SAFETY: the live string owns this payload, and the VM serializes representation access.
        matches!(unsafe { &*self.repr.get() }, Repr::Flat(_))
    }
}

/// An iterative walk over a string's leaf chunks.
struct Chunks<'string> {
    stack: Vec<*const ByteStringObject>,
    _marker: PhantomData<&'string ByteStringObject>,
}

impl<'string> Iterator for Chunks<'string> {
    type Item = &'string [u8];

    fn next(&mut self) -> Option<&'string [u8]> {
        while let Some(node) = self.stack.pop() {
            // SAFETY: the live string owns this payload, and the VM serializes representation access.
            let node = unsafe { &*node };
            // SAFETY: the live string owns this payload, and the VM serializes representation access.
            match unsafe { &*node.repr.get() } {
                // SAFETY: the live string owns this payload, and the VM serializes representation access.
                Repr::Flat(bytes) => return Some(unsafe { rebind(bytes.as_slice()) }),
                Repr::Slice { base, offset, len } => {
                    let base: &ByteStringObject = base;
                    // SAFETY: the live string owns this payload, and the VM serializes representation access.
                    match unsafe { &*base.repr.get() } {
                        Repr::Flat(bytes) => {
                            let window = &bytes.as_slice()[*offset..*offset + *len];
                            // SAFETY: the live string owns this payload, and the VM serializes representation access.
                            return Some(unsafe { rebind(window) });
                        }
                        // SAFETY: the surrounding invariant makes this path unreachable.
                        _ => unsafe { unreachable_invariant("a slice base is always flat") },
                    }
                }
                Repr::Cons { left, right, .. } => {
                    let left: &ByteStringObject = left;
                    let right: &ByteStringObject = right;
                    self.stack.push(ptr::from_ref(right));
                    self.stack.push(ptr::from_ref(left));
                }
            }
        }
        None
    }
}

/// # Safety
///
/// A chunk borrows a leaf's flat buffer, which the rope keeps alive through its
/// operand handles for as long as the root string the iterator borrows lives,
/// and no chunk is yielded across a mutation of that string.
const unsafe fn rebind<'string>(bytes: &[u8]) -> &'string [u8] {
    // SAFETY: the pointer and length share one live allocation.
    unsafe { slice::from_raw_parts(bytes.as_ptr(), bytes.len()) }
}

fn overlaps(left: &[u8], right: &[u8]) -> bool {
    let left = left.as_ptr_range();
    let right = right.as_ptr_range();
    left.start < right.end && right.start < left.end
}

pub(crate) fn hash_bytes(state: &HashState, bytes: &[u8]) -> u64 {
    if bytes.len() <= ShortString::CAPACITY {
        return state.hash_short_string(pack_short_string(bytes));
    }

    let mut hasher = state.string_hasher(bytes.len());
    for block in bytes.chunks(HASH_BLOCK_SIZE) {
        hasher.write(block);
    }
    hasher.finish()
}

fn hash_fragments<'string>(
    state: &HashState,
    len: usize,
    fragments: impl Iterator<Item = &'string [u8]>,
) -> u64 {
    let mut hasher = state.string_hasher(len);
    let mut pending = [0; HASH_BLOCK_SIZE];
    let mut pending_len = 0;

    for mut fragment in fragments {
        while !fragment.is_empty() {
            if pending_len == 0 && fragment.len() >= HASH_BLOCK_SIZE {
                hasher.write(&fragment[..HASH_BLOCK_SIZE]);
                fragment = &fragment[HASH_BLOCK_SIZE..];
                continue;
            }

            let copied = (HASH_BLOCK_SIZE - pending_len).min(fragment.len());
            pending[pending_len..pending_len + copied].copy_from_slice(&fragment[..copied]);
            pending_len += copied;
            fragment = &fragment[copied..];
            if pending_len == HASH_BLOCK_SIZE {
                hasher.write(&pending);
                pending_len = 0;
            }
        }
    }

    if pending_len != 0 {
        hasher.write(&pending[..pending_len]);
    }
    hasher.finish()
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::inline_always,
    reason = "the caller bounds inline strings to seven bytes"
)]
#[inline(always)]
fn pack_short_string(bytes: &[u8]) -> u64 {
    debug_assert!(bytes.len() <= ShortString::CAPACITY);
    let mut packed = [0; 8];
    packed[..bytes.len()].copy_from_slice(bytes);
    packed[7] = bytes.len() as u8;
    u64::from_le_bytes(packed)
}

impl Trace for ByteStringObject {
    fn type_tag() -> TypeTag {
        TypeTag::ByteString
    }

    fn enqueue_children(
        &mut self,
        _allocation: NonNull<HeapBox<()>>,
        queue: &mut DropQueue,
        mode: TeardownMode,
    ) {
        match mem::replace(self.repr.get_mut(), Repr::Flat(HeapBytes::empty())) {
            Repr::Flat(bytes) => queue.release_bytes(bytes),
            Repr::Cons { left, right, .. } => {
                queue.release_child(left, mode);
                queue.release_child(right, mode);
            }
            Repr::Slice { base, .. } => queue.release_child(base, mode),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::value::heap::Heap;
    use crate::value::string::ByteStringObject;
    use crate::value::string::hash_bytes;
    use crate::value::string::short::ShortString;

    const CONTENT: &[u8] = b"0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn hashes_equal_bytes_across_representations() {
        let heap = Heap::new();
        let flat = ByteStringObject::from_bytes(&heap, CONTENT);
        let left = ByteStringObject::from_bytes(&heap, &CONTENT[..19]);
        let right = ByteStringObject::from_bytes(&heap, &CONTENT[19..]);
        let rope = ByteStringObject::concat(&heap, &left, &right);

        let mut padded = b"prefix".to_vec();
        padded.extend_from_slice(CONTENT);
        padded.extend_from_slice(b"suffix");
        let base = ByteStringObject::from_vec(&heap, padded);
        let slice = ByteStringObject::slice(&heap, &base, 6, CONTENT.len());

        let expected = flat.hash64(heap.hash_state());
        assert_eq!(rope.hash64(heap.hash_state()), expected);
        assert_eq!(slice.hash64(heap.hash_state()), expected);
    }

    #[test]
    fn hashes_short_and_heap_strings_equally() {
        let heap = Heap::new();
        let Some(short) = ShortString::from_bytes(b"string") else {
            panic!("the fixture must fit in a short string");
        };
        let string = ByteStringObject::from_bytes(&heap, short.as_bytes());

        assert_eq!(
            short.hash64(heap.hash_state()),
            string.hash64(heap.hash_state())
        );
    }

    #[test]
    fn separates_legacy_djbx33a_collisions() {
        let heap = Heap::new();

        assert_ne!(
            hash_bytes(heap.hash_state(), b"Aa"),
            hash_bytes(heap.hash_state(), b"B@")
        );
    }
}
