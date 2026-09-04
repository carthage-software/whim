//! Public BSON value types.

use std::str;

use bson::oid::ObjectId as RawObjectId;
use uuid::Uuid as RawUuid;
use whim_macros::whim_class;
use whim_macros::whim_enum;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::heap::handle::ManagedRef;
use crate::value::object::InstanceObject;

const BINARY_CLASS: &str = "Whim\\BSON\\Binary";
const DECIMAL128_CLASS: &str = "Whim\\BSON\\Decimal128";
const OBJECT_ID_CLASS: &str = "Whim\\BSON\\ObjectId";
const REGULAR_EXPRESSION_CLASS: &str = "Whim\\BSON\\RegularExpression";
const UUID_CLASS: &str = "Whim\\UUID\\UUID";
const INVALID_ARGUMENT_EXCEPTION: &[u8] = b"Whim\\Unwind\\InvalidArgumentException";

#[whim_class("Whim\\BSON\\Binary", final, readonly)]
#[whim_implements("Whim\\Comparison\\Equal<Whim\\BSON\\Binary>")]
#[whim_property("public string $bytes")]
#[whim_property("public Whim\\Refine\\Uint8 $subtype")]
#[whim_class_like_constant("GENERIC", "int", visibility = "public", literal = 0)]
#[whim_class_like_constant("FUNCTION", "int", visibility = "public", literal = 1)]
#[whim_class_like_constant("OLD_BINARY", "int", visibility = "public", literal = 2)]
#[whim_class_like_constant("OLD_UUID", "int", visibility = "public", literal = 3)]
#[whim_class_like_constant("UUID", "int", visibility = "public", literal = 4)]
#[whim_class_like_constant("MD5", "int", visibility = "public", literal = 5)]
#[whim_class_like_constant("ENCRYPTED", "int", visibility = "public", literal = 6)]
#[whim_class_like_constant("COLUMN", "int", visibility = "public", literal = 7)]
#[whim_class_like_constant("SENSITIVE", "int", visibility = "public", literal = 8)]
#[whim_class_like_constant("VECTOR", "int", visibility = "public", literal = 9)]
pub(crate) struct Binary;

#[whim_methods]
impl Binary {
    #[whim_method(
        "__construct(string $bytes, Whim\\Refine\\Uint8 $subtype = 0): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let receiver = context.receiver();
        let bytes = arguments.local(0);
        let subtype = if arguments.is_absent(1) {
            Value::int(0)
        } else {
            arguments.local(1)
        };
        context.set_property(&receiver, "bytes", bytes)?;
        context.set_property(&receiver, "subtype", subtype)?;
        Ok(Value::null())
    }

    #[whim_method(
        "fromUUID(Whim\\UUID\\UUID $uuid): Whim\\BSON\\Binary",
        static,
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn from_uuid(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let uuid = arguments.instance(0);
        if class_name(context, &uuid) != UUID_CLASS.as_bytes() {
            return Err(context.type_error("the value must be a UUID"));
        }
        let bytes = uuid.read_slot(0);
        new_instance(context, BINARY_CLASS, [bytes, Value::int(4)])
    }

    #[whim_method(
        "toUUID(): null|Whim\\UUID\\UUID",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn to_uuid(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = receiver(context);
        let bytes = receiver.read_slot(0);
        let subtype = receiver.read_slot(1);
        let Some(bytes) = bytes.as_string_bytes() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the BSON binary bytes are a string") }
        };
        if subtype.as_int() != Some(4) || bytes.len() != 16 {
            return Ok(Value::null());
        }
        let uuid = RawUuid::from_slice(bytes)
            .map_err(|_| context.type_error("the BSON binary value contains invalid UUID bytes"))?;
        let version = uuid.get_version().and_then(|_| {
            let version = i64::try_from(uuid.get_version_num()).ok()?;
            (1..=8).contains(&version).then_some(Value::int(version))
        });
        new_instance(
            context,
            UUID_CLASS,
            [context.string(bytes), version.unwrap_or_else(Value::null)],
        )
    }

    #[whim_method(
        "equals(Whim\\BSON\\Binary $other): bool",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn equals(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
        let receiver = receiver(context);
        let other = arguments.instance(0);
        Value::bool(
            string_slots_equal(&receiver, &other, 0) && int_slots_equal(&receiver, &other, 1),
        )
    }
}

