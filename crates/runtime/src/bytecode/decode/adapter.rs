//! Adapts sequence-oriented formats to `serde-seeded`'s struct visitors.

use std::fmt;

use serde::Deserializer;
use serde::de::DeserializeSeed;
use serde::de::EnumAccess;
use serde::de::Error as DeserializeError;
use serde::de::IntoDeserializer;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::VariantAccess;
use serde::de::Visitor;

pub(in crate::bytecode::decode) struct SeededDeserializer<D>(pub D);

macro_rules! delegate {
    ($method:ident) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, D::Error>
        where
            V: Visitor<'de>,
        {
            self.0.$method(visitor)
        }
    };
    ($method:ident, $($argument:ident: $type:ty),+) => {
        fn $method<V>(self, $($argument: $type,)+ visitor: V) -> Result<V::Value, D::Error>
        where
            V: Visitor<'de>,
        {
            self.0.$method($($argument,)+ visitor)
        }
    };
}

impl<'de, D> Deserializer<'de> for SeededDeserializer<D>
where
    D: Deserializer<'de>,
{
    type Error = D::Error;

    delegate!(deserialize_any);
    delegate!(deserialize_bool);
    delegate!(deserialize_i8);
    delegate!(deserialize_i16);
    delegate!(deserialize_i32);
    delegate!(deserialize_i64);
    delegate!(deserialize_i128);
    delegate!(deserialize_u8);
    delegate!(deserialize_u16);
    delegate!(deserialize_u32);
    delegate!(deserialize_u64);
    delegate!(deserialize_u128);
    delegate!(deserialize_f32);
    delegate!(deserialize_f64);
    delegate!(deserialize_char);
    delegate!(deserialize_str);
    delegate!(deserialize_string);
    delegate!(deserialize_bytes);
    delegate!(deserialize_byte_buf);
    delegate!(deserialize_unit);
    delegate!(deserialize_unit_struct, name: &'static str);
    delegate!(deserialize_identifier);
    delegate!(deserialize_ignored_any);

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0.deserialize_option(OptionVisitor(visitor))
    }

    fn deserialize_newtype_struct<V>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0
            .deserialize_newtype_struct(name, NewtypeVisitor(visitor))
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0.deserialize_seq(SequenceVisitor(visitor))
    }

    fn deserialize_tuple<V>(self, length: usize, visitor: V) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0.deserialize_tuple(length, SequenceVisitor(visitor))
    }

    fn deserialize_tuple_struct<V>(
        self,
        name: &'static str,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0
            .deserialize_tuple_struct(name, length, SequenceVisitor(visitor))
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0.deserialize_map(MapVisitor(visitor))
    }

    fn deserialize_struct<V>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0
            .deserialize_struct(name, fields, StructVisitor(visitor, fields.len()))
    }

    fn deserialize_enum<V>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error>
    where
        V: Visitor<'de>,
    {
        self.0
            .deserialize_enum(name, variants, EnumVisitor(visitor))
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

struct WrappingSeed<S>(S);

impl<'de, S> DeserializeSeed<'de> for WrappingSeed<S>
where
    S: DeserializeSeed<'de>,
{
    type Value = S::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<S::Value, D::Error> {
        self.0.deserialize(SeededDeserializer(deserializer))
    }
}

struct SequenceAccess<A>(A);

impl<'de, A> SeqAccess<'de> for SequenceAccess<A>
where
    A: SeqAccess<'de>,
{
    type Error = A::Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, A::Error>
    where
        T: DeserializeSeed<'de>,
    {
        self.0.next_element_seed(WrappingSeed(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

struct MapAccessAdapter<A>(A);

impl<'de, A> MapAccess<'de> for MapAccessAdapter<A>
where
    A: MapAccess<'de>,
{
    type Error = A::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error>
    where
        K: DeserializeSeed<'de>,
    {
        self.0.next_key_seed(WrappingSeed(seed))
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, A::Error>
    where
        V: DeserializeSeed<'de>,
    {
        self.0.next_value_seed(WrappingSeed(seed))
    }

    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}

struct SequenceMapAccess<A> {
    sequence: A,
    index: usize,
    length: usize,
}

impl<'de, A> MapAccess<'de> for SequenceMapAccess<A>
where
    A: SeqAccess<'de>,
{
    type Error = A::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, A::Error>
    where
        K: DeserializeSeed<'de>,
    {
        if self.index == self.length {
            return Ok(None);
        }

        let index = self.index as u64;
        seed.deserialize(index.into_deserializer()).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, A::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let value = self
            .sequence
            .next_element_seed(WrappingSeed(seed))?
            .ok_or_else(|| DeserializeError::invalid_length(self.index, &"a complete struct"))?;
        self.index += 1;
        Ok(value)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.length - self.index)
    }
}

struct OptionVisitor<V>(V);

impl<'de, V> Visitor<'de> for OptionVisitor<V>
where
    V: Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_none<E>(self) -> Result<V::Value, E>
    where
        E: DeserializeError,
    {
        self.0.visit_none()
    }

    fn visit_unit<E>(self) -> Result<V::Value, E>
    where
        E: DeserializeError,
    {
        self.0.visit_unit()
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<V::Value, D::Error> {
        self.0.visit_some(SeededDeserializer(deserializer))
    }
}

struct NewtypeVisitor<V>(V);

impl<'de, V> Visitor<'de> for NewtypeVisitor<V>
where
    V: Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<V::Value, D::Error> {
        self.0
            .visit_newtype_struct(SeededDeserializer(deserializer))
    }
}

struct SequenceVisitor<V>(V);

impl<'de, V> Visitor<'de> for SequenceVisitor<V>
where
    V: Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, sequence: A) -> Result<V::Value, A::Error> {
        self.0.visit_seq(SequenceAccess(sequence))
    }
}

struct MapVisitor<V>(V);

impl<'de, V> Visitor<'de> for MapVisitor<V>
where
    V: Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(MapAccessAdapter(map))
    }
}

