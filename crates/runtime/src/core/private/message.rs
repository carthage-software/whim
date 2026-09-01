//! Internet message parsing.

use std::borrow::Cow;

use mail_parser::HeaderName;
use mail_parser::MessageParser;
use memchr::memchr;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

#[whim_function(
    "Whim\\_Private\\message_parse_headers(string $bytes): null|(vec<(string, string)>, string)",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn parse_headers(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let parser = MessageParser::new().header_raw(HeaderName::Other(Cow::Borrowed("x-whim-raw")));
    let Some(message) = parser.parse_headers(bytes) else {
        return Value::null();
    };
    let headers = message.headers();
    if headers.is_empty() {
        return Value::null();
    }

    let mut expected_start = 0;
    let mut pairs = Vec::with_capacity(headers.len());
    for header in headers {
        let field_start = header.offset_field() as usize;
        let value_start = header.offset_start() as usize;
        let value_end = header.offset_end() as usize;
        if field_start != expected_start
            || value_start <= field_start
            || value_end < value_start
            || value_end > bytes.len()
            || bytes.get(value_start - 1) != Some(&b':')
        {
            return Value::null();
        }

        let name = context.string(&bytes[field_start..value_start - 1]);
        let value = unfold(&bytes[value_start..value_end]);
        let value = context.string(&value);
        pairs.push(context.tuple([name, value]));
        expected_start = value_end;
    }

    let body_start = if expected_start == bytes.len() {
        expected_start
    } else if bytes[expected_start..].starts_with(b"\r\n") {
        expected_start + 2
    } else if bytes[expected_start..].starts_with(b"\n") {
        expected_start + 1
    } else {
        return Value::null();
    };
    let pairs = context.vec(pairs);
    let body = context.string(&bytes[body_start..]);

    context.tuple([pairs, body])
}

fn unfold(value: &[u8]) -> Cow<'_, [u8]> {
    let Some(first_line_end) = memchr(b'\n', value) else {
        return Cow::Borrowed(trim(value));
    };
    if first_line_end + 1 == value.len() {
        return Cow::Borrowed(trim(value));
    }

    let mut unfolded = Vec::with_capacity(value.len());
    let mut offset = 0;
    let mut first = true;
    while offset < value.len() {
        let line_end =
            memchr(b'\n', &value[offset..]).map_or(value.len(), |relative| offset + relative);
        if !first {
            unfolded.push(b' ');
        }
        unfolded.extend_from_slice(trim(&value[offset..line_end]));
        first = false;
        if line_end == value.len() {
            break;
        }
        offset = line_end + 1;
    }

    Cow::Owned(unfolded)
}

fn trim(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(|byte| is_whitespace(*byte)) {
        value = &value[1..];
    }
    while value.last().is_some_and(|byte| is_whitespace(*byte)) {
        value = &value[..value.len() - 1];
    }

    value
}

const fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 11 | b'\r' | b' ')
}
