//! WHATWG HTML character references.

use htmlize::Context as HtmlContext;
use htmlize::ENTITIES;
use htmlize::ENTITY_MAX_LENGTH;
use htmlize::escape_attribute_bytes;
use htmlize::escape_text_bytes;
use htmlize::unescape_bytes_in;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

#[whim_function(
    "Whim\\_Private\\html_escape_text(string $text): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn escape_text(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let text = arguments.bytes(0);
    let escaped = escape_text_bytes(text);
    context.string(escaped.as_ref())
}

#[whim_function(
    "Whim\\_Private\\html_escape_attribute(string $text): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn escape_attribute(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let text = arguments.bytes(0);
    let escaped = escape_attribute_bytes(text);
    context.string(escaped.as_ref())
}

#[whim_function(
    "Whim\\_Private\\html_decode(string $html): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn decode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let html = arguments.bytes(0);
    let decoded = unescape_bytes_in(html, HtmlContext::General);
    context.string(decoded.as_ref())
}

#[whim_function(
    "Whim\\_Private\\html_decode_attribute(string $html): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn decode_attribute(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let html = arguments.bytes(0);
    let decoded = unescape_bytes_in(html, HtmlContext::Attribute);
    context.string(decoded.as_ref())
}

#[whim_function(
    "Whim\\_Private\\html_entity(string $name): null|(string&!'')",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn entity(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let name = arguments.bytes(0);
    if name.len() > ENTITY_MAX_LENGTH - 2 {
        return Value::null();
    }

    let key_length = name.len() + 2;
    let mut key = [0; ENTITY_MAX_LENGTH];
    key[0] = b'&';
    key[1..key_length - 1].copy_from_slice(name);
    key[key_length - 1] = b';';

    ENTITIES
        .get(&key[..key_length])
        .map_or_else(Value::null, |value| context.string(value))
}
