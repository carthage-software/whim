//! Direct BSON encoding and decoding.

use std::collections::HashSet;
use std::fmt;
use std::str;

use bson::Bson;
use bson::RawBsonRef;
use bson::error::Error as BsonError;
use bson::oid::ObjectId as BsonObjectId;
use bson::raw::RawArray;
use bson::raw::RawDocument;
use serde::Deserialize;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::symbols::SymbolKind;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::dict::DictObject;
use crate::value::dict::keys::KeyRef;
use crate::value::newtype::NewtypeId;
use crate::value::object::ClassId;
use crate::value::object::InstanceObject;
use crate::value::object::TypeEnvironmentId;
use crate::value::string::ByteStringObject;
use crate::value::vec::VecObject;

const ENCODING_EXCEPTION: &str = "Whim\\BSON\\EncodingException";
const DECODING_EXCEPTION: &str = "Whim\\BSON\\DecodingException";
const INT32: &[u8] = b"Whim\\BSON\\Int32";
const JAVASCRIPT: &[u8] = b"Whim\\BSON\\JavaScript";
const SYMBOL: &[u8] = b"Whim\\BSON\\Symbol";
const BINARY: &[u8] = b"Whim\\BSON\\Binary";
const DB_POINTER: &[u8] = b"Whim\\BSON\\DBPointer";
const DECIMAL128: &[u8] = b"Whim\\BSON\\Decimal128";
const JAVASCRIPT_WITH_SCOPE: &[u8] = b"Whim\\BSON\\JavaScriptWithScope";
const MARKER: &[u8] = b"Whim\\BSON\\Marker";
const OBJECT_ID: &[u8] = b"Whim\\BSON\\ObjectId";
const REGULAR_EXPRESSION: &[u8] = b"Whim\\BSON\\RegularExpression";
const TIMESTAMP: &[u8] = b"Whim\\BSON\\Timestamp";
const SYSTEM_TIME: &[u8] = b"Whim\\Time\\SystemTime";
const MAXIMUM_DEPTH: usize = 128;
const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;
const NANOSECONDS_PER_MILLISECOND: i64 = 1_000_000;
const MILLISECONDS_PER_SECOND: i64 = 1_000;

#[derive(Debug)]
struct CodecError(String);

impl CodecError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<BsonError> for CodecError {
    fn from(error: BsonError) -> Self {
        Self(error.to_string())
    }
}

#[derive(Deserialize)]
struct OwnedDbPointerBody {
    #[serde(rename = "$ref")]
    namespace: String,
    #[serde(rename = "$id")]
    identifier: BsonObjectId,
}

#[derive(Deserialize)]
struct OwnedDbPointerEnvelope {
    #[serde(rename = "$dbPointer")]
    pointer: OwnedDbPointerBody,
}

#[whim_function(
    "Whim\\_Private\\bson_encode(mixed $document): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn encode(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    // SAFETY: built-in dispatch checked this argument against the declaration.
    let value = unsafe { arguments.value_unchecked(0) };
    let mut bytes = Vec::new();
    let result = match value.transparent() {
        ValueView::Dict(document) => write_document(context, document, 0, &mut bytes),
        _ => Err(CodecError::new("the BSON root value must be a document")),
    };
    match result {
        Ok(()) => Ok(Value::from_string_vec(context.vm.heap(), bytes)),
        Err(error) => Err(throw(context, ENCODING_EXCEPTION, &error)),
    }
}

#[whim_function(
    "Whim\\_Private\\bson_decode(string $bytes): mixed",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn decode(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let raw = match RawDocument::from_bytes(bytes) {
        Ok(raw) => raw,
        Err(error) => {
            return Err(throw(
                context,
                DECODING_EXCEPTION,
                &CodecError::new(error.to_string()),
            ));
        }
    };
    match decode_raw_document(context, raw, 0) {
        Ok(value) => Ok(value),
        Err(error) => Err(throw(context, DECODING_EXCEPTION, &error)),
    }
}

