//! IRI parsing, conversion, normalization, and resolution.

use std::str::from_utf8;

use idna::uts46::AsciiDenyList;
use idna::uts46::DnsLength;
use idna::uts46::Hyphens;
use idna::uts46::Uts46;
use iri_string::build::Builder;
use iri_string::components::AuthorityComponents;
use iri_string::types::IriAbsoluteStr;
use iri_string::types::IriReferenceStr;
use iri_string::types::UriReferenceStr;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::core::private::uri_common::compose;
use crate::core::private::uri_common::host_value;
use crate::core::private::uri_common::optional_string;
use crate::core::private::uri_common::optional_utf8;
use crate::core::private::uri_common::required_utf8;
use crate::core::private::uri_common::uri_reference_builtins;
use crate::value::Value;

uri_reference_builtins!(
    IriReferenceStr,
    IriAbsoluteStr,
    "Whim\\_Private\\iri_parse_reference(string $iri): null|(null|string, null|string, string, null|string, null|string)",
    "Whim\\_Private\\iri_parse_authority(string $authority): null|(null|string, 'future'|'name'|'ipv4'|'ipv6', string, null|string)",
    "Whim\\_Private\\iri_valid_components(null|string $scheme, null|string $authority, string $path, null|string $query, null|string $fragment): bool",
    "Whim\\_Private\\iri_normalize(string $iri): null|string",
    "Whim\\_Private\\iri_resolve(string $base, string $reference): null|string",
);

#[whim_function("Whim\\_Private\\iri_to_uri(string $iri): null|string")]
pub(crate) fn to_uri(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Some(input) = required_utf8(arguments, 0) else {
        return Value::null();
    };
    let Some(mapped) = encode_reference(input) else {
        return Value::null();
    };

    context.string(mapped.as_bytes())
}

pub(crate) fn encode_reference(input: &str) -> Option<String> {
    let Ok(iri) = IriReferenceStr::new(input) else {
        return None;
    };
    let encoded = iri.encode_to_uri().to_string();
    let encoded = UriReferenceStr::new(&encoded).ok()?;
    let authority = match (iri.authority_components(), encoded.authority_components()) {
        (Some(iri), Some(encoded)) => Some(encoded_authority(&iri, &encoded)),
        (None, None) => None,
        _ => return None,
    };
    let mapped = compose(
        encoded.scheme_str(),
        authority.as_deref(),
        encoded.path_str(),
        encoded.query_str(),
        encoded.fragment_str(),
    );
    if UriReferenceStr::new(&mapped).is_err() {
        return None;
    }

    Some(mapped)
}

#[whim_function("Whim\\_Private\\iri_from_uri(string $uri): null|string")]
pub(crate) fn from_uri(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Some(input) = required_utf8(arguments, 0) else {
        return Value::null();
    };
    let Ok(uri) = UriReferenceStr::new(input) else {
        return Value::null();
    };
    let authority = uri.authority_components().map(decoded_authority);
    let path = decode_non_ascii(uri.path_str());
    let query = uri.query_str().map(decode_non_ascii);
    let fragment = uri.fragment_str().map(decode_non_ascii);
    let mapped = compose(
        uri.scheme_str(),
        authority.as_deref(),
        &path,
        query.as_deref(),
        fragment.as_deref(),
    );
    let mapped = if IriReferenceStr::new(&mapped).is_ok() {
        mapped
    } else {
        input.to_string()
    };

    context.string(mapped.as_bytes())
}

fn encoded_authority(iri: &AuthorityComponents<'_>, encoded: &AuthorityComponents<'_>) -> String {
    let host = if iri.reg_name().is_some_and(|host| !host.is_ascii()) {
        idna_ascii(iri.host()).unwrap_or_else(|| encoded.host().to_string())
    } else {
        encoded.host().to_string()
    };
    let capacity = encoded.userinfo().map_or(0, |value| value.len() + 1)
        + host.len()
        + encoded.port().map_or(0, |value| value.len() + 1);
    let mut authority = String::with_capacity(capacity);
    if let Some(userinfo) = encoded.userinfo() {
        authority.push_str(userinfo);
        authority.push('@');
    }
    authority.push_str(&host);
    if let Some(port) = encoded.port() {
        authority.push(':');
        authority.push_str(port);
    }

    authority
}

fn decoded_authority(parts: AuthorityComponents<'_>) -> String {
    let userinfo = parts.userinfo().map(decode_non_ascii);
    let host = if parts.reg_name().is_some() {
        idna_unicode(parts.host()).unwrap_or_else(|| decode_non_ascii(parts.host()))
    } else {
        parts.host().to_string()
    };
    let capacity = userinfo.as_ref().map_or(0, |value| value.len() + 1)
        + host.len()
        + parts.port().map_or(0, |value| value.len() + 1);
    let mut authority = String::with_capacity(capacity);
    if let Some(userinfo) = userinfo {
        authority.push_str(&userinfo);
        authority.push('@');
    }
    authority.push_str(&host);
    if let Some(port) = parts.port() {
        authority.push(':');
        authority.push_str(port);
    }

    authority
}

pub(crate) fn idna_ascii(host: &str) -> Option<String> {
    let host = Uts46::new()
        .to_ascii(
            host.as_bytes(),
            AsciiDenyList::STD3,
            Hyphens::Check,
            DnsLength::Verify,
        )
        .ok()?;
    Some(host.into_owned())
}

fn idna_unicode(host: &str) -> Option<String> {
    if !host.split('.').any(|label| {
        label
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("xn--"))
    }) {
        return None;
    }
    idna_ascii(host)?;
    let (unicode, result) =
        Uts46::new().to_unicode(host.as_bytes(), AsciiDenyList::STD3, Hyphens::Check);
    result.ok()?;
    Some(unicode.into_owned())
}

fn decode_non_ascii(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        let Some(first) = encoded_byte(bytes, index) else {
            result.push(char::from(bytes[index]));
            index += 1;
            continue;
        };
        let width = utf8_width(first);
        if width < 2 {
            result.push_str(&input[index..index + 3]);
            index += 3;
            continue;
        }
        let mut sequence = [0_u8; 4];
        sequence[0] = first;
        let mut valid = true;
        for (offset, slot) in sequence.iter_mut().enumerate().take(width).skip(1) {
            let position = index + offset * 3;
            let Some(byte) = encoded_byte(bytes, position) else {
                valid = false;
                break;
            };
            if byte & 0xc0 != 0x80 {
                valid = false;
                break;
            }
            *slot = byte;
        }
        if valid && let Ok(decoded) = from_utf8(&sequence[..width]) {
            result.push_str(decoded);
            index += width * 3;
            continue;
        }

        result.push_str(&input[index..index + 3]);
        index += 3;
    }

    result
}

fn encoded_byte(bytes: &[u8], index: usize) -> Option<u8> {
    if bytes.get(index) != Some(&b'%') {
        return None;
    }
    let high = hex(*bytes.get(index + 1)?)?;
    let low = hex(*bytes.get(index + 2)?)?;
    Some(high << 4 | low)
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

const fn utf8_width(byte: u8) -> usize {
    match byte {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    }
}