#[whim_class("Whim\\BSON\\DBPointer", final, readonly)]
#[whim_property("public string $namespace")]
#[whim_property("public Whim\\BSON\\ObjectId $identifier")]
pub(crate) struct DBPointer;

#[whim_methods]
impl DBPointer {
    #[whim_method(
        "__construct(string $namespace, Whim\\BSON\\ObjectId $identifier): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        set_two_properties(context, arguments, "namespace", "identifier")
    }
}

#[whim_class("Whim\\BSON\\Decimal128", final, readonly)]
#[whim_implements("Whim\\Comparison\\Equal<Whim\\BSON\\Decimal128>")]
#[whim_property("private string[16] $bytes")]
pub(crate) struct Decimal128;

#[whim_methods]
impl Decimal128 {
    #[whim_method(
        "__construct(string[16] $bytes): void",
        visibility = "private",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        set_property(context, arguments.local(0), "bytes")
    }

    #[whim_method(
        "fromBytes(string[16] $bytes): Whim\\BSON\\Decimal128",
        static,
        must_use
    )]
    fn from_bytes(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.local(0);
        new_instance(context, DECIMAL128_CLASS, [bytes])
    }

    #[whim_method("toBytes(): string[16]", no_track_caller, no_trace_boundary, must_use)]
    fn to_bytes(context: &Context<'_, '_, '_>) -> Value {
        receiver(context).read_slot(0)
    }

    #[whim_method(
        "equals(Whim\\BSON\\Decimal128 $other): bool",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn equals(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
        Value::bool(string_slots_equal(
            &receiver(context),
            &arguments.instance(0),
            0,
        ))
    }
}

#[whim_class("Whim\\BSON\\JavaScriptWithScope", final, readonly)]
#[whim_property("public string $code")]
#[whim_property("public Whim\\BSON\\Document $scope")]
pub(crate) struct JavaScriptWithScope;

#[whim_methods]
impl JavaScriptWithScope {
    #[whim_method(
        "__construct(string $code, Whim\\BSON\\Document $scope): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        set_two_properties(context, arguments, "code", "scope")
    }
}

#[whim_class("Whim\\BSON\\ObjectId", final, readonly)]
#[whim_implements("Whim\\Comparison\\Equal<Whim\\BSON\\ObjectId>")]
#[whim_implements("Whim\\Convert\\ToString")]
#[whim_property("private string[12] $bytes")]
pub(crate) struct ObjectId;

