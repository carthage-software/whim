//! Regular expression primitives.

use std::cell::OnceCell;
use std::cell::RefCell;
use std::str::from_utf8;

use regex::bytes::CaptureLocations;
use regex::bytes::NoExpand;
use regex::bytes::Regex as BytesRegex;
use whim_macros::whim_class;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_result_invariant;
use crate::value::Value;

const REGEX: &str = "Whim\\_Private\\Regex";
const HEX: &[u8; 16] = b"0123456789abcdef";

struct RegexState {
    expression: BytesRegex,
    locations: RefCell<CaptureLocations>,
}

#[whim_class("Whim\\_Private\\Regex", final)]
#[derive(Default)]
pub(crate) struct Regex {
    state: OnceCell<RegexState>,
}

default_built_in_state!(Regex);

#[whim_methods]
impl Regex {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("compile(string $expression): null|Whim\\_Private\\Regex", static)]
    fn compile<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let expression = arguments.bytes(0);
        let Ok(expression) = from_utf8(expression) else {
            return Ok(Value::null());
        };

        let Ok(expression) = BytesRegex::new(expression) else {
            return Ok(Value::null());
        };

        let locations = expression.capture_locations();
        let object = context.new_built_in_instance(REGEX)?;
        let Some(regex) = state_ref::<Self>(&object) else {
            return Err(context.type_error("the regular expression has no built-in state"));
        };

        if regex
            .state
            .set(RegexState {
                expression,
                locations: RefCell::new(locations),
            })
            .is_err()
        {
            return Err(context.type_error("the regular expression is already initialized"));
        }

        Ok(object)
    }

    #[whim_method(
        "find(string $subject, 0.. $offset = 0): null|((0..), (0..), dict<int|string, null|string>)"
    )]
    fn find<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let subject = arguments.bytes(0);
        let offset = if arguments.is_absent(1) {
            0
        } else {
            arguments.int(1)
        };

        let Ok(offset) = usize::try_from(offset) else {
            return Ok(Value::null());
        };

        if offset > subject.len() {
            return Ok(Value::null());
        }

        let receiver = context.receiver();
        let Some(regex) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the regular expression has no built-in state"));
        };

        let Some(state) = regex.state.get() else {
            return Err(context.type_error("the regular expression is not initialized"));
        };

        let mut locations = state.locations.borrow_mut();
        let Some(found) = state
            .expression
            .captures_read_at(&mut locations, subject, offset)
        else {
            return Ok(Value::null());
        };

        let mut captures = Vec::with_capacity(state.expression.captures_len() * 2);
        for (index, name) in state.expression.capture_names().enumerate() {
            let value = match locations.get(index) {
                Some((start, end)) => context.string(&subject[start..end]),
                None => Value::null(),
            };
            // SAFETY: the surrounding invariant proves this result is successful.
            let index = unsafe {
                unwrap_result_invariant(
                    i64::try_from(index),
                    "capture indexes fit in Whim integers",
                )
            };
            captures.push((Value::int(index), value.clone()));
            if let Some(name) = name {
                captures.push((context.string(name.as_bytes()), value));
            }
        }

        let captures = context.dict(captures);

        // SAFETY: the surrounding invariant proves this result is successful.
        let start = unsafe {
            unwrap_result_invariant(
                i64::try_from(found.start()),
                "string offsets fit in Whim integers",
            )
        };
        // SAFETY: the surrounding invariant proves this result is successful.
        let end = unsafe {
            unwrap_result_invariant(
                i64::try_from(found.end()),
                "string offsets fit in Whim integers",
            )
        };
        Ok(context.tuple([Value::int(start), Value::int(end), captures]))
    }

    #[whim_method(
        "replaceLiteral(string $subject, string $replacement, null|(0..) $limit = null): string"
    )]
    fn replace_literal<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let subject = arguments.bytes(0);
        let replacement = arguments.bytes(1);
        let (limit, replaces_nothing) = if arguments.is_absent(2) {
            (0, false)
        } else {
            usize::try_from(arguments.int(2))
                .map_or((usize::MAX, false), |limit| (limit, limit == 0))
        };

        if replaces_nothing {
            return Ok(context.string(subject));
        }

        let receiver = context.receiver();
        let Some(regex) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the regular expression has no built-in state"));
        };
        let Some(state) = regex.state.get() else {
            return Err(context.type_error("the regular expression is not initialized"));
        };

        let replaced = state
            .expression
            .replacen(subject, limit, NoExpand(replacement));
        Ok(context.string_cow(replaced))
    }

    #[whim_method("split(string $subject): vec<string>")]
    fn split<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let subject = arguments.bytes(0);
        let receiver = context.receiver();
        let Some(regex) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the regular expression has no built-in state"));
        };
        let Some(state) = regex.state.get() else {
            return Err(context.type_error("the regular expression is not initialized"));
        };

        Ok(context.vec(
            state
                .expression
                .split(subject)
                .map(|part| context.string(part)),
        ))
    }
}

#[whim_function("Whim\\_Private\\regex_escape(string $literal): string", must_use)]
pub(crate) fn escape(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let literal = arguments.bytes(0);
    if let Ok(literal) = from_utf8(literal) {
        let escaped = regex::escape(literal);
        return context.owned_string(escaped.into_bytes());
    }

    let mut escaped = Vec::with_capacity(literal.len() * 4);
    for byte in literal {
        escaped.extend_from_slice(b"\\x");
        escaped.push(HEX[usize::from(byte >> 4)]);
        escaped.push(HEX[usize::from(byte & 0x0f)]);
    }

    context.owned_string(escaped)
}
