//! Strict URL parsing and canonicalization.

use std::net::Ipv6Addr;
use std::str::from_utf8;

use iri_string::build::Builder;
use iri_string::components::AuthorityComponents;
use iri_string::types::UriReferenceStr;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::core::private::iri::encode_reference;
use crate::core::private::iri::idna_ascii;
use crate::core::private::uri_common::host_value;
use crate::value::Value;

#[whim_function(
    "Whim\\_Private\\url_parse_reference(string $url): null|(string, string, string, null|string, null|string)"
)]
pub(crate) fn parse_reference(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Some(input) = required_utf8(arguments, 0) else {
        return Value::null();
    };
    let Some(url) = canonicalize(input) else {
        return Value::null();
    };

    context.tuple([
        context.string(url.scheme.as_bytes()),
        context.string(url.authority.as_bytes()),
        context.string(url.path.as_bytes()),
        optional_string(context, url.query.as_deref()),
        optional_string(context, url.fragment.as_deref()),
    ])
}

#[whim_function(
    "Whim\\_Private\\url_parse_authority(string $authority): null|(null|string, 'future'|'name'|'ipv4'|'ipv6', string, null|Whim\\Refine\\Uint16)"
)]
pub(crate) fn parse_authority(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Some(authority) = required_utf8(arguments, 0) else {
        return Value::null();
    };
    let Some(normalized) = normalize_authority(authority, true) else {
        return Value::null();
    };
    let Ok(normalized) = UriReferenceStr::new(&normalized) else {
        return Value::null();
    };
    let Some(parts) = normalized.authority_components() else {
        return Value::null();
    };
    if parts.host().is_empty() || invalid_registered_name(&parts) {
        return Value::null();
    }
    let Ok(port) = parse_port(parts.port()) else {
        return Value::null();
    };
    let Some((kind, host)) = host_value(context, parts.host()) else {
        return Value::null();
    };

    context.tuple([
        optional_string(context, parts.userinfo()),
        context.string(kind),
        host,
        optional_int(port),
    ])
}

pub(crate) fn is_valid_http_authority(authority: &str) -> bool {
    if authority
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'/' | b'?' | b'#'))
    {
        return false;
    }

    let mut candidate = String::with_capacity(authority.len() + 4);
    candidate.push_str("x://");
    candidate.push_str(authority);
    let Ok(reference) = UriReferenceStr::new(&candidate) else {
        return false;
    };
    let Some(parts) = reference.authority_components() else {
        return false;
    };

    parts.userinfo().is_none()
        && !parts.host().is_empty()
        && !invalid_registered_name(&parts)
        && parse_port(parts.port()).is_ok()
        && valid_host(parts.host())
}

fn normalize_authority(authority: &str, allow_userinfo: bool) -> Option<String> {
    if authority
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'/' | b'?' | b'#'))
    {
        return None;
    }

    let mut candidate = String::with_capacity(authority.len() + 4);
    candidate.push_str("x://");
    candidate.push_str(authority);
    let candidate = encode_reference(&candidate)?;
    let reference = UriReferenceStr::new(&candidate).ok()?;
    let raw_parts = reference.authority_components()?;
    if (!allow_userinfo && raw_parts.userinfo().is_some()) || parse_port(raw_parts.port()).is_err()
    {
        return None;
    }

    let mut builder = Builder::from(reference);
    builder.normalize();
    let normalized = builder.build::<UriReferenceStr>().ok()?.to_string();
    let reference = UriReferenceStr::new(&normalized).ok()?;
    let parts = reference.authority_components()?;
    if parts.host().is_empty()
        || invalid_registered_name(&parts)
        || parse_port(parts.port()).is_err()
        || !valid_host(parts.host())
    {
        return None;
    }

    Some(normalized)
}

fn valid_host(host: &str) -> bool {
    let Some(literal) = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
    else {
        return true;
    };

    literal.starts_with('v') || literal.starts_with('V') || literal.parse::<Ipv6Addr>().is_ok()
}

struct CanonicalUrl {
    scheme: String,
    authority: String,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

fn canonicalize(input: &str) -> Option<CanonicalUrl> {
    let encoded = encode_reference(input)?;
    let reference = UriReferenceStr::new(&encoded).ok()?;
    parse_port(reference.authority_components()?.port()).ok()?;
    let mut builder = Builder::from(reference);
    builder.normalize();
    let normalized = builder.build::<UriReferenceStr>().ok()?.to_string();
    let reference = UriReferenceStr::new(&normalized).ok()?;
    let scheme = reference.scheme_str()?;
    let parts = reference.authority_components()?;
    if !reference.path_str().is_empty() && !reference.path_str().starts_with('/') {
        return None;
    }
    if invalid_registered_name(&parts) {
        return None;
    }
    if parts.host().is_empty() {
        return None;
    }
    let mut port = parse_port(parts.port()).ok()?;
    if port.is_some_and(|port| default_port(scheme) == Some(port)) {
        port = None;
    }

    Some(CanonicalUrl {
        scheme: scheme.to_string(),
        authority: authority(&parts, port),
        path: reference.path_str().to_string(),
        query: reference.query_str().map(str::to_string),
        fragment: reference.fragment_str().map(str::to_string),
    })
}

fn invalid_registered_name(parts: &AuthorityComponents<'_>) -> bool {
    parts
        .reg_name()
        .is_some_and(|name| name.as_bytes().contains(&b'%') || idna_ascii(name).is_none())
}

fn parse_port(port: Option<&str>) -> Result<Option<u16>, ()> {
    match port {
        Some("") => Err(()),
        Some(port) => port.parse().map(Some).map_err(|_| ()),
        None => Ok(None),
    }
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        "ftps" => Some(990),
        "ssh" | "sftp" => Some(22),
        "ldap" => Some(389),
        "ldaps" => Some(636),
        "redis" => Some(6379),
        "rediss" => Some(6380),
        "mysql" => Some(3306),
        "postgres" => Some(5432),
        "amqp" => Some(5672),
        "amqps" => Some(5671),
        "mqtt" => Some(1883),
        "mqtts" => Some(8883),
        "git" => Some(9418),
        "telnet" => Some(23),
        "dns" => Some(53),
        _ => None,
    }
}

fn authority(parts: &AuthorityComponents<'_>, port: Option<u16>) -> String {
    let capacity = parts.userinfo().map_or(0, |value| value.len() + 1)
        + parts.host().len()
        + port.map_or(0, |_| 6);
    let mut authority = String::with_capacity(capacity);
    if let Some(userinfo) = parts.userinfo() {
        authority.push_str(userinfo);
        authority.push('@');
    }
    authority.push_str(parts.host());
    if let Some(port) = port {
        authority.push(':');
        authority.push_str(&port.to_string());
    }

    authority
}

fn required_utf8(arguments: Arguments<'_>, index: usize) -> Option<&str> {
    from_utf8(arguments.bytes(index)).ok()
}

fn optional_string(context: &Context<'_, '_, '_>, value: Option<&str>) -> Value {
    let Some(value) = value else {
        return Value::null();
    };

    context.string(value.as_bytes())
}

fn optional_int(value: Option<u16>) -> Value {
    let Some(value) = value else {
        return Value::null();
    };

    Value::int(i64::from(value))
}