#[whim_methods]
impl ObjectId {
    #[whim_method(
        "__construct(string[12] $bytes): void",
        visibility = "private",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        set_property(context, arguments.local(0), "bytes")
    }

    #[whim_method(
        "generate(): Whim\\BSON\\ObjectId",
        static,
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn generate(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let bytes = RawObjectId::new().bytes();
        let bytes = context.string(&bytes);
        new_instance(context, OBJECT_ID_CLASS, [bytes])
    }

    #[whim_method(
        "parse(string $value): null|Whim\\BSON\\ObjectId",
        static,
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn parse(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let Some(identifier) = parse_object_id(arguments.bytes(0)) else {
            return Ok(Value::null());
        };
        let bytes = context.string(&identifier.bytes());
        new_instance(context, OBJECT_ID_CLASS, [bytes])
    }

    #[whim_method("from(string $value): Whim\\BSON\\ObjectId", static, must_use)]
    fn from(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let value = arguments.bytes(0);
        let Some(identifier) = parse_object_id(value) else {
            return Err(invalid_argument(
                context,
                &format!(
                    "invalid canonical BSON object identifier: \"{}\"",
                    String::from_utf8_lossy(value)
                ),
            ));
        };
        let bytes = context.string(&identifier.bytes());
        new_instance(context, OBJECT_ID_CLASS, [bytes])
    }

    #[whim_method(
        "isValid(string $value): bool",
        static,
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn is_valid(arguments: Arguments<'_>) -> Value {
        Value::bool(parse_object_id(arguments.bytes(0)).is_some())
    }

    #[whim_method("fromBytes(string[12] $bytes): Whim\\BSON\\ObjectId", static, must_use)]
    fn from_bytes(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.local(0);
        new_instance(context, OBJECT_ID_CLASS, [bytes])
    }

    #[whim_method("toBytes(): string[12]", no_track_caller, no_trace_boundary, must_use)]
    fn to_bytes(context: &Context<'_, '_, '_>) -> Value {
        receiver(context).read_slot(0)
    }

    #[whim_method("toString(): string[24]", no_track_caller, no_trace_boundary, must_use)]
    fn to_string(context: &Context<'_, '_, '_>) -> Value {
        let receiver = receiver(context);
        let bytes = receiver.read_slot(0);
        // SAFETY: the surrounding invariant proves this option contains a value.
        let bytes: [u8; 12] = unsafe {
            unwrap_option_invariant(
                bytes
                    .as_string_bytes()
                    .and_then(|bytes| bytes.try_into().ok()),
                "an object identifier contains 12 bytes",
            )
        };
        let text = RawObjectId::from_bytes(bytes).to_hex();
        Value::from_string_vec(context.vm.heap(), text.into_bytes())
    }

    #[whim_method(
        "equals(Whim\\BSON\\ObjectId $other): bool",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn equals(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
        Value::bool(string_slots_equal(
            &receiver(context),
            &arguments.instance(0),
            0,
        ))
    }
}

#[whim_class("Whim\\BSON\\RegularExpression", final, readonly)]
#[whim_implements("Whim\\Comparison\\Equal<Whim\\BSON\\RegularExpression>")]
#[whim_property("public string $pattern")]
#[whim_property("public string $options")]
pub(crate) struct RegularExpression;

#[whim_methods]
impl RegularExpression {
    #[whim_method(
        "__construct(string $pattern, string $options): void",
        visibility = "private",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        set_two_properties(context, arguments, "pattern", "options")
    }

    #[whim_method(
        "fromParts(string $pattern, string $options = ''): Whim\\BSON\\RegularExpression",
        static,
        must_use
    )]
    fn from_parts(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let pattern = arguments.bytes(0);
        require_cstring(context, pattern, "regular expression pattern")?;
        let options = if arguments.is_absent(1) {
            &[][..]
        } else {
            arguments.bytes(1)
        };
        let options = normalize_options(context, options)?;
        let pattern = arguments.local(0);
        let options = Value::from_string_vec(context.vm.heap(), options);
        new_instance(context, REGULAR_EXPRESSION_CLASS, [pattern, options])
    }

    #[whim_method(
        "equals(Whim\\BSON\\RegularExpression $other): bool",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn equals(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
        let receiver = receiver(context);
        let other = arguments.instance(0);
        Value::bool(
            string_slots_equal(&receiver, &other, 0) && string_slots_equal(&receiver, &other, 1),
        )
    }
}

#[whim_class("Whim\\BSON\\Timestamp", final, readonly)]
#[whim_implements("Whim\\Comparison\\Equal<Whim\\BSON\\Timestamp>")]
#[whim_property("public Whim\\Refine\\Uint32 $seconds")]
#[whim_property("public Whim\\Refine\\Uint32 $increment")]
pub(crate) struct Timestamp;

#[whim_methods]
impl Timestamp {
    #[whim_method(
        "__construct(Whim\\Refine\\Uint32 $seconds, Whim\\Refine\\Uint32 $increment): void",
        no_track_caller,
        no_trace_boundary
    )]
    fn construct(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        set_two_properties(context, arguments, "seconds", "increment")
    }

    #[whim_method(
        "equals(Whim\\BSON\\Timestamp $other): bool",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn equals(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
        let receiver = receiver(context);
        let other = arguments.instance(0);
        Value::bool(int_slots_equal(&receiver, &other, 0) && int_slots_equal(&receiver, &other, 1))
    }
}