fn write_document(
    context: &mut Context<'_, '_, '_>,
    document: &DictObject,
    depth: usize,
    bytes: &mut Vec<u8>,
) -> Result<(), CodecError> {
    require_depth(depth)?;
    let start = begin_container(bytes);
    for (key, value) in document.iter() {
        let element_type = bytes.len();
        bytes.push(0);
        match key {
            KeyRef::String(key) => {
                write_cstring(
                    ByteStringObject::handle_bytes(key),
                    "a BSON field name",
                    bytes,
                )?;
            }
            KeyRef::ShortString(key) => {
                write_cstring(key.as_bytes(), "a BSON field name", bytes)?;
            }
            KeyRef::Int(_) | KeyRef::Bool(_) => {
                return Err(CodecError::new("document keys in BSON must be strings"));
            }
        }
        bytes[element_type] = write_value(context, value, depth, bytes)?;
    }
    finish_container(start, bytes)
}

fn write_array(
    context: &mut Context<'_, '_, '_>,
    values: &VecObject,
    depth: usize,
    bytes: &mut Vec<u8>,
) -> Result<(), CodecError> {
    require_depth(depth)?;
    let start = begin_container(bytes);
    let mut index_buffer = itoa::Buffer::new();
    for (index, value) in values.iter().enumerate() {
        let element_type = bytes.len();
        bytes.push(0);
        bytes.extend_from_slice(index_buffer.format(index).as_bytes());
        bytes.push(0);
        bytes[element_type] = write_value(context, value, depth, bytes)?;
    }
    finish_container(start, bytes)
}

fn write_value(
    context: &mut Context<'_, '_, '_>,
    value: &Value,
    depth: usize,
    bytes: &mut Vec<u8>,
) -> Result<u8, CodecError> {
    if is_newtype(context, value, INT32) {
        let value = value
            .as_int()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| CodecError::new("a BSON Int32 is outside the signed 32-bit range"))?;
        bytes.extend_from_slice(&value.to_le_bytes());
        return Ok(0x10);
    }
    if is_newtype(context, value, JAVASCRIPT) {
        write_string(value_bytes(value)?, "BSON JavaScript", bytes)?;
        return Ok(0x0d);
    }
    if is_newtype(context, value, SYMBOL) {
        write_string(value_bytes(value)?, "a BSON symbol", bytes)?;
        return Ok(0x0e);
    }

    match value.transparent() {
        ValueView::Null => Ok(0x0a),
        ValueView::Bool(value) => {
            bytes.push(u8::from(*value));
            Ok(0x08)
        }
        ValueView::Int(value) => {
            bytes.extend_from_slice(&value.to_le_bytes());
            Ok(0x12)
        }
        ValueView::Float(value) => {
            bytes.extend_from_slice(&value.to_le_bytes());
            Ok(0x01)
        }
        ValueView::String(_) | ValueView::ShortString(_) => {
            write_string(value_bytes(value)?, "a BSON string", bytes)?;
            Ok(0x02)
        }
        ValueView::Vec(values) => {
            write_array(context, values, depth + 1, bytes)?;
            Ok(0x04)
        }
        ValueView::Dict(document) => {
            write_document(context, document, depth + 1, bytes)?;
            Ok(0x03)
        }
        ValueView::Object(object) => write_object(context, object, depth, bytes),
        _ => Err(CodecError::new("the value cannot be encoded as BSON")),
    }
}

