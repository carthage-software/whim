//! SIMD JSON encoding and decoding over sonic-rs, used by `Whim\Json`.

use std::str::from_utf8;

use serde::Serialize;
use serde::Serializer;
use serde::ser::Error as _;
use serde::ser::SerializeMap;
use serde::ser::SerializeSeq;
use sonic_rs::JsonContainerTrait;
use sonic_rs::JsonType;
use sonic_rs::JsonValueTrait;

use whim_macros::whim_class;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::dict::keys::KeyRef;
use crate::value::string::ByteStringObject;

const JSON_ERROR: &str = "Whim\\_Private\\JsonError";
const DEPTH_LIMIT: usize = 512;

#[whim_class("Whim\\_Private\\JsonError", final)]
#[whim_extends("Whim\\Unwind\\Error")]
pub(crate) struct JsonError;

#[whim_methods]
impl JsonError {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}
}

struct JsonSource<'value> {
    value: &'value Value,
    depth: usize,
}

impl JsonSource<'_> {
    const fn nested<'value>(&self, value: &'value Value) -> JsonSource<'value> {
        JsonSource {
            value,
            depth: self.depth + 1,
        }
    }
}

impl Serialize for JsonSource<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.depth > DEPTH_LIMIT {
            return Err(S::Error::custom("the value nests deeper than 512 levels"));
        }

        match self.value.transparent() {
            ValueView::Null => serializer.serialize_unit(),
            ValueView::Bool(value) => serializer.serialize_bool(*value),
            ValueView::Int(value) => serializer.serialize_i64(*value),
            ValueView::Float(value) if value.is_finite() => serializer.serialize_f64(*value),
            ValueView::Float(_) => Err(S::Error::custom(
                "the value holds a float JSON cannot represent",
            )),
            ValueView::String(handle) => {
                serialize_string(ByteStringObject::handle_bytes(handle), serializer)
            }
            ValueView::ShortString(short) => serialize_string(short.as_bytes(), serializer),
            ValueView::Vec(handle) => {
                let mut sequence = serializer.serialize_seq(Some(handle.len()))?;
                let mut index = 0;
                while let Some(element) = handle.get(index) {
                    sequence.serialize_element(&self.nested(element))?;
                    index += 1;
                }
                sequence.end()
            }
            ValueView::Dict(handle) => {
                let mut map = serializer.serialize_map(Some(handle.len()))?;
                for (key, value) in handle.iter() {
                    let bytes = match &key {
                        KeyRef::String(string) => ByteStringObject::handle_bytes(string),
                        KeyRef::ShortString(short) => short.as_bytes(),
                        KeyRef::Int(_) | KeyRef::Bool(_) => {
                            return Err(S::Error::custom(
                                "the value holds a dictionary key that is not a string",
                            ));
                        }
                    };

                    let Ok(text) = from_utf8(bytes) else {
                        return Err(S::Error::custom(
                            "the value holds a dictionary key that is not valid UTF-8",
                        ));
                    };

                    map.serialize_entry(text, &self.nested(value))?;
                }

                map.end()
            }
            _ => Err(S::Error::custom("the value cannot be encoded as JSON")),
        }
    }
}

fn serialize_string<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    let Ok(text) = from_utf8(bytes) else {
        return Err(S::Error::custom(
            "the value holds a string that is not valid UTF-8",
        ));
    };

    serializer.serialize_str(text)
}

#[whim_function("Whim\\_Private\\json_encode(mixed $value, bool $pretty): (string&!'')")]
pub(crate) fn json_encode(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    // SAFETY: built-in dispatch checked this argument against the declaration.
    let value = unsafe { arguments.value_unchecked(0) };
    let pretty = arguments.bool(1);
    let source = JsonSource { value, depth: 0 };

    let encoded = if pretty {
        sonic_rs::to_vec_pretty(&source)
    } else {
        sonic_rs::to_vec(&source)
    };

    match encoded {
        Ok(bytes) => Ok(Value::from_string_vec(context.vm.heap(), bytes)),
        Err(error) => {
            let class = context.vm.intern(JSON_ERROR.as_bytes());
            Err(context.vm.throw(class, &error.to_string(), 0))
        }
    }
}

#[whim_function("Whim\\_Private\\json_decode(string $json): mixed")]
pub(crate) fn json_decode(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    match sonic_rs::from_slice::<sonic_rs::Value>(bytes) {
        Ok(node) => convert(context, &node),
        Err(error) => {
            let class = context.vm.intern(JSON_ERROR.as_bytes());
            let offset = i64::try_from(error.offset()).unwrap_or(i64::MAX);
            Err(context.vm.throw(class, &error.to_string(), offset))
        }
    }
}

fn convert(context: &mut Context<'_, '_, '_>, node: &sonic_rs::Value) -> Result<Value, Throw> {
    match node.get_type() {
        JsonType::Null => Ok(Value::null()),
        JsonType::Boolean => Ok(Value::bool(node.is_true())),
        JsonType::Number => {
            if let Some(value) = node.as_i64() {
                return Ok(Value::int(value));
            }

            let Some(value) = node.as_f64() else {
                let class = context.vm.intern(JSON_ERROR.as_bytes());
                return Err(context.vm.throw(
                    class,
                    "the JSON number is outside Whim's numeric range",
                    0,
                ));
            };
            Ok(Value::float(value))
        }
        JsonType::String => {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let string = unsafe {
                unwrap_option_invariant(node.as_str(), "a JSON string node contains a string")
            };
            Ok(context.string(string.as_bytes()))
        }
        JsonType::Array => {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let source = unsafe {
                unwrap_option_invariant(node.as_array(), "a JSON array node contains an array")
            };
            let mut elements = Vec::with_capacity(source.len());
            for element in source {
                elements.push(convert(context, element)?);
            }

            Ok(context.vec(elements))
        }
        JsonType::Object => {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let source = unsafe {
                unwrap_option_invariant(node.as_object(), "a JSON object node contains an object")
            };
            let mut entries = Vec::with_capacity(source.len());
            for (key, value) in source {
                let key = context.string(key.as_bytes());
                entries.push((key, convert(context, value)?));
            }

            Ok(context.dict(entries))
        }
    }
}
