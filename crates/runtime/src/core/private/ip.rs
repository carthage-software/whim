//! IP address text and byte conversion.

use std::fmt;
use std::fmt::Write;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::str::from_utf8;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::unreachable_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;

const MAX_ADDRESS_LENGTH: usize = 39;

struct AddressBuffer {
    bytes: [u8; MAX_ADDRESS_LENGTH],
    length: usize,
}

impl AddressBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; MAX_ADDRESS_LENGTH],
            length: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

impl Write for AddressBuffer {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let Some(end) = self.length.checked_add(value.len()) else {
            return Err(fmt::Error);
        };

        let Some(destination) = self.bytes.get_mut(self.length..end) else {
            return Err(fmt::Error);
        };

        destination.copy_from_slice(value.as_bytes());
        self.length = end;

        Ok(())
    }
}

#[whim_function("Whim\\_Private\\ip_parse(string $address): null|string[4]|string[16]")]
pub(crate) fn parse(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let address = arguments.bytes(0);
    let Ok(address) = from_utf8(address) else {
        return Value::null();
    };

    let Ok(address) = address.parse::<IpAddr>() else {
        return Value::null();
    };

    match address {
        IpAddr::V4(address) => context.string(&address.octets()),
        IpAddr::V6(address) => context.string(&address.octets()),
    }
}

#[whim_function("Whim\\_Private\\ip_format(string[4]|string[16] $bytes): string[2..=39]")]
pub(crate) fn format(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let address = match bytes.len() {
        4 => IpAddr::V4(Ipv4Addr::from(fixed(bytes))),
        16 => IpAddr::V6(Ipv6Addr::from(fixed(bytes))),
        // SAFETY: the argument type permits only four or 16 bytes.
        _ => unsafe { unreachable_invariant("an IP address has a valid byte length") },
    };

    let mut formatted = AddressBuffer::new();
    if write!(formatted, "{address}").is_err() {
        // SAFETY: the buffer holds the longest possible IP address text.
        unsafe { unreachable_invariant("an IP address fits its formatting buffer") }
    }

    context.string(formatted.as_bytes())
}

fn fixed<const N: usize>(bytes: &[u8]) -> [u8; N] {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            bytes.try_into(),
            "the address length matches its fixed-width representation",
        )
    }
}
