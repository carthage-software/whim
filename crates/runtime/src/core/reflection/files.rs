use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::bytecode::unit::is_external;
use crate::core::reflection::Operation;
use crate::core::reflection::functions::symbol_kind_argument;
use crate::core::reflection::metadata;
use crate::core::reflection::objects;
use crate::symbols::UnitContext;
use crate::value::Value;

pub(crate) fn dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    unit: &UnitContext,
    position: usize,
) -> Result<Value, Throw> {
    let file = &unit.unit.files[position];
    match operation {
        Operation::Path => Ok(file
            .path
            .as_ref()
            .map_or_else(Value::null, |path| Value::string(path.to_handle()))),
        Operation::Origin => metadata::reflect_origin(context, Some(unit.origin)),
        Operation::HasTopLevelCode => Ok(Value::bool(file.has_top_level_code)),
        Operation::Symbols => symbols(context, arguments, unit, position),
        _ => Err(context.type_error("the operation is not valid for a reflected file")),
    }
}

fn symbols(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    unit: &UnitContext,
    position: usize,
) -> Result<Value, Throw> {
    let kind = arguments
        .get(0)
        .filter(|value| !value.is_null() && !value.is_uninitialized())
        .map(|value| symbol_kind_argument(context, value))
        .transpose()?;
    let compiled = &unit.unit;
    let file = &compiled.files[position];
    let declarations = compiled
        .functions
        .iter()
        .map(|value| (&value.name, value.span, value.attributes.as_slice()))
        .chain(
            compiled
                .classes
                .iter()
                .map(|value| (&value.name, value.span, value.attributes.as_slice())),
        )
        .chain(
            compiled
                .constants
                .iter()
                .map(|value| (&value.name, value.span, value.attributes.as_slice())),
        )
        .chain(
            compiled
                .type_aliases
                .iter()
                .map(|value| (&value.name, value.span, value.attributes.as_slice())),
        )
        .chain(
            compiled
                .newtypes
                .iter()
                .map(|value| (&value.name, value.span, value.attributes.as_slice())),
        );

    let mut names = declarations
        .filter_map(|(name, span, attributes)| {
            if span.start.offset < file.span.start.offset
                || span.start.offset >= file.span.end.offset
                || is_external(attributes)
            {
                return None;
            }
            let entry = context.vm.engine.tables.symbols.get(name)?;
            kind.is_none_or(|kind| entry.kind == kind)
                .then(|| name.clone())
        })
        .collect::<Vec<_>>();

    names.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let mut reflections = Vec::with_capacity(names.len());
    for name in names {
        reflections.push(objects::symbol(context, name)?);
    }

    Ok(context.vec(reflections))
}
