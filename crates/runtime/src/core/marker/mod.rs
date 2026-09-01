//! The `Whim\Marker` attribute classes.

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::attribute::marker::TARGET_ALL;
use crate::core::attribute::marker::TARGET_CALLABLE;
use crate::core::attribute::marker::TARGET_CLASS;
use crate::core::attribute::marker::TARGET_PARAMETER;
use crate::core::attribute::marker::TARGET_SYMBOL;
use crate::value::Value;

#[whim_class("Whim\\Marker\\Stub", final, attribute = TARGET_SYMBOL)]
#[whim_property("public readonly string $reason")]
#[whim_property("public readonly null|string $issue")]
struct Stub;

#[whim_methods]
impl Stub {
    #[whim_method("__construct(string $reason = 'built-in', null|string $issue = null): void")]
    fn construct(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        let reason = arguments.local(0);
        let issue = arguments.local(1);
        cx.set_property(&this, "reason", reason)?;
        cx.set_property(&this, "issue", issue)?;
        Ok(Value::null())
    }
}

#[whim_class(
    "Whim\\Marker\\ConsistentConstructor",
    final,
    attribute = TARGET_CLASS
)]
struct ConsistentConstructor;

#[whim_class(
    "Whim\\Marker\\ConsistentGenerics",
    final,
    attribute = TARGET_CLASS
)]
struct ConsistentGenerics;

#[whim_class("Whim\\Marker\\Deprecated", final, attribute = TARGET_ALL)]
#[whim_property("public readonly string $version")]
#[whim_property("public readonly string $note")]
struct Deprecated;

#[whim_methods]
impl Deprecated {
    #[whim_method("__construct(string $version, null|string $note = null): void")]
    fn construct(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        let version = arguments.local(0);
        let note = if arguments.is_absent(1) {
            cx.string(b"")
        } else {
            arguments.local(1)
        };
        cx.set_property(&this, "version", version)?;
        cx.set_property(&this, "note", note)?;
        Ok(Value::null())
    }
}

#[whim_class(
    "Whim\\Marker\\SensitiveParameter",
    final,
    attribute = TARGET_PARAMETER
)]
struct SensitiveParameter;

#[whim_methods]
impl SensitiveParameter {
    #[whim_method("__construct(): void")]
    const fn construct() {}
}

#[whim_class(
    "Whim\\Marker\\TrackCaller",
    final,
    attribute = TARGET_CALLABLE
)]
struct TrackCaller;

#[whim_class(
    "Whim\\Marker\\TraceBoundary",
    final,
    attribute = TARGET_CALLABLE
)]
struct TraceBoundary;

#[whim_class(
    "Whim\\Marker\\NeverInline",
    final,
    attribute = TARGET_CALLABLE
)]
struct NeverInline;

#[whim_class(
    "Whim\\Marker\\AlwaysInline",
    final,
    attribute = TARGET_CALLABLE
)]
struct AlwaysInline;

#[whim_class("Whim\\Marker\\Frameless", final, attribute = TARGET_CALLABLE)]
struct Frameless;

#[whim_class("Whim\\Marker\\Cold", final, attribute = TARGET_CALLABLE)]
struct Cold;

#[whim_class("Whim\\Marker\\MustUse", final, attribute = TARGET_CALLABLE)]
#[whim_property("public readonly null|string $note")]
struct MustUse;

#[whim_methods]
impl MustUse {
    #[whim_method("__construct(null|string $note = null): void")]
    fn construct(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        let note = if arguments.is_absent(0) {
            Value::null()
        } else {
            arguments.local(0)
        };
        cx.set_property(&this, "note", note)?;
        Ok(Value::null())
    }
}

#[whim_class("Whim\\Marker\\SensitiveParameterValue", final)]
#[whim_property("private readonly mixed $value")]
struct SensitiveParameterValue;

#[whim_methods]
impl SensitiveParameterValue {
    #[whim_method("__construct(mixed $value): void")]
    fn construct(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        let value = arguments.local(0);
        cx.set_property(&this, "value", value)?;
        Ok(Value::null())
    }

    #[whim_method("getValue(): mixed")]
    fn get_value(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        cx.get_property(&this, "value")
    }
}
