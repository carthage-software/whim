//! Applied attribute and attribute-contract reflection.

use std::rc::Rc;
use std::slice;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledAttribute;
use crate::core::reflection::Operation;
use crate::core::reflection::metadata;
use crate::core::reflection::model::DeclarationKey;
use crate::core::reflection::model::ReflectionData;
use crate::core::reflection::objects;
use crate::core::reflection::support;
use crate::core::symbols::strip_leading_backslash;
use crate::symbols::UnitContext;
use crate::value::Value;
use crate::value::object::ClassId;

const TARGET_CLASS: i64 = 1;
const TARGET_FUNCTION: i64 = 2;
const TARGET_METHOD: i64 = 4;
const TARGET_PROPERTY: i64 = 8;
const TARGET_CLASS_CONSTANT: i64 = 16;
const TARGET_PARAMETER: i64 = 32;
const REPEATABLE: i64 = 64;
const TARGET_TYPE_ALIAS: i64 = 128;
const TARGET_NEWTYPE: i64 = 256;
const TARGET_CONSTANT: i64 = 512;

pub(crate) fn declaration_attributes(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    target: &DeclarationKey,
) -> Result<Value, Throw> {
    let metadata = support::declaration_metadata(context.vm, target);
    let wanted_name = if operation == Operation::AttributesByName {
        let name = context.vm.intern(arguments.bytes(0));
        Some(strip_leading_backslash(context.vm.heap(), name))
    } else {
        None
    };
    let wanted_type = (operation == Operation::Attributes).then(|| {
        context
            .vm
            .built_in_type_descriptor(&TypeSpec::Parameter("T"), context.type_environment)
    });

    let mut reflections = Vec::with_capacity(metadata.attributes.len());
    for declaration in metadata.attributes {
        if wanted_name
            .as_ref()
            .is_some_and(|name| declaration.class != *name)
        {
            continue;
        }
        if let Some(expected) = wanted_type.as_ref() {
            let actual = TypeDescriptor::Named {
                name: declaration.class.clone(),
                arguments: None,
                recursive: false,
            };
            let matches = context
                .vm
                .descriptor_is_subtype(&actual, expected, context.type_environment, 0)
                .map_err(|control| context.vm.control_to_throw(control))?;
            if !matches {
                continue;
            }
        }
        reflections.push(objects::build(
            context,
            ReflectionData::Attribute {
                target: target.clone(),
                declaration,
                unit: metadata.unit.clone(),
            },
            Vec::new(),
        )?);
    }
    Ok(context.vec(reflections))
}

pub(crate) fn attribute_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    target: &DeclarationKey,
    declaration: &CompiledAttribute,
    unit: Option<&Rc<UnitContext>>,
) -> Result<Value, Throw> {
    match operation {
        Operation::Class => objects::symbol(context, declaration.class.clone()),
        Operation::Target => objects::declaration(context, target.clone()),
        Operation::Arguments => positional_arguments(context, declaration, unit),
        Operation::NamedArguments => named_arguments(context, declaration, unit),
        Operation::Location => {
            let Some(unit) = unit else {
                return Ok(Value::null());
            };
            metadata::reflect_location(context, unit, declaration.span)
        }
        Operation::NewInstance => {
            let instances = context
                .vm
                .build_attribute_instances(slice::from_ref(declaration), unit)
                .map_err(|control| context.vm.control_to_throw(control))?;
            instances
                .into_iter()
                .next()
                .ok_or_else(|| context.type_error("the reflected attribute could not be built"))
        }
        _ => Err(context.type_error("the operation is not valid for this reflected attribute")),
    }
}

pub(crate) fn definition_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    class: ClassId,
) -> Result<Value, Throw> {
    let declaration = &context.vm.engine.tables.classes[class.0 as usize];
    let Some(flags) = declaration.attribute_flags else {
        return Err(context.type_error("the reflected class is not an attribute class"));
    };
    match operation {
        Operation::Class => objects::symbol(context, declaration.name.clone()),
        Operation::Targets => targets(context, flags),
        Operation::IsRepeatable => Ok(Value::bool(flags & REPEATABLE != 0)),
        _ => Err(context
            .type_error("the operation is not valid for this reflected attribute definition")),
    }
}

fn positional_arguments(
    context: &mut Context<'_, '_, '_>,
    declaration: &CompiledAttribute,
    unit: Option<&Rc<UnitContext>>,
) -> Result<Value, Throw> {
    let mut values = Vec::with_capacity(declaration.arguments.len());
    for argument in &declaration.arguments {
        values.push(support::evaluate_initializer(context.vm, argument, unit)?);
    }
    Ok(context.vec(values))
}

fn named_arguments(
    context: &mut Context<'_, '_, '_>,
    declaration: &CompiledAttribute,
    unit: Option<&Rc<UnitContext>>,
) -> Result<Value, Throw> {
    let mut values = Vec::with_capacity(declaration.named_arguments.len());
    for (name, argument) in &declaration.named_arguments {
        values.push((
            Value::string(name.to_handle()),
            support::evaluate_initializer(context.vm, argument, unit)?,
        ));
    }
    Ok(context.dict(values))
}

fn targets(context: &mut Context<'_, '_, '_>, flags: i64) -> Result<Value, Throw> {
    let mut targets = Vec::new();
    for (flag, name) in [
        (TARGET_CLASS, b"Class".as_slice()),
        (TARGET_FUNCTION, b"Function".as_slice()),
        (TARGET_METHOD, b"Method".as_slice()),
        (TARGET_PROPERTY, b"Property".as_slice()),
        (TARGET_CLASS_CONSTANT, b"ClassConstant".as_slice()),
        (TARGET_PARAMETER, b"Parameter".as_slice()),
        (TARGET_TYPE_ALIAS, b"TypeAlias".as_slice()),
        (TARGET_NEWTYPE, b"Newtype".as_slice()),
        (TARGET_CONSTANT, b"Constant".as_slice()),
    ] {
        if flags & flag != 0 {
            targets.push(objects::enum_case(
                context,
                b"Whim\\Reflection\\Attribute\\Target",
                name,
            )?);
        }
    }
    Ok(context.vec(targets))
}
