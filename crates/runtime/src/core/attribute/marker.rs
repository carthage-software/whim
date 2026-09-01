//! The `Whim\Attribute\Attribute` marker.

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::value::Value;

pub(crate) const TARGET_CLASS: i64 = 1;
pub(crate) const TARGET_CALLABLE: i64 = 6;
pub(crate) const TARGET_PARAMETER: i64 = 32;
pub(crate) const TARGET_SYMBOL: i64 = 899;
pub(crate) const TARGET_ALL: i64 = 959;

#[whim_class("Whim\\Attribute\\Attribute", final, attribute = 1)]
#[whim_property("public readonly int $flags")]
#[whim_class_like_constant("TARGET_CLASS", "int", visibility = "public", literal = 1)]
#[whim_class_like_constant("TARGET_FUNCTION", "int", visibility = "public", literal = 2)]
#[whim_class_like_constant("TARGET_METHOD", "int", visibility = "public", literal = 4)]
#[whim_class_like_constant("TARGET_PROPERTY", "int", visibility = "public", literal = 8)]
#[whim_class_like_constant("TARGET_CLASS_CONSTANT", "int", visibility = "public", literal = 16)]
#[whim_class_like_constant("TARGET_PARAMETER", "int", visibility = "public", literal = 32)]
#[whim_class_like_constant("IS_REPEATABLE", "int", visibility = "public", literal = 64)]
#[whim_class_like_constant("TARGET_TYPE_ALIAS", "int", visibility = "public", literal = 128)]
#[whim_class_like_constant("TARGET_NEWTYPE", "int", visibility = "public", literal = 256)]
#[whim_class_like_constant("TARGET_CONSTANT", "int", visibility = "public", literal = 512)]
#[whim_class_like_constant("TARGET_SYMBOL", "int", visibility = "public", literal = 899)]
#[whim_class_like_constant("TARGET_ALL", "int", visibility = "public", literal = 959)]
struct Attribute;

#[whim_methods]
impl Attribute {
    #[whim_method("__construct(null|int $flags = null): void")]
    fn construct(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        let flags = arguments.optional_int(0).unwrap_or(TARGET_ALL);
        let flags = Value::int(flags);
        cx.set_property(&this, "flags", flags)?;
        Ok(Value::null())
    }
}