fn write_object(
    context: &mut Context<'_, '_, '_>,
    object: &InstanceObject,
    depth: usize,
    bytes: &mut Vec<u8>,
) -> Result<u8, CodecError> {
    match class_name(context, object) {
        BINARY => {
            write_binary(object, bytes)?;
            Ok(0x05)
        }
        OBJECT_ID => {
            bytes.extend_from_slice(&object_id(object)?.bytes());
            Ok(0x07)
        }
        SYSTEM_TIME => {
            bytes.extend_from_slice(&system_time(object)?.timestamp_millis().to_le_bytes());
            Ok(0x09)
        }
        REGULAR_EXPRESSION => {
            let pattern = object.read_slot(0);
            let options = object.read_slot(1);
            write_cstring(
                value_bytes(&pattern)?,
                "a BSON regular expression pattern",
                bytes,
            )?;
            write_cstring(
                value_bytes(&options)?,
                "BSON regular expression options",
                bytes,
            )?;
            Ok(0x0b)
        }
        TIMESTAMP => {
            let seconds = uint32(&object.read_slot(0), "a BSON timestamp second")?;
            let increment = uint32(&object.read_slot(1), "a BSON timestamp increment")?;
            bytes.extend_from_slice(&increment.to_le_bytes());
            bytes.extend_from_slice(&seconds.to_le_bytes());
            Ok(0x11)
        }
        DECIMAL128 => {
            let value = object.read_slot(0);
            let value = value
                .as_string_bytes()
                .filter(|bytes| bytes.len() == 16)
                .ok_or_else(|| CodecError::new("the BSON decimal128 data must contain 16 bytes"))?;
            bytes.extend_from_slice(value);
            Ok(0x13)
        }
        JAVASCRIPT_WITH_SCOPE => {
            require_depth(depth + 1)?;
            let start = bytes.len();
            bytes.extend_from_slice(&[0; 4]);
            let code = object.read_slot(0);
            write_string(value_bytes(&code)?, "BSON JavaScript", bytes)?;
            let scope = object.read_slot(1);
            let ValueView::Dict(scope) = scope.transparent() else {
                return Err(CodecError::new(
                    "a BSON JavaScript scope must be a document",
                ));
            };
            write_document(context, scope, depth + 1, bytes)?;
            patch_length(start, bytes, "BSON JavaScript with scope")?;
            Ok(0x0f)
        }
        DB_POINTER => {
            let namespace = object.read_slot(0);
            write_string(
                value_bytes(&namespace)?,
                "a BSON DB pointer namespace",
                bytes,
            )?;
            let identifier = object.read_slot(1);
            let identifier = identifier
                .as_object()
                .ok_or_else(|| CodecError::new("a BSON DB pointer needs an object identifier"))?;
            bytes.extend_from_slice(&object_id(identifier)?.bytes());
            Ok(0x0c)
        }
        MARKER => match object.read_slot(0).as_string_bytes() {
            Some(b"Undefined") => Ok(0x06),
            Some(b"MinKey") => Ok(0xff),
            Some(b"MaxKey") => Ok(0x7f),
            _ => Err(CodecError::new("unknown BSON marker")),
        },
        _ => Err(CodecError::new("the object cannot be encoded as BSON")),
    }
}

fn write_binary(object: &InstanceObject, bytes: &mut Vec<u8>) -> Result<(), CodecError> {
    let value = object.read_slot(0);
    let subtype = object.read_slot(1);
    let value = value
        .as_string_bytes()
        .ok_or_else(|| CodecError::new("the BSON binary data must be a string"))?;
    let subtype = subtype
        .as_int()
        .and_then(|subtype| u8::try_from(subtype).ok())
        .ok_or_else(|| CodecError::new("a BSON binary subtype must fit in one byte"))?;
    if subtype == 0x02 {
        let outer = value
            .len()
            .checked_add(4)
            .ok_or_else(|| CodecError::new("the BSON binary data is too large"))?;
        write_i32_length(outer, "BSON binary data", bytes)?;
        bytes.push(subtype);
        write_i32_length(value.len(), "BSON binary data", bytes)?;
    } else {
        write_i32_length(value.len(), "BSON binary data", bytes)?;
        bytes.push(subtype);
    }
    bytes.extend_from_slice(value);
    Ok(())
}

fn begin_container(bytes: &mut Vec<u8>) -> usize {
    let start = bytes.len();
    bytes.extend_from_slice(&[0; 4]);
    start
}

fn finish_container(start: usize, bytes: &mut Vec<u8>) -> Result<(), CodecError> {
    bytes.push(0);
    patch_length(start, bytes, "a BSON document")
}

fn patch_length(start: usize, bytes: &mut [u8], name: &str) -> Result<(), CodecError> {
    let length = bytes.len() - start;
    let length =
        i32::try_from(length).map_err(|_| CodecError::new(format!("{name} is too large")))?;
    bytes[start..start + 4].copy_from_slice(&length.to_le_bytes());
    Ok(())
}

fn write_i32_length(length: usize, name: &str, bytes: &mut Vec<u8>) -> Result<(), CodecError> {
    let length =
        i32::try_from(length).map_err(|_| CodecError::new(format!("{name} is too large")))?;
    bytes.extend_from_slice(&length.to_le_bytes());
    Ok(())
}

