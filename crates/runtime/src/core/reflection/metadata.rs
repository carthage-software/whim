//! Source and origin metadata for reflected declarations.

use whim_span::Span;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::reflection::Operation;
use crate::core::reflection::attributes;
use crate::core::reflection::model::DeclarationKey;
use crate::core::reflection::model::ReflectionData;
use crate::core::reflection::model::SourceLocationData;
use crate::core::reflection::objects;
use crate::core::reflection::support;
use crate::symbols::UnitContext;
use crate::symbols::UnitOrigin;
use crate::symbols::line_of;
use crate::value::Value;

pub(crate) fn declaration_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    declaration: &DeclarationKey,
) -> Option<Result<Value, Throw>> {
    let result = match operation {
        Operation::Origin => origin(context, declaration),
        Operation::Location => location(context, declaration),
        Operation::Documentation => Ok(documentation(context, declaration)),
        Operation::Attributes | Operation::AttributesByName => {
            return Some(attributes::declaration_attributes(
                context,
                arguments,
                operation,
                declaration,
            ));
        }
        _ => return None,
    };

    Some(result)
}

pub(crate) fn source_location_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    location: &SourceLocationData,
) -> Result<Value, Throw> {
    let value = match operation {
        Operation::File => Value::string(location.file.to_handle()),
        Operation::StartOffset => Value::int(i64::from(location.start_offset)),
        Operation::EndOffset => Value::int(i64::from(location.end_offset)),
        Operation::StartLine => Value::int(i64::from(location.start_line)),
        Operation::StartColumn => Value::int(i64::from(location.start_column)),
        Operation::EndLine => Value::int(i64::from(location.end_line)),
        Operation::EndColumn => Value::int(i64::from(location.end_column)),
        _ => return Err(context.type_error("the operation is not valid for a source location")),
    };

    Ok(value)
}

pub(crate) fn reflect_location(
    context: &mut Context<'_, '_, '_>,
    unit: &UnitContext,
    span: Span,
) -> Result<Value, Throw> {
    let Some(location) = source_location(unit, span) else {
        return Ok(Value::null());
    };

    objects::build(
        context,
        ReflectionData::SourceLocation(location),
        Vec::new(),
    )
}

fn origin(context: &mut Context<'_, '_, '_>, declaration: &DeclarationKey) -> Result<Value, Throw> {
    let metadata = support::declaration_metadata(context.vm, declaration);
    let case = match metadata.unit.as_deref().map(|unit| unit.origin) {
        None => b"Core".as_slice(),
        Some(UnitOrigin::Extension) => b"Extension".as_slice(),
        Some(UnitOrigin::User) => b"User".as_slice(),
    };

    objects::enum_case(context, b"Whim\\Reflection\\DeclarationOrigin", case)
}

fn location(
    context: &mut Context<'_, '_, '_>,
    declaration: &DeclarationKey,
) -> Result<Value, Throw> {
    let metadata = support::declaration_metadata(context.vm, declaration);
    let (Some(unit), Some(span)) = (metadata.unit.as_deref(), metadata.span) else {
        return Ok(Value::null());
    };

    reflect_location(context, unit, span)
}

fn documentation(context: &Context<'_, '_, '_>, declaration: &DeclarationKey) -> Value {
    let metadata = support::declaration_metadata(context.vm, declaration);
    let (Some(unit), Some(span)) = (metadata.unit.as_deref(), metadata.span) else {
        return Value::null();
    };

    if unit.origin != UnitOrigin::User {
        return Value::null();
    }

    let Some(source) = unit.source.as_deref() else {
        return Value::null();
    };

    let start = usize::try_from(span.start.offset).unwrap_or(usize::MAX);
    let Some(prefix) = source.as_bytes().get(..start) else {
        return Value::null();
    };

    // TODO(azjezz): there might be a better way to do this...
    // we could retain docblock content in the unit during compilation
    // instead of extracting it here again. however, retaining it in the unit/metadata
    // would increase their size, and it's only used for reflection, so it might not be worth it.
    let Some(documentation) = preceding_docblock(prefix) else {
        return Value::null();
    };

    context.string(documentation)
}

fn source_location(unit: &UnitContext, span: Span) -> Option<SourceLocationData> {
    if unit.origin != UnitOrigin::User {
        return None;
    }

    let (path, line_starts, base, limit) = unit
        .source_files
        .iter()
        .find(|file| span.start.offset >= file.start && span.start.offset < file.end)
        .map_or(
            (&unit.path, unit.line_starts.as_slice(), 0, u32::MAX),
            |file| {
                (
                    &file.path,
                    file.line_starts.as_slice(),
                    file.start,
                    file.end,
                )
            },
        );

    let start = span.start.offset.saturating_sub(base);
    let end = span.end.offset.min(limit).saturating_sub(base);
    let (start_line, start_column) = line_and_column(line_starts, start);
    let (end_line, end_column) = line_and_column(line_starts, end);
    Some(SourceLocationData {
        file: path.clone(),
        start_offset: start,
        end_offset: end,
        start_line,
        start_column,
        end_line,
        end_column,
    })
}

fn line_and_column(line_starts: &[u32], offset: u32) -> (u32, u32) {
    let line = line_of(line_starts, offset).max(1);
    let start = line_starts
        .get(line.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(0);

    (line, offset.saturating_sub(start) + 1)
}

fn preceding_docblock(prefix: &[u8]) -> Option<&[u8]> {
    let trimmed = prefix.strip_suffix(trim_ascii_end(prefix))?;
    if !trimmed.ends_with(b"*/") {
        return None;
    }

    let start = trimmed.windows(3).rposition(|window| window == b"/**")?;
    Some(&trimmed[start..])
}

fn trim_ascii_end(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |position| position + 1);

    &value[start..]
}
