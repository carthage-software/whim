//! IDNA domain conversion.

use idna::uts46::AsciiDenyList;
use idna::uts46::DnsLength;
use idna::uts46::Hyphens;
use idna::uts46::Uts46;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

#[whim_function("Whim\\_Private\\idna_to_ascii(string $domain): null|string")]
pub(crate) fn to_ascii(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let domain = arguments.bytes(0);
    let Ok(domain) = Uts46::new().to_ascii(
        domain,
        AsciiDenyList::STD3,
        Hyphens::Check,
        DnsLength::Verify,
    ) else {
        return Value::null();
    };

    context.string(domain.as_bytes())
}

#[whim_function("Whim\\_Private\\idna_to_unicode(string $domain): null|string")]
pub(crate) fn to_unicode(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let domain = arguments.bytes(0);
    let converter = Uts46::new();
    if converter
        .to_ascii(
            domain,
            AsciiDenyList::STD3,
            Hyphens::Check,
            DnsLength::Verify,
        )
        .is_err()
    {
        return Value::null();
    }

    let (domain, result) = converter.to_unicode(domain, AsciiDenyList::STD3, Hyphens::Check);
    if result.is_err() {
        return Value::null();
    }

    context.string(domain.as_bytes())
}