#[whim_enum("Whim\\BSON\\Marker")]
pub(crate) enum Marker {
    #[whim_case("Undefined")]
    Undefined,
    #[whim_case("MinKey")]
    MinKey,
    #[whim_case("MaxKey")]
    MaxKey,
}

fn receiver(context: &Context<'_, '_, '_>) -> ManagedRef<InstanceObject> {
    // SAFETY: the surrounding invariant proves this option contains a value.
    unsafe {
        unwrap_option_invariant(
            context.receiver().as_object().cloned(),
            "a built-in instance method has an object receiver",
        )
    }
}

fn class_name<'value>(
    context: &'value Context<'_, '_, '_>,
    object: &InstanceObject,
) -> &'value [u8] {
    context.vm.engine.tables.classes[object.class().0 as usize]
        .name
        .as_bytes()
}

fn string_slots_equal(left: &InstanceObject, right: &InstanceObject, slot: usize) -> bool {
    let left = left.read_slot(slot);
    let right = right.read_slot(slot);
    left.as_string_bytes() == right.as_string_bytes()
}

fn int_slots_equal(left: &InstanceObject, right: &InstanceObject, slot: usize) -> bool {
    left.read_slot(slot).as_int() == right.read_slot(slot).as_int()
}

fn set_property(
    context: &mut Context<'_, '_, '_>,
    value: Value,
    name: &str,
) -> Result<Value, Throw> {
    let receiver = context.receiver();
    context.set_property(&receiver, name, value)?;
    Ok(Value::null())
}

fn set_two_properties(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    first: &str,
    second: &str,
) -> Result<Value, Throw> {
    let receiver = context.receiver();
    context.set_property(&receiver, first, arguments.local(0))?;
    context.set_property(&receiver, second, arguments.local(1))?;
    Ok(Value::null())
}

fn new_instance<const N: usize>(
    context: &mut Context<'_, '_, '_>,
    class: &str,
    slots: [Value; N],
) -> Result<Value, Throw> {
    let value = context.new_instance(class)?;
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

fn parse_object_id(value: &[u8]) -> Option<RawObjectId> {
    if value.len() != 24
        || value
            .iter()
            .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    RawObjectId::parse_str(str::from_utf8(value).ok()?).ok()
}

fn require_cstring(
    context: &mut Context<'_, '_, '_>,
    value: &[u8],
    name: &str,
) -> Result<(), Throw> {
    if str::from_utf8(value).is_err() {
        return Err(invalid_argument(
            context,
            &format!("{name} must be valid UTF-8"),
        ));
    }
    if value.contains(&0) {
        return Err(invalid_argument(
            context,
            &format!("{name} must not contain a null byte"),
        ));
    }
    Ok(())
}

fn normalize_options(context: &mut Context<'_, '_, '_>, options: &[u8]) -> Result<Vec<u8>, Throw> {
    let mut found = [false; 6];
    for option in options {
        let index = match option {
            b'i' => 0,
            b'l' => 1,
            b'm' => 2,
            b's' => 3,
            b'u' => 4,
            b'x' => 5,
            _ => {
                return Err(invalid_argument(
                    context,
                    "regular expression options in BSON must be unique letters from \"ilmsux\"",
                ));
            }
        };
        if found[index] {
            return Err(invalid_argument(
                context,
                "regular expression options in BSON must be unique letters from \"ilmsux\"",
            ));
        }
        found[index] = true;
    }
    Ok(b"ilmsux"
        .iter()
        .zip(found)
        .filter_map(|(option, present)| present.then_some(*option))
        .collect())
}

fn invalid_argument(context: &mut Context<'_, '_, '_>, message: &str) -> Throw {
    let class = context.vm.intern(INVALID_ARGUMENT_EXCEPTION);
    context.vm.throw(class, message, 0)
}
