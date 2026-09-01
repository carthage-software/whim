//! URI parsing, normalization, and resolution.

use iri_string::build::Builder;
use iri_string::types::UriAbsoluteStr;
use iri_string::types::UriReferenceStr;

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
    UriReferenceStr,
    UriAbsoluteStr,
    "Whim\\_Private\\uri_parse_reference(string $uri): null|(null|string, null|string, string, null|string, null|string)",
    "Whim\\_Private\\uri_parse_authority(string $authority): null|(null|string, 'future'|'name'|'ipv4'|'ipv6', string, null|string)",
    "Whim\\_Private\\uri_valid_components(null|string $scheme, null|string $authority, string $path, null|string $query, null|string $fragment): bool",
    "Whim\\_Private\\uri_normalize(string $uri): null|string",
    "Whim\\_Private\\uri_resolve(string $base, string $reference): null|string",
);
