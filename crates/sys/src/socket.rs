use std::io;
use std::net::{IpAddr, Shutdown, SocketAddr};
use std::str::from_utf8;

use socket2::{Domain, SockAddr, Socket, Type};

use crate::error::errno;
use crate::path::path_from_bytes;
use crate::platform::socket::{family, prepare, set_option_raw, validate_kind};
use crate::{Descriptor, Error, Result};

pub(crate) use crate::platform::socket::decoded_address;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub host: Vec<u8>,
    pub port: u16,
}

pub(crate) fn address(family: i32, host: &[u8], port: i64) -> io::Result<SockAddr> {
    let port = u16::try_from(port).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    if Domain::from(family) == Domain::UNIX {
        if host.is_empty() || host.contains(&0) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        return SockAddr::unix(path_from_bytes(host)?);
    }
    let host = from_utf8(host)
        .ok()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .ok_or(io::ErrorKind::InvalidInput)?;
    if (Domain::from(family) == Domain::IPV4 && host.is_ipv4())
        || (Domain::from(family) == Domain::IPV6 && host.is_ipv6())
    {
        Ok(SocketAddr::new(host, port).into())
    } else {
        Err(io::ErrorKind::InvalidInput.into())
    }
}

pub fn create(family: i64, kind: i64) -> Result<Descriptor> {
    let family = i32::try_from(family).map_err(|_| Error::invalid("socket"))?;
    let kind = i32::try_from(kind).map_err(|_| Error::invalid("socket"))?;
    validate_kind(family, kind)?;
    let socket = Socket::new(Domain::from(family), Type::from(kind), None)
        .map_err(|error| Error::new("socket", error))?;
    prepare(&socket).map_err(|error| Error::new("socket", error))?;
    Ok(Descriptor::from_socket(socket))
}

pub fn bind(descriptor: &Descriptor, host: &[u8], port: i64) -> Result<()> {
    descriptor
        .with_socket(|socket| socket.bind(&address(family(socket)?, host, port)?))
        .map_err(|error| Error::new("bind", error))
}

pub fn listen(descriptor: &Descriptor, backlog: i64) -> Result<()> {
    let backlog = i32::try_from(backlog).map_err(|_| Error::invalid("listen"))?;
    descriptor
        .with_socket(|socket| socket.listen(backlog))
        .map_err(|error| Error::new("listen", error))
}

pub fn accept(descriptor: &Descriptor) -> Result<Option<(Descriptor, Address)>> {
    descriptor
        .with_socket(|socket| match socket.accept() {
            Ok((socket, address)) => {
                prepare(&socket)?;
                Ok(Some((
                    Descriptor::from_socket(socket),
                    decoded_address(&address)?,
                )))
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        })
        .map_err(|error| Error::new("accept", error))
}

pub fn connect(descriptor: &Descriptor, host: &[u8], port: i64) -> Result<bool> {
    descriptor
        .with_socket(
            |socket| match socket.connect(&address(family(socket)?, host, port)?) {
                Ok(()) => Ok(true),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || matches!(errno(&error), libc::EINPROGRESS | libc::EALREADY) =>
                {
                    Ok(false)
                }
                Err(error) => Err(error),
            },
        )
        .map_err(|error| Error::new("connect", error))
}

pub fn complete_connection(descriptor: &Descriptor) -> Result<()> {
    descriptor
        .with_socket(|socket| socket.take_error()?.map_or(Ok(()), Err))
        .map_err(|error| Error::new("connect", error))
}

pub fn local_address(descriptor: &Descriptor) -> Result<Address> {
    descriptor
        .with_socket(|socket| decoded_address(&socket.local_addr()?))
        .map_err(|error| Error::new("getsockname", error))
}

pub fn peer_address(descriptor: &Descriptor) -> Result<Address> {
    descriptor
        .with_socket(|socket| decoded_address(&socket.peer_addr()?))
        .map_err(|error| Error::new("getpeername", error))
}

pub fn set_option(descriptor: &Descriptor, level: i64, option: i64, value: i64) -> Result<()> {
    let level = i32::try_from(level).map_err(|_| Error::invalid("setsockopt"))?;
    let option = i32::try_from(option).map_err(|_| Error::invalid("setsockopt"))?;
    set_option_raw(descriptor, level, option, value)
}

pub fn send_to(descriptor: &Descriptor, bytes: &[u8], host: &[u8], port: i64) -> Result<usize> {
    descriptor
        .with_socket(|socket| socket.send_to(bytes, &address(family(socket)?, host, port)?))
        .map_err(|error| Error::new("sendto", error))
}

pub fn shutdown(descriptor: &Descriptor, direction: i64) -> Result<()> {
    let direction = match direction {
        0 => Shutdown::Read,
        1 => Shutdown::Write,
        2 => Shutdown::Both,
        _ => return Err(Error::invalid("shutdown")),
    };
    descriptor
        .with_socket(|socket| socket.shutdown(direction))
        .map_err(|error| Error::new("shutdown", error))
}
