use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;

use socket2::{Domain, SockAddr, Socket};

use crate::platform::descriptor::close_on_exec;
use crate::socket::Address;
use crate::{Descriptor, Error, Result};

pub(crate) fn family(socket: &Socket) -> io::Result<i32> {
    socket
        .local_addr()
        .map(|address| i32::from(address.family()))
}

pub(crate) fn validate_kind(_family: i32, _kind: i32) -> Result<()> {
    Ok(())
}

pub(crate) fn prepare(socket: &Socket) -> io::Result<()> {
    close_on_exec(socket.as_raw_fd()).map_err(|error| match error {
        Error::Io { source, .. } => source,
        error => io::Error::other(error),
    })?;
    socket.set_nonblocking(true)
}

pub(crate) fn decoded_address(address: &SockAddr) -> io::Result<Address> {
    if let Some(address) = address.as_socket() {
        return Ok(Address {
            host: address.ip().to_string().into_bytes(),
            port: address.port(),
        });
    }
    if Domain::from(i32::from(address.family())) == Domain::UNIX {
        return Ok(Address {
            host: address
                .as_pathname()
                .map_or_else(Vec::new, |path| path.as_os_str().as_bytes().to_vec()),
            port: 0,
        });
    }
    Err(io::Error::from_raw_os_error(libc::EAFNOSUPPORT))
}

pub(crate) fn set_option_raw(
    descriptor: &Descriptor,
    level: i32,
    option: i32,
    value: i64,
) -> Result<()> {
    let result = if option == libc::SO_LINGER {
        let linger = libc::linger {
            l_onoff: i32::from(value != 0),
            l_linger: i32::try_from(value.max(0)).map_err(|_| Error::invalid("setsockopt"))?,
        };
        // SAFETY: descriptor is live and linger has the supplied size.
        unsafe {
            libc::setsockopt(
                descriptor.raw(),
                level,
                option,
                (&raw const linger).cast(),
                size_of::<libc::linger>() as libc::socklen_t,
            )
        }
    } else {
        let value = i32::try_from(value).map_err(|_| Error::invalid("setsockopt"))?;
        // SAFETY: descriptor is live and value has the supplied size.
        unsafe {
            libc::setsockopt(
                descriptor.raw(),
                level,
                option,
                (&raw const value).cast(),
                size_of::<i32>() as libc::socklen_t,
            )
        }
    };
    if result < 0 {
        Err(Error::last("setsockopt"))
    } else {
        Ok(())
    }
}