fn write_string(value: &[u8], name: &str, bytes: &mut Vec<u8>) -> Result<(), CodecError> {
    str::from_utf8(value)
        .map_err(|_| CodecError::new(format!("{name} must contain valid UTF-8")))?;
    let length = value
        .len()
        .checked_add(1)
        .ok_or_else(|| CodecError::new(format!("{name} is too large")))?;
    write_i32_length(length, name, bytes)?;
    bytes.extend_from_slice(value);
    bytes.push(0);
    Ok(())
}

fn write_cstring(value: &[u8], name: &str, bytes: &mut Vec<u8>) -> Result<(), CodecError> {
    str::from_utf8(value)
        .map_err(|_| CodecError::new(format!("{name} must contain valid UTF-8")))?;
    if value.contains(&0) {
        return Err(CodecError::new(format!(
            "{name} must not contain a null byte"
        )));
    }
    bytes.extend_from_slice(value);
    bytes.push(0);
    Ok(())
}

fn decode_raw_document(
    context: &mut Context<'_, '_, '_>,
    document: &RawDocument,
    depth: usize,
) -> Result<Value, CodecError> {
    require_depth(depth)?;
    let mut names = HashSet::new();
    let mut entries = Vec::new();
    for element in document.iter_elements() {
        let element = element.map_err(|error| CodecError::new(error.to_string()))?;
        let name = element.key().as_str();
        if !names.insert(name) {
            return Err(CodecError::new(
                "the BSON document contains a duplicate field",
            ));
        }
        let value = element
            .value()
            .map_err(|error| CodecError::new(error.to_string()))?;
        entries.push((
            context.string(name.as_bytes()),
            decode_raw_value(context, value, depth)?,
        ));
    }
    Ok(context.dict(entries))
}

fn decode_raw_array(
    context: &mut Context<'_, '_, '_>,
    array: &RawArray,
    depth: usize,
) -> Result<Value, CodecError> {
    require_depth(depth)?;
    let mut values = Vec::new();
    let mut expected = itoa::Buffer::new();
    for (index, element) in array.iter_elements().enumerate() {
        let element = element.map_err(|error| CodecError::new(error.to_string()))?;
        if element.key().as_str() != expected.format(index) {
            return Err(CodecError::new("the BSON array has a non-canonical index"));
        }
        let value = element
            .value()
            .map_err(|error| CodecError::new(error.to_string()))?;
        values.push(decode_raw_value(context, value, depth)?);
    }
    Ok(context.vec(values))
}

fn decode_raw_value(
    context: &mut Context<'_, '_, '_>,
    value: RawBsonRef<'_>,
    depth: usize,
) -> Result<Value, CodecError> {
    match value {
        RawBsonRef::Double(value) => Ok(Value::float(value)),
        RawBsonRef::String(value) => Ok(context.string(value.as_bytes())),
        RawBsonRef::Array(value) => decode_raw_array(context, value, depth + 1),
        RawBsonRef::Document(value) => decode_raw_document(context, value, depth + 1),
        RawBsonRef::Boolean(value) => Ok(Value::bool(value)),
        RawBsonRef::Null => Ok(Value::null()),
        RawBsonRef::RegularExpression(value) => {
            validate_regex_options(value.options.as_str())?;
            instance(
                context,
                class_text(REGULAR_EXPRESSION),
                [
                    context.string(value.pattern.as_str().as_bytes()),
                    context.string(value.options.as_str().as_bytes()),
                ],
            )
        }
        RawBsonRef::JavaScriptCode(value) => tagged_string(context, JAVASCRIPT, value.to_owned()),
        RawBsonRef::JavaScriptCodeWithScope(value) => {
            let code = context.string(value.code.as_bytes());
            let scope = decode_raw_document(context, value.scope, depth + 1)?;
            instance(context, class_text(JAVASCRIPT_WITH_SCOPE), [code, scope])
        }
        RawBsonRef::Int32(value) => tagged_int(context, INT32, i64::from(value)),
        RawBsonRef::Int64(value) => Ok(Value::int(value)),
        RawBsonRef::Timestamp(value) => instance(
            context,
            class_text(TIMESTAMP),
            [
                Value::int(i64::from(value.time)),
                Value::int(i64::from(value.increment)),
            ],
        ),
        RawBsonRef::Binary(value) => instance(
            context,
            class_text(BINARY),
            [
                context.string(value.bytes),
                Value::int(i64::from(u8::from(value.subtype))),
            ],
        ),
        RawBsonRef::ObjectId(value) => instance(
            context,
            class_text(OBJECT_ID),
            [context.string(&value.bytes())],
        ),
        RawBsonRef::DateTime(value) => system_time_value(context, value.timestamp_millis()),
        RawBsonRef::Symbol(value) => tagged_string(context, SYMBOL, value.to_owned()),
        RawBsonRef::Decimal128(value) => instance(
            context,
            class_text(DECIMAL128),
            [context.string(&value.bytes())],
        ),
        RawBsonRef::Undefined => marker(context, b"Undefined"),
        RawBsonRef::MaxKey => marker(context, b"MaxKey"),
        RawBsonRef::MinKey => marker(context, b"MinKey"),
        RawBsonRef::DbPointer(_) => {
            let owned = bson::RawBson::from(value);
            let pointer = Bson::try_from(owned)?;
            let Bson::DbPointer(pointer) = pointer else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a raw DB pointer converts to a DB pointer") }
            };
            let decoded: OwnedDbPointerEnvelope =
                bson::deserialize_from_bson(Bson::DbPointer(pointer))?;
            let identifier = instance(
                context,
                class_text(OBJECT_ID),
                [context.string(&decoded.pointer.identifier.bytes())],
            )?;
            instance(
                context,
                class_text(DB_POINTER),
                [
                    context.string(decoded.pointer.namespace.as_bytes()),
                    identifier,
                ],
            )
        }
    }
}

