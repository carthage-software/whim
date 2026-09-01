//! Class attribute reflection.

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::value::Value;

#[whim_function("Whim\\Attribute\\has_attribute(object $object, string $attribute): bool")]
fn has_attribute(cx: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let class = arguments.instance(0).class();
    let wanted = cx.vm.intern(arguments.bytes(1));
    let has = cx.vm.class_has_attribute(class, wanted);
    Value::bool(has)
}

#[whim_function("Whim\\Attribute\\get_attribute<T: object>(object $object): vec<T>")]
fn get_attribute(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let class = arguments.instance(0).class();
    let Some(attribute) = cx
        .vm
        .built_in_named_type(&TypeSpec::Parameter("T"), cx.type_environment)
    else {
        return Err(cx.type_error("the attribute type must be a named class"));
    };
    let filtered = cx.vm.class_attribute(class, attribute)?;
    Ok(cx.vec(filtered))
}

#[whim_function("Whim\\Attribute\\get_attributes(object $object): vec<object>")]
fn get_attributes(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let class = arguments.instance(0).class();
    let instances = cx.vm.class_attributes(class)?;
    Ok(cx.vec(instances))
}
