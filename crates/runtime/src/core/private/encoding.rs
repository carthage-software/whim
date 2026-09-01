//! The encoding boundary: base64, base32, hex, percent, and punycode transforms.

use std::str::from_utf8;

use base16ct::lower;
use base16ct::mixed;
use base32ct::Encoding as _;
use base64ct::Encoding as _;
use percent_encoding::AsciiSet;
use percent_encoding::NON_ALPHANUMERIC;
use percent_encoding::percent_encode;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

const RESERVED: AsciiSet = NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Clone, Copy, Eq, PartialEq)]
enum PercentDecodeMode {
    Component,
    Form,
    Uri,
}

#[whim_function(
    "Whim\\_Private\\utf8_lossy(string $bytes): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn utf8_lossy(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let text = String::from_utf8_lossy(bytes);
    context.string(text.as_bytes())
}

#[whim_function(
    "Whim\\_Private\\mime_encoded_word_encode(string $value): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn mime_encoded_word_encode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let value = String::from_utf8_lossy(value);
    let encoded = mailrs_rfc2047::encode(&value);
    context.string(encoded.as_bytes())
}

#[whim_function(
    "Whim\\_Private\\mime_encoded_word_decode(string $value): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn mime_encoded_word_decode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let decoded = mailrs_rfc2047::decode(value);
    context.string(decoded.as_bytes())
}

#[whim_function(
    "Whim\\_Private\\mime_quoted_printable_encode(string $bytes, bool $binary): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn mime_quoted_printable_encode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let binary = arguments.bool(1);
    let encoded = if binary {
        quoted_printable::encode_binary(bytes)
    } else {
        quoted_printable::encode(bytes)
    };
    context.string(&encoded)
}

#[whim_function(
    "Whim\\_Private\\mime_quoted_printable_decode(string $encoded, bool $strict): null|string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
fn mime_quoted_printable_decode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let encoded = arguments.bytes(0);
    let strict = arguments.bool(1);
    let mode = if strict {
        quoted_printable::ParseMode::Strict
    } else {
        quoted_printable::ParseMode::Robust
    };
    quoted_printable::decode(encoded, mode)
        .map_or_else(|_| Value::null(), |decoded| context.string(&decoded))
}

fn pad_base64(mut encoded: String) -> String {
    while !encoded.len().is_multiple_of(4) {
        encoded.push('=');
    }

    encoded
}

fn strip_base64_padding(encoded: &str) -> Option<&str> {
    if encoded.is_empty() {
        return Some(encoded);
    }
    if !encoded.len().is_multiple_of(4) {
        return None;
    }

    let stripped = encoded.trim_end_matches('=');
    let removed = encoded.len() - stripped.len();
    if removed > 2 || stripped.contains('=') {
        return None;
    }

    Some(stripped)
}

const fn hex_digit_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn strict_percent_decode(bytes: &[u8], mode: PercentDecodeMode) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = hex_digit_value(*bytes.get(index + 1)?)?;
                let low = hex_digit_value(*bytes.get(index + 2)?)?;
                let decoded = (high << 4) | low;
                if mode == PercentDecodeMode::Uri && is_uri_reserved(decoded) {
                    output.extend_from_slice(&bytes[index..index + 3]);
                } else {
                    output.push(decoded);
                }
                index += 3;
            }
            b'+' if mode == PercentDecodeMode::Form => {
                output.push(b' ');
                index += 1;
            }
            other => {
                output.push(other);
                index += 1;
            }
        }
    }

    Some(output)
}

fn uri_percent_encode(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%'
            && bytes
                .get(index + 1)
                .is_some_and(|byte| hex_digit_value(*byte).is_some())
            && bytes
                .get(index + 2)
                .is_some_and(|byte| hex_digit_value(*byte).is_some())
        {
            output.extend_from_slice(&bytes[index..index + 3]);
            index += 3;
            continue;
        }

        if is_uri_literal(byte) {
            output.push(byte);
        } else {
            output.push(b'%');
            output.push(HEX_DIGITS[usize::from(byte >> 4)]);
            output.push(HEX_DIGITS[usize::from(byte & 0x0f)]);
        }
        index += 1;
    }

    output
}

const fn is_uri_literal(byte: u8) -> bool {
    matches!(
        byte,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b':'
            | b'/'
            | b'?'
            | b'#'
            | b'['
            | b']'
            | b'@'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
    )
}

const fn is_uri_reserved(byte: u8) -> bool {
    matches!(
        byte,
        b':' | b'/'
            | b'?'
            | b'#'
            | b'['
            | b']'
            | b'@'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
    )
}