struct StructVisitor<V>(V, usize);

impl<'de, V> Visitor<'de> for StructVisitor<V>
where
    V: Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, sequence: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(SequenceMapAccess {
            sequence,
            index: 0,
            length: self.1,
        })
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.0.visit_map(MapAccessAdapter(map))
    }
}

struct EnumVisitor<V>(V);

impl<'de, V> Visitor<'de> for EnumVisitor<V>
where
    V: Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_enum<A: EnumAccess<'de>>(self, access: A) -> Result<V::Value, A::Error> {
        self.0.visit_enum(EnumAccessAdapter(access))
    }
}

struct EnumAccessAdapter<A>(A);

impl<'de, A> EnumAccess<'de> for EnumAccessAdapter<A>
where
    A: EnumAccess<'de>,
{
    type Error = A::Error;
    type Variant = VariantAccessAdapter<A::Variant>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), A::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let (value, variant) = self.0.variant_seed(seed)?;
        Ok((value, VariantAccessAdapter(variant)))
    }
}

struct VariantAccessAdapter<A>(A);

impl<'de, A> VariantAccess<'de> for VariantAccessAdapter<A>
where
    A: VariantAccess<'de>,
{
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.unit_variant()
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, A::Error>
    where
        T: DeserializeSeed<'de>,
    {
        self.0.newtype_variant_seed(WrappingSeed(seed))
    }

    fn tuple_variant<V>(self, length: usize, visitor: V) -> Result<V::Value, A::Error>
    where
        V: Visitor<'de>,
    {
        self.0.tuple_variant(length, SequenceVisitor(visitor))
    }

    fn struct_variant<V>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error>
    where
        V: Visitor<'de>,
    {
        self.0
            .struct_variant(fields, StructVisitor(visitor, fields.len()))
    }
}
