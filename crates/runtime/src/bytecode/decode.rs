//! Heap-seeded decoding for compiled units.

use bincode::Options;
use serde::Deserializer;
use serde::de::DeserializeSeed;
use serde_seeded::DeserializeSeeded;

use crate::bytecode::unit::CompiledUnit;
use crate::value::heap::Heap;

mod adapter;

use crate::bytecode::decode::adapter::SeededDeserializer;

const MAX_SEQUENCE_PREALLOCATION: usize = 4096;

struct CompiledUnitSeed<'heap>(&'heap Heap);

impl<'de> DeserializeSeed<'de> for CompiledUnitSeed<'_> {
    type Value = CompiledUnit;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<CompiledUnit, D::Error> {
        CompiledUnit::deserialize_seeded(self.0, SeededDeserializer(deserializer))
    }
}

pub(crate) fn compiled_unit(bytes: &[u8], heap: &Heap) -> bincode::Result<CompiledUnit> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize_seed(CompiledUnitSeed(heap), bytes)
}

pub(crate) mod pairs {
    use std::fmt;
    use std::marker::PhantomData;

    use serde::Deserializer;
    use serde::de::DeserializeSeed;
    use serde::de::Error as DeserializeError;
    use serde::de::SeqAccess;
    use serde::de::Visitor;
    use serde_seeded::DeserializeSeeded;
    use serde_seeded::de::Seed;

    struct PairSeed<'seed, Q: ?Sized, A, B> {
        seed: &'seed Q,
        marker: PhantomData<(A, B)>,
    }

    impl<'de, Q, A, B> DeserializeSeed<'de> for PairSeed<'_, Q, A, B>
    where
        Q: ?Sized,
        A: DeserializeSeeded<'de, Q>,
        B: DeserializeSeeded<'de, Q>,
    {
        type Value = (A, B);

        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(A, B), D::Error> {
            struct PairVisitor<'seed, Q: ?Sized, A, B> {
                seed: &'seed Q,
                marker: PhantomData<(A, B)>,
            }

            impl<'de, Q, A, B> Visitor<'de> for PairVisitor<'_, Q, A, B>
            where
                Q: ?Sized,
                A: DeserializeSeeded<'de, Q>,
                B: DeserializeSeeded<'de, Q>,
            {
                type Value = (A, B);

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a pair")
                }

                fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<(A, B), S::Error> {
                    let first = sequence
                        .next_element_seed(Seed::<Q, A>::new(self.seed))?
                        .ok_or_else(|| DeserializeError::invalid_length(0, &self))?;
                    let second = sequence
                        .next_element_seed(Seed::<Q, B>::new(self.seed))?
                        .ok_or_else(|| DeserializeError::invalid_length(1, &self))?;
                    Ok((first, second))
                }
            }

            deserializer.deserialize_tuple(
                2,
                PairVisitor {
                    seed: self.seed,
                    marker: PhantomData,
                },
            )
        }
    }

    pub(crate) fn deserialize_seeded<'de, Q, A, B, D>(
        seed: &Q,
        deserializer: D,
    ) -> Result<Vec<(A, B)>, D::Error>
    where
        Q: ?Sized,
        A: DeserializeSeeded<'de, Q>,
        B: DeserializeSeeded<'de, Q>,
        D: Deserializer<'de>,
    {
        struct PairVectorVisitor<'seed, Q: ?Sized, A, B> {
            seed: &'seed Q,
            marker: PhantomData<(A, B)>,
        }

        impl<'de, Q, A, B> Visitor<'de> for PairVectorVisitor<'_, Q, A, B>
        where
            Q: ?Sized,
            A: DeserializeSeeded<'de, Q>,
            B: DeserializeSeeded<'de, Q>,
        {
            type Value = Vec<(A, B)>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a sequence of pairs")
            }

            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut sequence: S,
            ) -> Result<Vec<(A, B)>, S::Error> {
                let mut result = Vec::with_capacity(
                    sequence
                        .size_hint()
                        .unwrap_or(0)
                        .min(super::MAX_SEQUENCE_PREALLOCATION),
                );
                while let Some(value) = sequence.next_element_seed(PairSeed {
                    seed: self.seed,
                    marker: PhantomData,
                })? {
                    result.push(value);
                }
                Ok(result)
            }
        }

        deserializer.deserialize_seq(PairVectorVisitor {
            seed,
            marker: PhantomData,
        })
    }

    pub(crate) mod optional {
        use std::fmt;
        use std::marker::PhantomData;

        use serde::Deserializer;
        use serde::de::DeserializeSeed;
        use serde::de::Visitor;
        use serde_seeded::DeserializeSeeded;

        use crate::bytecode::decode::pairs::PairSeed;

        pub(crate) fn deserialize_seeded<'de, Q, A, B, D>(
            seed: &Q,
            deserializer: D,
        ) -> Result<Option<(A, B)>, D::Error>
        where
            Q: ?Sized,
            A: DeserializeSeeded<'de, Q>,
            B: DeserializeSeeded<'de, Q>,
            D: Deserializer<'de>,
        {
            struct OptionalPairVisitor<'seed, Q: ?Sized, A, B> {
                seed: &'seed Q,
                marker: PhantomData<(A, B)>,
            }

            impl<'de, Q, A, B> Visitor<'de> for OptionalPairVisitor<'_, Q, A, B>
            where
                Q: ?Sized,
                A: DeserializeSeeded<'de, Q>,
                B: DeserializeSeeded<'de, Q>,
            {
                type Value = Option<(A, B)>;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("an optional pair")
                }

                fn visit_none<E>(self) -> Result<Option<(A, B)>, E> {
                    Ok(None)
                }

                fn visit_some<D: Deserializer<'de>>(
                    self,
                    deserializer: D,
                ) -> Result<Option<(A, B)>, D::Error> {
                    PairSeed {
                        seed: self.seed,
                        marker: PhantomData,
                    }
                    .deserialize(deserializer)
                    .map(Some)
                }
            }

            deserializer.deserialize_option(OptionalPairVisitor {
                seed,
                marker: PhantomData,
            })
        }
    }
}