fn base64_encode(
    context: &Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    padded: fn(&[u8]) -> String,
    unpadded: fn(&[u8]) -> String,
) -> Value {
    let bytes = arguments.bytes(0);
    let padding = arguments.bool(1);
    let encoded = if padding {
        padded(bytes)
    } else {
        unpadded(bytes)
    };
    context.string(encoded.as_bytes())
}

fn base64_decode(
    context: &Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    padded: fn(&str) -> Option<Vec<u8>>,
    unpadded: fn(&str) -> Option<Vec<u8>>,
) -> Value {
    let encoded = arguments.bytes(0);
    let padding = arguments.bool(1);
    let Ok(encoded) = from_utf8(encoded) else {
        return Value::null();
    };

    let decoded = if padding {
        padded(encoded)
    } else {
        unpadded(encoded)
    };
    decoded.map_or_else(Value::null, |bytes| context.string(&bytes))
}

#[whim_function("Whim\\_Private\\base64_encode_standard(string $bytes, bool $padding): string")]
fn base64_encode_standard(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    base64_encode(
        context,
        arguments,
        base64ct::Base64::encode_string,
        base64ct::Base64Unpadded::encode_string,
    )
}

#[whim_function(
    "Whim\\_Private\\base64_decode_standard(string $encoded, bool $padding): null|string"
)]
fn base64_decode_standard(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    base64_decode(
        context,
        arguments,
        |encoded| base64ct::Base64::decode_vec(encoded).ok(),
        |encoded| base64ct::Base64Unpadded::decode_vec(encoded).ok(),
    )
}

#[whim_function("Whim\\_Private\\base64_encode_url_safe(string $bytes, bool $padding): string")]
fn base64_encode_url_safe(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    base64_encode(
        context,
        arguments,
        base64ct::Base64Url::encode_string,
        base64ct::Base64UrlUnpadded::encode_string,
    )
}

#[whim_function(
    "Whim\\_Private\\base64_decode_url_safe(string $encoded, bool $padding): null|string"
)]
fn base64_decode_url_safe(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    base64_decode(
        context,
        arguments,
        |encoded| base64ct::Base64Url::decode_vec(encoded).ok(),
        |encoded| base64ct::Base64UrlUnpadded::decode_vec(encoded).ok(),
    )
}

#[whim_function("Whim\\_Private\\base64_encode_dot_slash(string $bytes, bool $padding): string")]
fn base64_encode_dot_slash(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    base64_encode(
        context,
        arguments,
        |bytes| pad_base64(base64ct::Base64Bcrypt::encode_string(bytes)),
        base64ct::Base64Bcrypt::encode_string,
    )
}

#[whim_function(
    "Whim\\_Private\\base64_decode_dot_slash(string $encoded, bool $padding): null|string"
)]
fn base64_decode_dot_slash(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    base64_decode(
        context,
        arguments,
        |encoded| {
            let stripped = strip_base64_padding(encoded)?;
            base64ct::Base64Bcrypt::decode_vec(stripped).ok()
        },
        |encoded| base64ct::Base64Bcrypt::decode_vec(encoded).ok(),
    )
}

#[expect(
    deprecated,
    reason = "the API exposes the exact nonstandard dot-slash-ordered Base64Crypt alphabet"
)]
#[whim_function(
    "Whim\\_Private\\base64_encode_dot_slash_ordered(string $bytes, bool $padding): string"
)]
fn base64_encode_dot_slash_ordered(
    context: &Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Value {
    base64_encode(
        context,
        arguments,
        |bytes| pad_base64(base64ct::Base64Crypt::encode_string(bytes)),
        base64ct::Base64Crypt::encode_string,
    )
}

#[expect(
    deprecated,
    reason = "the API exposes the exact nonstandard dot-slash-ordered Base64Crypt alphabet"
)]
#[whim_function(
    "Whim\\_Private\\base64_decode_dot_slash_ordered(string $encoded, bool $padding): null|string"
)]
fn base64_decode_dot_slash_ordered(
    context: &Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Value {
    base64_decode(
        context,
        arguments,
        |encoded| {
            let stripped = strip_base64_padding(encoded)?;
            base64ct::Base64Crypt::decode_vec(stripped).ok()
        },
        |encoded| base64ct::Base64Crypt::decode_vec(encoded).ok(),
    )
}

#[whim_function("Whim\\_Private\\base32_encode_standard(string $bytes, bool $padding): string")]
fn base32_encode_standard(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let padding = arguments.bool(1);
    let encoded = if padding {
        base32ct::Base32Upper::encode_string(bytes)
    } else {
        base32ct::Base32UpperUnpadded::encode_string(bytes)
    };
    context.string(encoded.as_bytes())
}

#[whim_function(
    "Whim\\_Private\\base32_decode_standard(string $encoded, bool $padding): null|string"
)]
fn base32_decode_standard(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let encoded = arguments.bytes(0);
    let padding = arguments.bool(1);
    let Ok(encoded) = from_utf8(encoded) else {
        return Value::null();
    };

    let uppercased = encoded.to_ascii_uppercase();
    let decoded = if padding {
        base32ct::Base32Upper::decode_vec(&uppercased).ok()
    } else {
        base32ct::Base32UpperUnpadded::decode_vec(&uppercased).ok()
    };
    decoded.map_or_else(Value::null, |bytes| context.string(&bytes))
}

