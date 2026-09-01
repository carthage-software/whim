//! HTTP message validation.

use std::str::from_utf8;

use httparse::EMPTY_HEADER;
use httparse::Header as HttpHeader;
use httparse::Request as HttpRequest;
use httparse::Response as HttpResponse;
use httparse::Status as HttpParseStatus;
use memchr::memchr_iter;
use memchr::memmem::find;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::core::private::url::is_valid_http_authority;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::dict::DictObject;
use crate::value::dict::keys::Key;
use crate::value::dict::keys::KeyRef;
use crate::value::vec::VecObject;

const COMMON_HTTP_HEADER_CAPACITY: usize = 32;

#[whim_function("Whim\\_Private\\http_valid_token(string $token): bool")]
pub(crate) fn valid_token(arguments: Arguments<'_>) -> Value {
    Value::bool(is_token(arguments.bytes(0)))
}

#[whim_function("Whim\\_Private\\http_valid_field_value(string $value): bool")]
pub(crate) fn valid_field_value(arguments: Arguments<'_>) -> Value {
    Value::bool(is_field_value(arguments.bytes(0)))
}

#[whim_function("Whim\\_Private\\http_valid_lowercase_field_name(string $name): bool")]
pub(crate) fn valid_lowercase_field_name(arguments: Arguments<'_>) -> Value {
    Value::bool(is_lowercase_field_name(arguments.bytes(0)))
}

#[whim_function("Whim\\_Private\\http_valid_compressed_field_value(string $value): bool")]
pub(crate) fn valid_compressed_field_value(arguments: Arguments<'_>) -> Value {
    Value::bool(is_compressed_field_value(arguments.bytes(0)))
}

#[whim_function("Whim\\_Private\\http_valid_request_target(string $target): bool")]
pub(crate) fn valid_request_target(arguments: Arguments<'_>) -> Value {
    Value::bool(is_request_target(arguments.bytes(0)))
}

#[whim_function(
    "Whim\\_Private\\http_valid_request_pseudo_fields(string $method, string $scheme, string $path): bool"
)]
pub(crate) fn valid_request_pseudo_fields(arguments: Arguments<'_>) -> Value {
    Value::bool(is_request_pseudo_fields(
        arguments.bytes(0),
        arguments.bytes(1),
        arguments.bytes(2),
    ))
}