fn value_bytes(value: &Value) -> Result<&[u8], CodecError> {
    value
        .as_string_bytes()
        .ok_or_else(|| CodecError::new("the BSON value must be a string"))
}

const fn class_text(name: &'static [u8]) -> &'static str {
    // SAFETY: every caller passes an ASCII class-name constant from this file.
    unsafe { str::from_utf8_unchecked(name) }
}

fn throw(context: &mut Context<'_, '_, '_>, class: &str, error: &CodecError) -> Throw {
    let class = context.vm.intern(class.as_bytes());
    context.vm.throw(class, &error.to_string(), 0)
}

fn validate_regex_options(options: &str) -> Result<(), CodecError> {
    let mut previous = None;
    for option in options.bytes() {
        if !matches!(option, b'i' | b'l' | b'm' | b's' | b'u' | b'x')
            || previous.is_some_and(|previous| option <= previous)
        {
            return Err(CodecError::new(
                "the BSON regular expression options are invalid",
            ));
        }
        previous = Some(option);
    }
    Ok(())
}

fn instance<const N: usize>(
    context: &mut Context<'_, '_, '_>,
    class: &str,
    slots: [Value; N],
) -> Result<Value, CodecError> {
    let value = context
        .new_instance(class)
        .map_err(|_| CodecError::new(format!("could not allocate {class}")))?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let object = unsafe {
        unwrap_option_invariant(
            value.as_object(),
            "a freshly allocated class value is an object",
        )
    };
    for (index, slot) in slots.into_iter().enumerate() {
        drop(object.write_slot(index, slot));
    }
    Ok(value)
}

fn marker(context: &Context<'_, '_, '_>, name: &[u8]) -> Result<Value, CodecError> {
    let class_name = context.vm.intern(MARKER);
    let entry = context
        .vm
        .engine
        .tables
        .symbols
        .get(&class_name)
        .copied()
        .ok_or_else(|| CodecError::new("the BSON marker enum is not declared"))?;
    if entry.kind != SymbolKind::Enum {
        return Err(CodecError::new("the BSON marker symbol is not an enum"));
    }
    let name = context.vm.intern(name);
    context
        .vm
        .enum_case_value(ClassId(entry.index), name)
        .ok_or_else(|| CodecError::new("the BSON marker case is not declared"))
}

fn tagged_string(
    context: &mut Context<'_, '_, '_>,
    name: &[u8],
    value: String,
) -> Result<Value, CodecError> {
    let value = Value::from_string_vec(context.vm.heap(), value.into_bytes());
    tagged(context, name, value)
}

fn tagged_int(
    context: &mut Context<'_, '_, '_>,
    name: &[u8],
    value: i64,
) -> Result<Value, CodecError> {
    tagged(context, name, Value::int(value))
}

fn tagged(
    context: &mut Context<'_, '_, '_>,
    name: &[u8],
    value: Value,
) -> Result<Value, CodecError> {
    let name = context.vm.intern(name);
    let entry = context
        .vm
        .engine
        .tables
        .symbols
        .get(&name)
        .copied()
        .ok_or_else(|| CodecError::new("the BSON newtype is not declared"))?;
    if entry.kind != SymbolKind::Newtype {
        return Err(CodecError::new("the BSON symbol is not a newtype"));
    }
    let tag = context.vm.engine.tables.intern_newtype_value(
        NewtypeId(entry.index),
        TypeEnvironmentId::default(),
        None,
    );
    Ok(Value::newtype(value, tag))
}

fn is_newtype(context: &Context<'_, '_, '_>, value: &Value, name: &[u8]) -> bool {
    let Some(id) = value.newtype_id() else {
        return false;
    };
    let tagged = context.vm.engine.tables.newtype_value(id);
    context.vm.engine.tables.newtypes[tagged.declaration.0 as usize]
        .name
        .as_bytes()
        == name
}

fn class_name<'value>(
    context: &'value Context<'_, '_, '_>,
    object: &InstanceObject,
) -> &'value [u8] {
    context.vm.engine.tables.classes[object.class().0 as usize]
        .name
        .as_bytes()
}

