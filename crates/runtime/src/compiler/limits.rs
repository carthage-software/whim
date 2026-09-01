//! The static limits on how much one piece of syntax may contain.

use whim_span::HasSpan;
use whim_span::Span;

use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;

/// The most of any one repeated thing a single piece of syntax may name.
pub(in crate::compiler) const COUNT_LIMIT: usize = 64;
pub(in crate::compiler) const TUPLE_LIMIT: usize = 12;

pub(in crate::compiler) const WINDOW_CAPACITY: usize = u8::MAX as usize;

/// One extra slot lets a method window hold the receiver and all arguments.
const _: () = assert!(COUNT_LIMIT < WINDOW_CAPACITY);

pub(in crate::compiler) fn check_sequence<'item, T>(
    kind: CompileErrorKind,
    subject: &str,
    item: &str,
    items: impl IntoIterator<Item = &'item T>,
) -> Result<(), CompileError>
where
    T: HasSpan + 'item,
{
    check_sequence_limit(kind, subject, item, items, COUNT_LIMIT)
}

pub(in crate::compiler) fn check_tuple_sequence<'item, T>(
    kind: CompileErrorKind,
    subject: &str,
    item: &str,
    items: impl IntoIterator<Item = &'item T>,
) -> Result<(), CompileError>
where
    T: HasSpan + 'item,
{
    check_sequence_limit(kind, subject, item, items, TUPLE_LIMIT)
}

fn check_sequence_limit<'item, T>(
    kind: CompileErrorKind,
    subject: &str,
    item: &str,
    items: impl IntoIterator<Item = &'item T>,
    limit: usize,
) -> Result<(), CompileError>
where
    T: HasSpan + 'item,
{
    let Some(offender) = items.into_iter().nth(limit) else {
        return Ok(());
    };

    Err(CompileError::new(
        kind,
        format!("{subject} at most {limit} {item}"),
        offender.span(),
    ))
}

pub(in crate::compiler) fn check_count(
    kind: CompileErrorKind,
    subject: &str,
    item: &str,
    count: usize,
    span: Span,
) -> Result<u8, CompileError> {
    check_count_limit(kind, subject, item, count, span, COUNT_LIMIT)
}

pub(in crate::compiler) fn check_tuple_count(
    kind: CompileErrorKind,
    subject: &str,
    item: &str,
    count: usize,
    span: Span,
) -> Result<u8, CompileError> {
    check_count_limit(kind, subject, item, count, span, TUPLE_LIMIT)
}

fn check_count_limit(
    kind: CompileErrorKind,
    subject: &str,
    item: &str,
    count: usize,
    span: Span,
    limit: usize,
) -> Result<u8, CompileError> {
    match u8::try_from(count) {
        Ok(within) if usize::from(within) <= limit => Ok(within),
        _ => Err(CompileError::new(
            kind,
            format!("{subject} at most {limit} {item}"),
            span,
        )),
    }
}