#[whim_function(
    "Whim\\_Private\\http_index_fields(vec<(string, string)> $fields): null|dict<string, vec<string>>"
)]
pub(crate) fn index_fields(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let fields = arguments.vec(0);
    let mut values = DictObject::new(context.vm.heap());
    values.make_mut().reserve_for_build(fields.len());

    for field in fields.iter() {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let pair = unsafe { unwrap_option_invariant(field.as_tuple(), "an HTTP field is a pair") };
        let [name, value] = pair.as_slice() else {
            return Value::null();
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let name_bytes = unsafe {
            unwrap_option_invariant(name.as_string_bytes(), "an HTTP field name is a string")
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let value_bytes = unsafe {
            unwrap_option_invariant(value.as_string_bytes(), "an HTTP field value is a string")
        };
        if !is_token(name_bytes) || !is_field_value(value_bytes) {
            return Value::null();
        }

        let normalized = if name_bytes.iter().any(u8::is_ascii_uppercase) {
            context.string(&name_bytes.to_ascii_lowercase())
        } else {
            name.clone()
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let key = unsafe {
            unwrap_option_invariant(
                Key::from_owned_value(normalized),
                "a normalized HTTP field name is a dict key",
            )
        };
        if let Some(group) = values.make_mut().get_mut_ref(KeyRef::from(&key)) {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let group = unsafe {
                unwrap_option_invariant(group.as_vec_mut(), "an HTTP field group is a vec")
            };
            group.make_mut().push(value.clone());
        } else {
            let group = VecObject::with_elements(context.vm.heap(), [value.clone()]);
            values.make_mut().insert(key, Value::vec(group));
        }
    }

    Value::dict(values)
}

#[whim_function(
    "Whim\\_Private\\http1_parse_request_head(string $block): int|(string, string, int, vec<(string, string)>, dict<string, vec<string>>, null|int, bool, bool, bool, bool, null|string, null|string)"
)]
pub(crate) fn parse_request_head(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let block = arguments.bytes(0);
    let parsed = match parse_request(block) {
        Ok(parsed) => parsed,
        Err(error) => return Value::int(error.status()),
    };
    let (fields, field_index) = materialize_indexed_fields(context, &parsed.fields);
    let host = parsed
        .host
        .map_or_else(Value::null, |host| context.string(host));
    let upgrade = parsed.upgrade.map_or_else(Value::null, |upgrade| {
        context.string(&upgrade.to_ascii_lowercase())
    });

    context.tuple([
        context.string(parsed.method),
        context.string(parsed.target),
        Value::int(parsed.version),
        fields,
        field_index,
        parsed.content_length.map_or_else(Value::null, Value::int),
        Value::bool(parsed.chunked),
        Value::bool(parsed.connection_close),
        Value::bool(parsed.connection_keep_alive),
        Value::bool(parsed.expect_continue),
        host,
        upgrade,
    ])
}

#[whim_function(
    "Whim\\_Private\\http1_parse_response_head(string $block): null|(int, int, vec<(string, string)>, dict<string, vec<string>>)"
)]
pub(crate) fn parse_response_head(
    context: &Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Value {
    let block = arguments.bytes(0);
    let Some(parsed) = parse_response(block) else {
        return Value::null();
    };
    let (fields, field_index) = materialize_indexed_fields(context, &parsed.fields);

    context.tuple([
        Value::int(parsed.version),
        Value::int(parsed.status),
        fields,
        field_index,
    ])
}

fn materialize_indexed_fields(
    context: &Context<'_, '_, '_>,
    fields: &[(&[u8], &[u8])],
) -> (Value, Value) {
    let mut ordered = VecObject::new(context.vm.heap());
    ordered.make_mut().reserve_hint(fields.len());
    let mut index = DictObject::new(context.vm.heap());
    index.make_mut().reserve_for_build(fields.len());

    for (name_bytes, value_bytes) in fields {
        let name = context.string(name_bytes);
        let value = context.string(value_bytes);
        ordered
            .make_mut()
            .push(context.tuple([name.clone(), value.clone()]));

        let normalized = if name_bytes.iter().any(u8::is_ascii_uppercase) {
            context.string(&name_bytes.to_ascii_lowercase())
        } else {
            name
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let key = unsafe {
            unwrap_option_invariant(
                Key::from_owned_value(normalized),
                "a normalized HTTP field name is a dict key",
            )
        };
        if let Some(group) = index.make_mut().get_mut_ref(KeyRef::from(&key)) {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let group = unsafe {
                unwrap_option_invariant(group.as_vec_mut(), "an HTTP field group is a vec")
            };
            group.make_mut().push(value);
        } else {
            let group = VecObject::with_elements(context.vm.heap(), [value]);
            index.make_mut().insert(key, Value::vec(group));
        }
    }

    (Value::vec(ordered), Value::dict(index))
}

struct ParsedResponse<'a> {
    version: i64,
    status: i64,
    fields: Vec<(&'a [u8], &'a [u8])>,
}

fn parse_response(block: &[u8]) -> Option<ParsedResponse<'_>> {
    let header_count = strict_http_header_count(block)?;

    if header_count <= COMMON_HTTP_HEADER_CAPACITY {
        let mut headers = [EMPTY_HEADER; COMMON_HTTP_HEADER_CAPACITY];
        parse_response_with_headers(block, &mut headers)
    } else {
        let mut headers = vec![EMPTY_HEADER; header_count];
        parse_response_with_headers(block, &mut headers)
    }
}

