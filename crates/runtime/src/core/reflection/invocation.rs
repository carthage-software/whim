use whim_bytecode::chunk::descriptors::CalleeDescriptor;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::reflection::Operation;
use crate::core::reflection::declarations;
use crate::core::reflection::model::ReflectionData;
use crate::core::reflection::objects;

pub(super) fn dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    data: &ReflectionData,
    values: &[Value],
) -> Result<Value, Throw> {
    let offset = usize::from(matches!(data, ReflectionData::Member(_)));
    let types = type_arguments(context, vector(&arguments, offset))?;
    let args = vector(&arguments, offset + 1);
    if operation == Operation::Instantiate {
        let ReflectionData::Symbol(name) = data else {
            return Err(context.type_error("the reflection does not describe a class"));
        };

        let class = context
            .vm
            .resolve_class_symbol(name)
            .ok_or_else(|| context.type_error("the reflected class is no longer loaded"))?;
        return context
            .vm
            .instantiate_reflected(class, &types, args)
            .map_err(|control| context.vm.control_to_throw(control));
    }

    let (callee, descriptor) = match data {
        ReflectionData::Symbol(name) => {
            (context.string(name.as_bytes()), CalleeDescriptor::Function)
        }
        ReflectionData::CallableValue => (
            values
                .first()
                .cloned()
                .ok_or_else(|| context.type_error("the reflected callable is missing"))?,
            CalleeDescriptor::Value,
        ),
        ReflectionData::Member(member) => {
            let target = arguments.local(0);
            let entry = declarations::method_entry(context.vm, member)
                .ok_or_else(|| context.type_error("the reflected method is no longer loaded"))?;
            let descriptor = context
                .vm
                .reflected_method_target(entry, &member.name, &target)
                .map_err(|control| context.vm.control_to_throw(control))?;

            (target, descriptor)
        }
        _ => return Err(context.type_error("the reflection does not describe a callable")),
    };

    context
        .vm
        .invoke_reflected(&callee, &descriptor, &types, args)
        .map_err(|control| context.vm.control_to_throw(control))
}

fn vector<'a>(arguments: &Arguments<'a>, position: usize) -> &'a [Value] {
    arguments
        .get(position)
        .and_then(Value::as_vec)
        .map_or(&[], |values| values.as_slice())
}

fn type_arguments(
    context: &mut Context<'_, '_, '_>,
    values: &[Value],
) -> Result<Vec<TypeDescriptor>, Throw> {
    values
        .iter()
        .map(|value| {
            let Some(ReflectionData::Type(reflected)) = objects::data(value) else {
                return Err(context.type_error("the argument is not a reflected type"));
            };

            if !reflected.descriptor.is_resolved() {
                return Err(context.type_error("the reflected type argument is not resolved"));
            }

            Ok(reflected.descriptor)
        })
        .collect()
}