#[whim_function("Whim\\_Private\\base32_encode_hex(string $bytes, bool $padding): string")]
fn base32_encode_hex(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let padding = arguments.bool(1);
    let encoded = if padding {
        data_encoding::BASE32HEX.encode(bytes)
    } else {
        data_encoding::BASE32HEX_NOPAD.encode(bytes)
    };
    context.string(encoded.as_bytes())
}

#[whim_function("Whim\\_Private\\base32_decode_hex(string $encoded, bool $padding): null|string")]
fn base32_decode_hex(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let encoded = arguments.bytes(0);
    let padding = arguments.bool(1);
    let encoded = encoded.to_ascii_uppercase();
    let decoded = if padding {
        data_encoding::BASE32HEX.decode(&encoded).ok()
    } else {
        data_encoding::BASE32HEX_NOPAD.decode(&encoded).ok()
    };
    decoded.map_or_else(Value::null, |bytes| context.string(&bytes))
}

#[whim_function("Whim\\_Private\\hex_encode(string $bytes): string")]
fn hex_encode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let encoded = lower::encode_string(bytes);
    context.string(encoded.as_bytes())
}

#[whim_function("Whim\\_Private\\hex_decode(string $encoded): null|string")]
fn hex_decode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    mixed::decode_vec(arguments.bytes(0))
        .map_or_else(|_| Value::null(), |bytes| context.string(&bytes))
}

#[whim_function("Whim\\_Private\\url_encode(string $bytes): string")]
fn url_encode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let encoded = percent_encode(bytes, &RESERVED).to_string();
    context.string(encoded.as_bytes())
}

#[whim_function("Whim\\_Private\\url_decode(string $encoded): null|string")]
fn url_decode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    strict_percent_decode(arguments.bytes(0), PercentDecodeMode::Component)
        .map_or_else(Value::null, |bytes| context.string(&bytes))
}

#[whim_function("Whim\\_Private\\uri_encode(string $bytes): string")]
fn uri_encode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    context.string(&uri_percent_encode(arguments.bytes(0)))
}

#[whim_function("Whim\\_Private\\uri_decode(string $encoded): null|string")]
fn uri_decode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    strict_percent_decode(arguments.bytes(0), PercentDecodeMode::Uri)
        .map_or_else(Value::null, |bytes| context.string(&bytes))
}

#[whim_function("Whim\\_Private\\url_encode_form(string $bytes): string")]
fn url_encode_form(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let mut encoded = String::with_capacity(bytes.len());
    for segment in percent_encode(bytes, &RESERVED) {
        if segment == "%20" {
            encoded.push('+');
        } else {
            encoded.push_str(segment);
        }
    }

    context.string(encoded.as_bytes())
}

#[whim_function("Whim\\_Private\\url_decode_form(string $encoded): null|string")]
fn url_decode_form(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    strict_percent_decode(arguments.bytes(0), PercentDecodeMode::Form)
        .map_or_else(Value::null, |bytes| context.string(&bytes))
}

#[whim_function("Whim\\_Private\\punycode_encode(string $input): null|string")]
fn punycode_encode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let input = arguments.bytes(0);
    let Ok(input) = from_utf8(input) else {
        return Value::null();
    };

    punycode::encode(input).map_or_else(
        |()| Value::null(),
        |encoded| context.string(encoded.as_bytes()),
    )
}

#[whim_function("Whim\\_Private\\punycode_decode(string $input): null|string")]
fn punycode_decode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let input = arguments.bytes(0);
    let Ok(input) = from_utf8(input) else {
        return Value::null();
    };

    punycode::decode(input).map_or_else(
        |()| Value::null(),
        |decoded| context.string(decoded.as_bytes()),
    )
}