fn parse_response_with_headers<'block>(
    block: &'block [u8],
    headers: &mut [HttpHeader<'block>],
) -> Option<ParsedResponse<'block>> {
    let mut response = HttpResponse::new(headers);
    let HttpParseStatus::Complete(consumed) = response.parse(block).ok()? else {
        return None;
    };
    if consumed != block.len() {
        return None;
    }

    let version = match response.version {
        Some(0) => 10,
        Some(1) => 11,
        _ => return None,
    };
    let Some(status @ 100..=599) = response.code else {
        return None;
    };
    let fields = response
        .headers
        .iter()
        .map(|header| (header.name.as_bytes(), header.value))
        .collect();

    Some(ParsedResponse {
        version,
        status: i64::from(status),
        fields,
    })
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "the parser returns independent HTTP framing flags"
)]
struct ParsedRequest<'a> {
    method: &'a [u8],
    target: &'a [u8],
    version: i64,
    fields: Vec<(&'a [u8], &'a [u8])>,
    content_length: Option<i64>,
    chunked: bool,
    connection_close: bool,
    connection_keep_alive: bool,
    expect_continue: bool,
    host: Option<&'a [u8]>,
    upgrade: Option<&'a [u8]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestParseError {
    BadRequest,
    ExpectationFailed,
    VersionNotSupported,
}

impl RequestParseError {
    const fn status(self) -> i64 {
        match self {
            Self::BadRequest => 400,
            Self::ExpectationFailed => 417,
            Self::VersionNotSupported => 505,
        }
    }
}

fn parse_request(block: &[u8]) -> Result<ParsedRequest<'_>, RequestParseError> {
    let header_count =
        strict_http_header_count(block).ok_or_else(|| classify_request_parse_error(block))?;

    if header_count <= COMMON_HTTP_HEADER_CAPACITY {
        let mut headers = [EMPTY_HEADER; COMMON_HTTP_HEADER_CAPACITY];
        parse_request_with_headers(block, &mut headers)
    } else {
        let mut headers = vec![EMPTY_HEADER; header_count];
        parse_request_with_headers(block, &mut headers)
    }
}

fn parse_request_with_headers<'block>(
    block: &'block [u8],
    headers: &mut [HttpHeader<'block>],
) -> Result<ParsedRequest<'block>, RequestParseError> {
    let mut request = HttpRequest::new(headers);
    let parsed = request
        .parse(block)
        .map_err(|_| classify_request_parse_error(block))?;
    let HttpParseStatus::Complete(consumed) = parsed else {
        return Err(RequestParseError::BadRequest);
    };
    if consumed != block.len() {
        return Err(RequestParseError::BadRequest);
    }

    let method = request.method.ok_or(RequestParseError::BadRequest)?;
    let target = request.path.ok_or(RequestParseError::BadRequest)?;
    let method = method.as_bytes();
    let target = target.as_bytes();
    if !is_request_target(target) {
        return Err(RequestParseError::BadRequest);
    }
    let version = match request.version {
        Some(0) => 10,
        Some(1) => 11,
        _ => {
            return Err(RequestParseError::VersionNotSupported);
        }
    };

    let mut fields = Vec::new();
    let mut host = None;
    let mut host_count = 0;
    let mut transfer_coding_count = 0;
    let mut chunked = false;
    let mut last_transfer_coding_chunked = false;
    let mut expectation_count = 0;
    let mut expectation_supported = false;
    let mut connection_close = false;
    let mut connection_keep_alive = false;
    let mut upgrade = None;
    fields.reserve(request.headers.len());
    for header in request.headers.iter() {
        let name = header.name.as_bytes();
        let value = header.value;
        fields.push((name, value));

        if name.eq_ignore_ascii_case(b"host") {
            host = Some(value);
            host_count += 1;
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            for token in field_tokens(value) {
                transfer_coding_count += 1;
                last_transfer_coding_chunked = token.eq_ignore_ascii_case(b"chunked");
                chunked |= last_transfer_coding_chunked;
            }
        } else if name.eq_ignore_ascii_case(b"expect") {
            for token in field_tokens(value) {
                expectation_count += 1;
                expectation_supported = token.eq_ignore_ascii_case(b"100-continue");
            }
        } else if name.eq_ignore_ascii_case(b"connection") {
            for token in field_tokens(value) {
                connection_close |= token.eq_ignore_ascii_case(b"close");
                connection_keep_alive |= token.eq_ignore_ascii_case(b"keep-alive");
            }
        } else if name.eq_ignore_ascii_case(b"upgrade") && upgrade.is_none() {
            upgrade = field_tokens(value).next();
        }
    }
    if host_count > 1 {
        return Err(RequestParseError::BadRequest);
    }
    if let Some(host) = host {
        let valid = from_utf8(host).is_ok_and(is_valid_http_authority);
        if !valid {
            return Err(RequestParseError::BadRequest);
        }
    } else if version == 11 {
        return Err(RequestParseError::BadRequest);
    }
    if chunked && !last_transfer_coding_chunked {
        return Err(RequestParseError::BadRequest);
    }
    if expectation_count != 0 && (expectation_count != 1 || !expectation_supported) {
        return Err(RequestParseError::ExpectationFailed);
    }

    let content_length = if transfer_coding_count == 0 {
        parse_content_length(&fields)?
    } else {
        None
    };
    connection_close |= transfer_coding_count != 0 && !chunked;

    Ok(ParsedRequest {
        method,
        target,
        version,
        fields,
        content_length,
        chunked,
        connection_close,
        connection_keep_alive,
        expect_continue: expectation_count != 0,
        host,
        upgrade,
    })
}