pub(crate) mod atom_i32_pairs {
    use std::fmt;
    use std::marker::PhantomData;

    use serde::Deserializer;
    use serde::de::DeserializeSeed;
    use serde::de::Error as DeserializeError;
    use serde::de::SeqAccess;
    use serde::de::Visitor;
    use serde_seeded::de::Seed;

    use crate::value::atom::Atom;
    use crate::value::heap::Heap;

    struct PairSeed<'heap>(&'heap Heap);

    impl<'de> DeserializeSeed<'de> for PairSeed<'_> {
        type Value = (Atom, i32);

        fn deserialize<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<(Atom, i32), D::Error> {
            struct PairVisitor<'heap>(&'heap Heap);

            impl<'de> Visitor<'de> for PairVisitor<'_> {
                type Value = (Atom, i32);

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("an atom and offset pair")
                }

                fn visit_seq<S: SeqAccess<'de>>(
                    self,
                    mut sequence: S,
                ) -> Result<(Atom, i32), S::Error> {
                    let atom = sequence
                        .next_element_seed(Seed::<Heap, Atom>::new(self.0))?
                        .ok_or_else(|| DeserializeError::invalid_length(0, &self))?;
                    let offset = sequence
                        .next_element::<i32>()?
                        .ok_or_else(|| DeserializeError::invalid_length(1, &self))?;
                    Ok((atom, offset))
                }
            }

            deserializer.deserialize_tuple(2, PairVisitor(self.0))
        }
    }

    pub(crate) fn deserialize_seeded<'de, D: Deserializer<'de>>(
        heap: &Heap,
        deserializer: D,
    ) -> Result<Vec<(Atom, i32)>, D::Error> {
        struct PairVectorVisitor<'heap>(&'heap Heap, PhantomData<(Atom, i32)>);

        impl<'de> Visitor<'de> for PairVectorVisitor<'_> {
            type Value = Vec<(Atom, i32)>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a sequence of atom and offset pairs")
            }

            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut sequence: S,
            ) -> Result<Vec<(Atom, i32)>, S::Error> {
                let mut result = Vec::with_capacity(
                    sequence
                        .size_hint()
                        .unwrap_or(0)
                        .min(super::MAX_SEQUENCE_PREALLOCATION),
                );
                while let Some(value) = sequence.next_element_seed(PairSeed(self.0))? {
                    result.push(value);
                }
                Ok(result)
            }
        }

        deserializer.deserialize_seq(PairVectorVisitor(heap, PhantomData))
    }
}
