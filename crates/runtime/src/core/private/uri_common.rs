use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::str::from_utf8;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

macro_rules! uri_reference_builtins {
    (
        $reference:ty,
        $absolute:ty,
        $parse_reference:literal,
        $parse_authority:literal,
        $valid_components:literal,
        $normalize:literal,
        $resolve:literal $(,)?
    ) => {
        #[whim_macros::whim_function($parse_reference)]
        pub(crate) fn parse_reference(
            context: &Context<'_, '_, '_>,
            arguments: Arguments<'_>,
        ) -> Value {
            let Some(input) = required_utf8(arguments, 0) else {
                return Value::null();
            };
            let Ok(reference) = <$reference>::new(input) else {
                return Value::null();
            };

            context.tuple([
                optional_string(context, reference.scheme_str()),
                optional_string(context, reference.authority_str()),
                context.string(reference.path_str().as_bytes()),
                optional_string(context, reference.query_str()),
                optional_string(context, reference.fragment_str()),
            ])
        }

        #[whim_macros::whim_function($parse_authority)]
        pub(crate) fn parse_authority(
            context: &Context<'_, '_, '_>,
            arguments: Arguments<'_>,
        ) -> Value {
            let Some(authority) = required_utf8(arguments, 0) else {
                return Value::null();
            };
            let mut candidate = String::with_capacity(authority.len() + 2);
            candidate.push_str("//");
            candidate.push_str(authority);
            let Ok(reference) = <$reference>::new(&candidate) else {
                return Value::null();
            };
            if reference.authority_str() != Some(authority)
                || !reference.path_str().is_empty()
                || reference.query_str().is_some()
                || reference.fragment_str().is_some()
            {
                return Value::null();
            }
            let Some(parts) = reference.authority_components() else {
                return Value::null();
            };
            let Some((kind, host)) = host_value(context, parts.host()) else {
                return Value::null();
            };

            context.tuple([
                optional_string(context, parts.userinfo()),
                context.string(kind),
                host,
                optional_string(context, parts.port()),
            ])
        }

        #[whim_macros::whim_function($valid_components)]
        pub(crate) fn valid_components(arguments: Arguments<'_>) -> Value {
            let Ok(scheme) = optional_utf8(arguments, 0) else {
                return Value::bool(false);
            };
            let Ok(authority) = optional_utf8(arguments, 1) else {
                return Value::bool(false);
            };
            let Some(path) = required_utf8(arguments, 2) else {
                return Value::bool(false);
            };
            let Ok(query) = optional_utf8(arguments, 3) else {
                return Value::bool(false);
            };
            let Ok(fragment) = optional_utf8(arguments, 4) else {
                return Value::bool(false);
            };

            let candidate = compose(scheme, authority, path, query, fragment);
            Value::bool(<$reference>::new(&candidate).is_ok_and(|reference| {
                reference.scheme_str() == scheme
                    && reference.authority_str() == authority
                    && reference.path_str() == path
                    && reference.query_str() == query
                    && reference.fragment_str() == fragment
            }))
        }

        #[whim_macros::whim_function($normalize)]
        pub(crate) fn normalize(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
            let Some(input) = required_utf8(arguments, 0) else {
                return Value::null();
            };
            let Ok(reference) = <$reference>::new(input) else {
                return Value::null();
            };
            let mut builder = Builder::from(reference);
            builder.normalize();
            let Ok(normalized) = builder.build::<$reference>() else {
                return Value::null();
            };

            context.string(normalized.to_string().as_bytes())
        }

        #[whim_macros::whim_function($resolve)]
        pub(crate) fn resolve(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
            let Some(base) = required_utf8(arguments, 0) else {
                return Value::null();
            };
            let Some(reference) = required_utf8(arguments, 1) else {
                return Value::null();
            };
            let Ok(base) = <$reference>::new(base) else {
                return Value::null();
            };
            let Ok(base) = <$absolute>::new(
                base.as_str()
                    .split_once('#')
                    .map_or(base.as_str(), |(absolute, _)| absolute),
            ) else {
                return Value::null();
            };
            let Ok(reference) = <$reference>::new(reference) else {
                return Value::null();
            };

            context.string(reference.resolve_against(base).to_string().as_bytes())
        }
    };
}

pub(super) use uri_reference_builtins;

pub(super) fn required_utf8(arguments: Arguments<'_>, index: usize) -> Option<&str> {
    from_utf8(arguments.bytes(index)).ok()
}

pub(super) fn optional_utf8(arguments: Arguments<'_>, index: usize) -> Result<Option<&str>, ()> {
    let Some(value) = arguments.get(index) else {
        return Err(());
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(bytes) = value.as_string_bytes() else {
        return Err(());
    };

    from_utf8(bytes).map(Some).map_err(|_| ())
}

pub(super) fn optional_string(context: &Context<'_, '_, '_>, value: Option<&str>) -> Value {
    value.map_or_else(Value::null, |value| context.string(value.as_bytes()))
}

pub(super) fn host_value(
    context: &Context<'_, '_, '_>,
    host: &str,
) -> Option<(&'static [u8], Value)> {
    if let Some(literal) = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
    {
        if literal.starts_with('v') || literal.starts_with('V') {
            return Some((b"future", context.string(literal.as_bytes())));
        }

        let address = literal.parse::<Ipv6Addr>().ok()?;
        return Some((b"ipv6", context.string(&address.octets())));
    }
    if let Ok(address) = host.parse::<Ipv4Addr>() {
        return Some((b"ipv4", context.string(&address.octets())));
    }

    Some((b"name", context.string(host.as_bytes())))
}

pub(super) fn compose(
    scheme: Option<&str>,
    authority: Option<&str>,
    path: &str,
    query: Option<&str>,
    fragment: Option<&str>,
) -> String {
    let capacity = scheme.map_or(0, |value| value.len() + 1)
        + authority.map_or(0, |value| value.len() + 2)
        + path.len()
        + query.map_or(0, |value| value.len() + 1)
        + fragment.map_or(0, |value| value.len() + 1);
    let mut result = String::with_capacity(capacity);
    if let Some(scheme) = scheme {
        result.push_str(scheme);
        result.push(':');
    }
    if let Some(authority) = authority {
        result.push_str("//");
        result.push_str(authority);
    }
    result.push_str(path);
    if let Some(query) = query {
        result.push('?');
        result.push_str(query);
    }
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }

    result
}