fn strict_http_header_count(block: &[u8]) -> Option<usize> {
    if !block.ends_with(b"\r\n\r\n") || block.starts_with(b"\r\n") {
        return None;
    }

    let mut line_count = 0_usize;
    for index in memchr_iter(b'\n', block) {
        if index == 0 || block[index - 1] != b'\r' {
            return None;
        }
        line_count += 1;
    }

    line_count.checked_sub(2)
}

fn classify_request_parse_error(block: &[u8]) -> RequestParseError {
    if !block.ends_with(b"\r\n\r\n") {
        return RequestParseError::BadRequest;
    }
    let Some(request_line_end) = find(block, b"\r\n") else {
        return RequestParseError::BadRequest;
    };
    let mut request_line = block[..request_line_end].split(|byte| *byte == b' ');
    let Some(method) = request_line.next() else {
        return RequestParseError::BadRequest;
    };
    let Some(target) = request_line.next() else {
        return RequestParseError::BadRequest;
    };
    let Some(version) = request_line.next() else {
        return RequestParseError::BadRequest;
    };
    if method.is_empty()
        || target.is_empty()
        || version.is_empty()
        || request_line.next().is_some()
        || !is_token(method)
        || !is_request_target(target)
    {
        return RequestParseError::BadRequest;
    }

    if matches!(version, b"HTTP/1.0" | b"HTTP/1.1") {
        RequestParseError::BadRequest
    } else {
        RequestParseError::VersionNotSupported
    }
}

fn parse_content_length(fields: &[(&[u8], &[u8])]) -> Result<Option<i64>, RequestParseError> {
    let mut result = None;
    for (name, value) in fields {
        if !name.eq_ignore_ascii_case(b"content-length") {
            continue;
        }
        for value in value.split(|byte| *byte == b',') {
            let value = parse_decimal(trim_optional_whitespace(value))?;
            if result.is_some_and(|current| current != value) {
                return Err(RequestParseError::BadRequest);
            }
            result = Some(value);
        }
    }
    Ok(result)
}

fn parse_decimal(value: &[u8]) -> Result<i64, RequestParseError> {
    if value.is_empty() {
        return Err(RequestParseError::BadRequest);
    }
    let mut result = 0_i64;
    for byte in value {
        let digit = byte
            .checked_sub(b'0')
            .filter(|digit| *digit <= 9)
            .ok_or(RequestParseError::BadRequest)?;
        result = result
            .checked_mul(10)
            .and_then(|result| result.checked_add(i64::from(digit)))
            .ok_or(RequestParseError::BadRequest)?;
    }
    Ok(result)
}

fn trim_optional_whitespace(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[..value.len() - 1];
    }
    value
}

fn field_tokens(value: &[u8]) -> impl Iterator<Item = &[u8]> {
    value
        .split(|byte| *byte == b',')
        .map(trim_optional_whitespace)
        .filter(|token| !token.is_empty())
}