fn require_depth(depth: usize) -> Result<(), CodecError> {
    if depth > MAXIMUM_DEPTH {
        return Err(CodecError::new(
            "values in BSON cannot nest deeper than 128 levels",
        ));
    }
    Ok(())
}

fn object_id(object: &InstanceObject) -> Result<BsonObjectId, CodecError> {
    let bytes = object.read_slot(0);
    let bytes: [u8; 12] = bytes
        .as_string_bytes()
        .ok_or_else(|| CodecError::new("a BSON object identifier must be a string"))?
        .try_into()
        .map_err(|_| CodecError::new("a BSON object identifier must contain 12 bytes"))?;
    Ok(BsonObjectId::from_bytes(bytes))
}

fn system_time(object: &InstanceObject) -> Result<bson::DateTime, CodecError> {
    let seconds = object.read_slot(0);
    let nanoseconds = object.read_slot(1);
    let seconds =
        // SAFETY: the surrounding invariant proves this option contains a value.
        unsafe { unwrap_option_invariant(seconds.as_int(), "system time seconds are an integer") };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let nanoseconds = unsafe {
        unwrap_option_invariant(
            nanoseconds.as_int(),
            "system time nanoseconds are an integer",
        )
    };
    let extra_seconds = nanoseconds.div_euclid(NANOSECONDS_PER_SECOND);
    let nanoseconds = nanoseconds.rem_euclid(NANOSECONDS_PER_SECOND);
    let seconds = i128::from(seconds) + i128::from(extra_seconds);
    let milliseconds = seconds * i128::from(MILLISECONDS_PER_SECOND)
        + i128::from(nanoseconds / NANOSECONDS_PER_MILLISECOND);
    let milliseconds = i64::try_from(milliseconds)
        .map_err(|_| CodecError::new("the system time is outside the BSON datetime range"))?;
    Ok(bson::DateTime::from_millis(milliseconds))
}

fn system_time_value(
    context: &mut Context<'_, '_, '_>,
    milliseconds: i64,
) -> Result<Value, CodecError> {
    let seconds = milliseconds.div_euclid(MILLISECONDS_PER_SECOND);
    let remainder = milliseconds.rem_euclid(MILLISECONDS_PER_SECOND);
    instance(
        context,
        class_text(SYSTEM_TIME),
        [
            Value::int(seconds),
            Value::int(remainder * NANOSECONDS_PER_MILLISECOND),
        ],
    )
}

fn uint32(value: &Value, name: &str) -> Result<u32, CodecError> {
    let value = value
        .as_int()
        .ok_or_else(|| CodecError::new(format!("{name} must be an integer")))?;
    u32::try_from(value).map_err(|_| CodecError::new(format!("{name} must fit in 32 bits")))
}
