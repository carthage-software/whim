//! The engine-scoped interner for immutable, reference-counted names.

use std::borrow::Cow;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::ptr::NonNull;

use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde_seeded::DeserializeSeeded;

use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::heap::metadata::HeapBox;
use crate::value::string::ByteStringObject;
use crate::value::string::hash_bytes;

pub(in crate::value) type AtomBox = NonNull<HeapBox<ByteStringObject>>;

pub(crate) struct Atom(ManagedRef<ByteStringObject>);

impl Serialize for Atom {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let immortal = unsafe { self.0.raw_box().as_ref() }
            .header_ref()
            .is_immortal();
        (self.as_bytes(), immortal).serialize(serializer)
    }
}

impl<'de> DeserializeSeeded<'de, Heap> for Atom {
    fn deserialize_seeded<D: Deserializer<'de>>(
        heap: &Heap,
        deserializer: D,
    ) -> Result<Self, D::Error> {
        let (bytes, immortal): (Vec<u8>, bool) = serde::Deserialize::deserialize(deserializer)?;
        let atom = heap.intern(&bytes);
        if immortal {
            atom.make_immortal();
        }
        Ok(atom)
    }
}

impl fmt::Debug for Atom {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Atom({:?})", self.to_string_lossy())
    }
}

impl fmt::Display for Atom {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_string_lossy())
    }
}

impl Clone for Atom {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl PartialEq for Atom {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl Eq for Atom {}

impl Hash for Atom {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.0.raw_box().addr().get());
    }
}

/// # Safety
///
/// `pointer` must reference a live interned box, whose payload is always flat.
unsafe fn box_bytes<'a>(pointer: AtomBox) -> &'a [u8] {
    // SAFETY: the tag and managed handle prove the payload type and lifetime.
    unsafe { pointer.as_ref().state_ref().flat_slice() }
}

impl Heap {
    /// Interns bytes with pointer identity within this engine.
    #[must_use]
    pub(crate) fn intern(&self, bytes: &[u8]) -> Atom {
        let state = self.hash_state();
        let hash = hash_bytes(state, bytes);
        if let Some(&pointer) = self
            .interner()
            .borrow()
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            .find(hash, |&pointer| unsafe { box_bytes(pointer) } == bytes)
        {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            return Atom(unsafe { ManagedRef::retain_raw(pointer) });
        }
        let string = ByteStringObject::from_bytes(self, bytes);
        string.hash64(state);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { string.raw_box().as_ref() }
            .header_ref()
            .set_interned();
        let pointer = string.raw_box();
        self.interner()
            .borrow_mut()
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            .insert_unique(hash, pointer, |&pointer| unsafe {
                hash_bytes(state, box_bytes(pointer))
            });
        Atom(string)
    }

    /// Removes a dying string before its bytes are freed.
    pub(crate) fn unintern(&self, pointer: AtomBox) {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let bytes = unsafe { box_bytes(pointer) };
        let hash = hash_bytes(self.hash_state(), bytes);
        let mut interner = self.interner().borrow_mut();
        if let Ok(entry) = interner.find_entry(hash, |&candidate| candidate == pointer) {
            entry.remove();
        }
        if interner.is_empty() {
            *interner = Self::empty_interner();
        }
    }
}

impl Atom {
    #[must_use]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { self.0.flat_slice() }
    }

    #[must_use]
    pub(crate) fn to_string_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(self.as_bytes())
    }

    #[must_use]
    pub(crate) fn to_handle(&self) -> ManagedRef<ByteStringObject> {
        self.0.clone()
    }

    /// Makes the atom's storage immortal.
    pub(crate) fn make_immortal(&self) {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { self.0.raw_box().as_ref() }
            .header_ref()
            .set_immortal();
    }

    /// Borrows the atom's string handle without retaining it.
    #[must_use]
    pub(crate) const fn as_handle(&self) -> &ManagedRef<ByteStringObject> {
        &self.0
    }
}