#[whim_function(
    "Whim\\_Private\\http1_serialize_request_head(string $method, string $target, string $version, vec<(string, string)> $fields): string"
)]
pub(crate) fn serialize_request_head(
    context: &Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Value {
    let method = arguments.bytes(0);
    let target = arguments.bytes(1);
    let version = arguments.bytes(2);
    let fields = arguments.vec(3);
    let mut head = Vec::with_capacity(
        method.len() + target.len() + version.len() + 4 + serialized_fields_len(&fields),
    );
    head.extend_from_slice(method);
    head.push(b' ');
    head.extend_from_slice(target);
    head.push(b' ');
    head.extend_from_slice(version);
    head.extend_from_slice(b"\r\n");
    append_fields(&mut head, &fields);
    head.extend_from_slice(b"\r\n");

    Value::from_string_vec(context.vm.heap(), head)
}

#[whim_function(
    "Whim\\_Private\\http1_serialize_response_head(string $version, 100..=599 $status, string $reason, vec<(string, string)> $fields, bool $bodyAllowed, bool $chunked, bool $addContentLength, bool $close, string $date): string"
)]
pub(crate) fn serialize_response_head(
    context: &Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Value {
    let version = arguments.bytes(0);
    let status = arguments.int(1);
    let reason = arguments.bytes(2);
    let fields = arguments.vec(3);
    let body_allowed = arguments.bool(4);
    let chunked = arguments.bool(5);
    let add_content_length = arguments.bool(6);
    let close = arguments.bool(7);
    let date = arguments.bytes(8);
    let mut status_buffer = itoa::Buffer::new();
    let status = status_buffer.format(status).as_bytes();
    let mut head = Vec::with_capacity(
        version.len() + status.len() + reason.len() + 96 + serialized_fields_len(&fields),
    );
    head.extend_from_slice(version);
    head.push(b' ');
    head.extend_from_slice(status);
    head.push(b' ');
    head.extend_from_slice(reason);
    head.extend_from_slice(b"\r\n");

    let mut has_server = false;
    let mut has_date = false;
    for field in fields.iter() {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let pair = unsafe { unwrap_option_invariant(field.as_tuple(), "an HTTP field is a pair") };
        let [name, value] = pair.as_slice() else {
            continue;
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let name = unsafe {
            unwrap_option_invariant(name.as_string_bytes(), "an HTTP field name is a string")
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let value = unsafe {
            unwrap_option_invariant(value.as_string_bytes(), "an HTTP field value is a string")
        };
        let lowercase;
        let lowered = if name.iter().any(u8::is_ascii_uppercase) {
            lowercase = name.to_ascii_lowercase();
            &lowercase
        } else {
            name
        };
        if (lowered == b"transfer-encoding" && (!body_allowed || chunked))
            || (lowered == b"content-length"
                && !body_allowed
                && (status[0] == b'1' || status == b"204"))
            || (lowered == b"connection" && (close || version == b"HTTP/1.0"))
        {
            continue;
        }
        has_server |= lowered == b"server";
        has_date |= lowered == b"date";
        append_field(&mut head, lowered, value);
    }

    if !has_server {
        append_field(&mut head, b"server", b"Whim");
    }
    if !has_date {
        append_field(&mut head, b"date", date);
    }
    if add_content_length {
        append_field(&mut head, b"content-length", b"0");
    }
    if chunked {
        append_field(&mut head, b"transfer-encoding", b"chunked");
    }
    if close {
        append_field(&mut head, b"connection", b"close");
    } else if version == b"HTTP/1.0" {
        append_field(&mut head, b"connection", b"keep-alive");
    }
    head.extend_from_slice(b"\r\n");

    Value::from_string_vec(context.vm.heap(), head)
}

fn append_field(output: &mut Vec<u8>, name: &[u8], value: &[u8]) {
    output.extend_from_slice(name);
    output.extend_from_slice(b": ");
    output.extend_from_slice(value);
    output.extend_from_slice(b"\r\n");
}

fn serialized_fields_len(fields: &VecObject) -> usize {
    fields
        .iter()
        .filter_map(Value::as_tuple)
        .filter_map(|field| {
            let [name, value] = field.as_slice() else {
                return None;
            };
            Some(name.as_string_bytes()?.len() + value.as_string_bytes()?.len() + 4)
        })
        .sum()
}

fn append_fields(output: &mut Vec<u8>, fields: &VecObject) {
    for field in fields.iter() {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let pair = unsafe { unwrap_option_invariant(field.as_tuple(), "an HTTP field is a pair") };
        let [name, value] = pair.as_slice() else {
            continue;
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let name = unsafe {
            unwrap_option_invariant(name.as_string_bytes(), "an HTTP field name is a string")
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let value = unsafe {
            unwrap_option_invariant(value.as_string_bytes(), "an HTTP field value is a string")
        };
        append_field(output, name, value);
    }
}

fn is_token(token: &[u8]) -> bool {
    !token.is_empty()
        && token.iter().all(|byte| {
            matches!(
                byte,
                b'0'..=b'9'
                    | b'A'..=b'Z'
                    | b'a'..=b'z'
                    | b'!'
                    | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            )
        })
}

fn is_field_value(value: &[u8]) -> bool {
    value
        .iter()
        .all(|byte| matches!(byte, b'\t' | 0x20..=0x7e | 0x80..=0xff))
}

fn is_lowercase_field_name(name: &[u8]) -> bool {
    let token = name.strip_prefix(b":").unwrap_or(name);
    is_token(token) && !token.iter().any(u8::is_ascii_uppercase)
}

fn is_compressed_field_value(value: &[u8]) -> bool {
    let has_surrounding_whitespace = value
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        || value
            .last()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'));

    !has_surrounding_whitespace && is_field_value(value)
}

fn is_request_target(target: &[u8]) -> bool {
    !target.is_empty()
        && target
            .iter()
            .all(|byte| matches!(byte, 0x21..=0x22 | 0x24..=0x7e))
}

fn is_request_pseudo_fields(method: &[u8], scheme: &[u8], path: &[u8]) -> bool {
    if !is_token(method) || !is_scheme(scheme) {
        return false;
    }

    if path == b"*" {
        return method == b"OPTIONS";
    }

    if matches!(scheme, b"http" | b"https") && !path.starts_with(b"/") {
        return false;
    }

    path.is_empty() || is_request_target(path)
}

fn is_scheme(scheme: &[u8]) -> bool {
    scheme.first().is_some_and(u8::is_ascii_alphabetic)
        && scheme
            .iter()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use crate::core::private::http::RequestParseError;
    use crate::core::private::http::is_compressed_field_value;
    use crate::core::private::http::is_field_value;
    use crate::core::private::http::is_lowercase_field_name;
    use crate::core::private::http::is_request_pseudo_fields;
    use crate::core::private::http::is_request_target;
    use crate::core::private::http::is_token;
    use crate::core::private::http::parse_request;
    use crate::core::private::http::parse_response;
    use crate::core::private::url::is_valid_http_authority;

    #[test]
    fn validates_http_tokens() {
        assert!(is_token(b"PATCH"));
        assert!(is_token(b"custom!#$%&'*+-.^_`|~"));
        assert!(!is_token(b""));
        assert!(!is_token(b"two words"));
        assert!(!is_token(b"bad:name"));
    }

    #[test]
    fn validates_http_field_values() {
        assert!(is_field_value(b"text\twith spaces"));
        assert!(is_field_value(&[0x80, 0xff]));
        assert!(!is_field_value(b"line\r\nbreak"));
        assert!(!is_field_value(&[0x7f]));
    }

    #[test]
    fn validates_compressed_http_fields() {
        assert!(is_lowercase_field_name(b"content-type"));
        assert!(is_lowercase_field_name(b":method"));
        assert!(!is_lowercase_field_name(b"Content-Type"));
        assert!(!is_lowercase_field_name(b"bad?name"));
        assert!(!is_lowercase_field_name(b"bad:name"));
        assert!(!is_lowercase_field_name(b"::method"));
        assert!(is_compressed_field_value(b"text with spaces"));
        assert!(!is_compressed_field_value(b" padded"));
        assert!(!is_compressed_field_value(b"padded\t"));
        assert!(!is_compressed_field_value(b"line\nbreak"));
    }

    #[test]
    fn validates_http_request_targets() {
        assert!(is_request_target(b"/path?q=1"));
        assert!(is_request_target(b"*"));
        assert!(is_request_target(b"example.com:443"));
        assert!(!is_request_target(b""));
        assert!(!is_request_target(b"/fragment#bad"));
        assert!(!is_request_target(b"/line\nbreak"));
    }

    #[test]
    fn validates_http_authorities() {
        assert!(is_valid_http_authority("example.com"));
        assert!(is_valid_http_authority("example.com:8080"));
        assert!(is_valid_http_authority("127.0.0.1:8080"));
        assert!(is_valid_http_authority("[::1]:8080"));
        assert!(!is_valid_http_authority(""));
        assert!(!is_valid_http_authority("user@example.com"));
        assert!(!is_valid_http_authority("example.com:"));
        assert!(!is_valid_http_authority("example.com:65536"));
        assert!(!is_valid_http_authority("example.com/path"));
    }

    #[test]
    fn validates_request_pseudo_fields() {
        assert!(is_request_pseudo_fields(b"GET", b"https", b"/"));
        assert!(is_request_pseudo_fields(b"OPTIONS", b"https", b"*"));
        assert!(is_request_pseudo_fields(b"FETCH", b"custom+v1", b""));
        assert!(!is_request_pseudo_fields(b"bad method", b"https", b"/"));
        assert!(!is_request_pseudo_fields(b"GET", b"", b"/"));
        assert!(!is_request_pseudo_fields(b"GET", b"https", b""));
        assert!(!is_request_pseudo_fields(b"GET", b"https", b"relative"));
        assert!(!is_request_pseudo_fields(b"GET", b"https", b"*"));
        assert!(!is_request_pseudo_fields(
            b"GET",
            b"https",
            b"/bad#fragment"
        ));
    }

    #[test]
    fn parses_strict_http_heads() {
        let request = parse_request(
            b"POST /path HTTP/1.1\r\nHost: example.com\r\nContent-Length: 2, 2\r\n\r\n",
        )
        .map(|request| {
            (
                request.method,
                request.target,
                request.version,
                request.fields.len(),
                request.content_length,
            )
        });
        assert_eq!(
            request,
            Ok((b"POST".as_slice(), b"/path".as_slice(), 11, 2, Some(2)))
        );

        let response = parse_response(b"HTTP/1.0 204 No Content\r\nX-Test: yes\r\n\r\n")
            .map(|response| (response.version, response.status, response.fields.len()));
        assert_eq!(response, Some((10, 204, 1)));
    }

    #[test]
    fn preserves_request_error_statuses() {
        assert_eq!(
            parse_request(b"GET  / HTTP/1.1\r\nHost: example.com\r\n\r\n").err(),
            Some(RequestParseError::BadRequest)
        );
        assert_eq!(
            parse_request(b"GET / HTTP/2.0\r\nHost: example.com\r\n\r\n").err(),
            Some(RequestParseError::VersionNotSupported)
        );
        assert_eq!(
            parse_request(b"POST / HTTP/1.1\r\nHost: example.com\r\nExpect: other\r\n\r\n").err(),
            Some(RequestParseError::ExpectationFailed)
        );
    }

    #[test]
    fn parses_more_than_common_header_limit() {
        let mut request = b"GET / HTTP/1.1\r\nHost: example.com\r\n".to_vec();
        let mut response = b"HTTP/1.1 200 OK\r\n".to_vec();
        for index in 0..48 {
            let field = format!("X-Field-{index}: value\r\n");
            request.extend_from_slice(field.as_bytes());
            response.extend_from_slice(field.as_bytes());
        }
        request.extend_from_slice(b"\r\n");
        response.extend_from_slice(b"\r\n");

        assert_eq!(
            parse_request(&request).map(|head| head.fields.len()),
            Ok(49)
        );
        assert_eq!(
            parse_response(&response).map(|head| head.fields.len()),
            Some(48)
        );
    }

    #[test]
    fn rejects_non_crlf_heads_and_invalid_reason_phrases() {
        assert_eq!(
            parse_request(b"GET / HTTP/1.1\nHost: example.com\r\n\r\n").err(),
            Some(RequestParseError::BadRequest)
        );
        assert!(parse_response(b"HTTP/1.1 200 OK\n\r\n").is_none());
        assert!(parse_response(b"HTTP/1.1 200 \x7f\r\n\r\n").is_none());
    }
}
