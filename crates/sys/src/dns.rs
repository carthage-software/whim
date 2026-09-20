use std::collections::HashSet;
use std::net::{IpAddr, ToSocketAddrs};

use crate::{Error, Result, constants};

#[derive(Clone, Copy)]
pub enum Family {
    Any,
    Ipv4,
    Ipv6,
}

impl Family {
    pub fn from_raw(value: i64) -> Result<Self> {
        if value == 0 {
            Ok(Self::Any)
        } else if value == i64::from(constants::AF_INET) {
            Ok(Self::Ipv4)
        } else if value == i64::from(constants::AF_INET6) {
            Ok(Self::Ipv6)
        } else {
            Err(Error::invalid("getaddrinfo"))
        }
    }
}

pub fn resolve(host: &str, family: Family) -> Result<Vec<IpAddr>> {
    let mut seen = HashSet::new();
    (host, 0)
        .to_socket_addrs()
        .map(|addresses| {
            addresses
                .map(|address| address.ip())
                .filter(|address| {
                    matches!(
                        (family, address),
                        (Family::Any, _)
                            | (Family::Ipv4, IpAddr::V4(_))
                            | (Family::Ipv6, IpAddr::V6(_))
                    )
                })
                .filter(|address| seen.insert(*address))
                .collect()
        })
        .map_err(|error| Error::new("getaddrinfo", error))
}
