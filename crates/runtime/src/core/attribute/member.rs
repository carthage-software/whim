//! Member attribute reflection.

use core::iter;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::value::Value;
use crate::value::ValueView;

fn invalid_parameter_selector(cx: &mut Context<'_, '_, '_>, message: &str) -> Throw {
    let error = cx.vm.intern(b"Whim\\Unwind\\InvalidArgumentException");
    cx.vm.throw(error, message, 0)
}

#[whim_function("Whim\\Attribute\\get_function_attributes(string $function): vec<object>")]
fn get_function_attributes(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let name = cx.vm.intern(arguments.bytes(0));
    let instances = cx.vm.function_attributes(name)?;
    Ok(cx.vec(instances))
}

#[whim_function(
    "Whim\\Attribute\\get_method_attributes(object $object, string $method): vec<object>"
)]
fn get_method_attributes(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let class = arguments.instance(0).class();
    let name = cx.vm.intern(arguments.bytes(1));
    let instances = cx.vm.method_attributes(class, name)?;
    Ok(cx.vec(instances))
}

#[whim_function(
    "Whim\\Attribute\\get_property_attributes(object $object, string $property): vec<object>"
)]
fn get_property_attributes(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let class = arguments.instance(0).class();
    let name = cx.vm.intern(arguments.bytes(1));
    let instances = cx.vm.property_attributes(class, name)?;
    Ok(cx.vec(instances))
}

#[whim_function(
    "Whim\\Attribute\\get_constant_attributes(object $object, string $constant): vec<object>"
)]
fn get_constant_attributes(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let class = arguments.instance(0).class();
    let name = cx.vm.intern(arguments.bytes(1));
    let instances = cx.vm.class_constant_attributes(class, name)?;
    Ok(cx.vec(instances))
}

#[whim_function("Whim\\Attribute\\get_enum_case_attributes(object $case): vec<object>")]
fn get_enum_case_attributes(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let case = arguments.local(0);
    let class = arguments.instance(0).class();
    let name = cx.get_property(&case, "name")?;
    let Some(name) = cx.value_atom(&name) else {
        return Ok(cx.vec(iter::empty()));
    };
    let instances = cx.vm.enum_case_attributes(class, name)?;
    Ok(cx.vec(instances))
}

#[whim_function(
    "Whim\\Attribute\\get_parameter_attributes(object $object, string $method, int|string $parameter): vec<object>"
)]
fn get_parameter_attributes(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let class = arguments.instance(0).class();
    let name = cx.vm.intern(arguments.bytes(1));
    let parameter = arguments.local(2);
    let instances = match parameter.transparent() {
        ValueView::Int(position) => {
            let Ok(position) = usize::try_from(*position) else {
                return Err(invalid_parameter_selector(
                    cx,
                    "$parameter must be a non-negative int or a non-empty string",
                ));
            };
            cx.vm.parameter_attributes(class, name, position)?
        }
        ValueView::String(_) | ValueView::ShortString(_) => {
            if parameter.as_string_bytes().is_none_or(<[u8]>::is_empty) {
                return Err(invalid_parameter_selector(
                    cx,
                    "$parameter must be a non-empty string",
                ));
            }
            let Some(parameter) = cx.value_atom(&parameter) else {
                return Ok(cx.vec(iter::empty()));
            };
            cx.vm.parameter_attributes_named(class, name, parameter)?
        }
        _ => return Err(cx.type_error("$parameter must be an int or string")),
    };
    Ok(cx.vec(instances))
}
