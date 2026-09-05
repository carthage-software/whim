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
        // Artifact decoding borrows its input; interning retains independent heap storage.
        let (bytes, immortal): (&[u8], bool) = serde::Deserialize::deserialize(deserializer)?;
        let atom = heap.intern(bytes);
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
        let string = ByteStringObject::from_hashed_bytes(self, bytes, hash);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { string.raw_box().as_ref() }
            .header_ref()
            .set_interned();
        let pointer = string.raw_box();
        self.interner()
            .borrow_mut()
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            .insert_unique(hash, pointer, |&pointer| unsafe {
                pointer.as_ref().state_ref().hash64(state)
            });
        Atom(string)
    }

    /// Removes a dying string before its bytes are freed.
    pub(crate) fn unintern(&self, pointer: AtomBox) {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let hash = unsafe { pointer.as_ref().state_ref() }.hash64(self.hash_state());
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

#[cfg(test)]
mod tests {
    use bincode::ErrorKind;
    use bincode::Options;
    use serde_seeded::de::Seed;

    use super::Atom;
    use crate::value::heap::Heap;
    use crate::value::heap::metadata::TeardownMode;
    use crate::value::heap::metadata::TypeTag;

    fn decode(bytes: &[u8], heap: &Heap) -> bincode::Result<Atom> {
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .reject_trailing_bytes()
            .deserialize_seed(Seed::<Heap, Atom>::new(heap), bytes)
    }

    #[test]
    fn cached_atom_hashes_preserve_identity_through_growth_and_removal() {
        let heap = Heap::new();
        let names: Vec<_> = (0..128)
            .map(|index| {
                let mut name = format!("an_externally_stored_atom_name_{index}").into_bytes();
                name.extend_from_slice(b"\0\xff");
                name
            })
            .collect();
        let mut atoms: Vec<_> = names.iter().map(|name| Some(heap.intern(name))).collect();
        assert_eq!(heap.interner().borrow().len(), names.len());

        for (name, atom) in names.iter().zip(&atoms) {
            assert_eq!(
                &heap.intern(name),
                atom.as_ref()
                    .expect("every original atom is still retained"),
            );
        }

        for index in (0..atoms.len()).step_by(2) {
            drop(atoms[index].take());
        }
        assert_eq!(heap.interner().borrow().len(), names.len() / 2);

        for (index, name) in names.iter().enumerate() {
            let atom = heap.intern(name);
            assert_eq!(atom.as_bytes(), name.as_slice());
            assert_eq!(heap.intern(name), atom);
            if let Some(retained) = &atoms[index] {
                assert_eq!(&atom, retained);
            }
        }
        assert_eq!(heap.interner().borrow().len(), names.len() / 2);
        drop(atoms);
        assert!(heap.interner().borrow().is_empty());
    }

    #[test]
    fn artifact_atoms_preserve_their_encoding_identity_and_immortality() {
        for bytes in [b"".as_slice(), b"repeated\\atom\0\xff"] {
            for immortal in [false, true] {
                let mut encoded = bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .serialize(&(bytes, immortal))
                    .expect("the existing atom representation serializes");
                let heap = Heap::new();
                let first = decode(&encoded, &heap).expect("the atom decodes");
                let second = decode(&encoded, &heap).expect("the repeated atom decodes");
                assert_eq!(first, second, "repeated atoms keep interned identity");

                let round_trip = bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .serialize(&first)
                    .expect("the decoded atom serializes");
                assert_eq!(round_trip, encoded, "the immortal flag is preserved");

                encoded.fill(0);
                drop(encoded);
                assert_eq!(first.as_bytes(), bytes, "input storage is not retained");

                if immortal {
                    let allocation = first.0.raw_box();
                    drop(second);
                    drop(first);
                    heap.unintern(allocation);
                    // SAFETY: the isolated immortal atom is live in this heap,
                    // and both handles plus its interner entry are now gone.
                    // Immortality left its initial reference count unchanged.
                    unsafe {
                        assert_eq!(allocation.as_ref().header_ref().decrement(), 0);
                        heap.teardown_in_mode(
                            allocation.cast(),
                            TypeTag::ByteString,
                            TeardownMode::Full,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn malformed_artifact_atoms_keep_the_existing_decode_errors() {
        let encoded = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .serialize(&(b"artifact atom".as_slice(), false))
            .expect("the atom representation serializes");
        let mut malformed: Vec<Vec<u8>> = (0..encoded.len())
            .map(|length| encoded[..length].to_vec())
            .collect();
        let mut invalid_flag = encoded.clone();
        *invalid_flag
            .last_mut()
            .expect("the encoded tuple ends with a flag") = 2;
        malformed.push(invalid_flag);
        let mut trailing = encoded;
        trailing.push(0);
        malformed.push(trailing);
        malformed.push(u64::MAX.to_le_bytes().to_vec());

        let heap = Heap::new();
        for bytes in malformed {
            let previous = bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .reject_trailing_bytes()
                .deserialize::<(Vec<u8>, bool)>(&bytes)
                .expect_err("the original atom decoder rejects malformed input");
            let current = decode(&bytes, &heap)
                .expect_err("the borrowed atom decoder rejects malformed input");
            match (*current, *previous) {
                (ErrorKind::Io(current), ErrorKind::Io(previous)) => {
                    assert_eq!(current.kind(), previous.kind());
                }
                (
                    ErrorKind::InvalidBoolEncoding(current),
                    ErrorKind::InvalidBoolEncoding(previous),
                ) => assert_eq!(current, previous),
                (ErrorKind::Custom(current), ErrorKind::Custom(previous)) => {
                    assert_eq!(current, previous);
                }
                (current, previous) => {
                    panic!("unexpected atom decode errors: {current:?}, previously {previous:?}");
                }
            }
        }
    }
}
