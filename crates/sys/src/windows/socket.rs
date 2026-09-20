use std::io;
use std::os::windows::io::AsRawSocket;
use std::time::Duration;

use socket2::{SockAddr, Socket};
use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
use windows_sys::Win32::Networking::WinSock as ws;

use crate::socket::Address;
use crate::{Descriptor, Error, Result};

pub(crate) fn last_error() -> io::Error {
    // SAFETY: WSAGetLastError reads the calling thread's error code.
    io::Error::from_raw_os_error(unsafe { ws::WSAGetLastError() })
}

pub(crate) fn family(socket: &Socket) -> io::Result<i32> {
    let mut info = ws::WSAPROTOCOL_INFOW::default();
    let mut length = size_of_val(&info) as i32;
    // SAFETY: info and length are writable outputs of the supplied size.
    if unsafe {
        ws::getsockopt(
            socket.as_raw_socket() as usize,
            ws::SOL_SOCKET,
            ws::SO_PROTOCOL_INFOW,
            (&raw mut info).cast(),
            &raw mut length,
        )
    } != 0
    {
        return Err(last_error());
    }
    Ok(info.iAddressFamily)
}

pub(crate) fn validate_kind(family: i32, kind: i32) -> Result<()> {
    if family == i32::from(ws::AF_UNIX) && kind != ws::SOCK_STREAM {
        Err(Error::Unsupported("Unix datagram sockets"))
    } else {
        Ok(())
    }
}

pub(crate) fn prepare(socket: &Socket) -> io::Result<()> {
    socket.set_nonblocking(true)?;
    // SAFETY: socket is live; this clears only its inheritance flag.
    if unsafe {
        SetHandleInformation(
            socket.as_raw_socket() as usize as *mut _,
            HANDLE_FLAG_INHERIT,
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn decoded_address(address: &SockAddr) -> io::Result<Address> {
    if let Some(address) = address.as_socket() {
        return Ok(Address {
            host: address.ip().to_string().into_bytes(),
            port: address.port(),
        });
    }
    if address.family() == ws::AF_UNIX {
        // SAFETY: the family identifies SOCKADDR_UN; SockAddr owns the full zeroed buffer.
        let address = unsafe { &*address.as_ptr().cast::<ws::SOCKADDR_UN>() };
        return Ok(Address {
            host: address
                .sun_path
                .iter()
                .take_while(|byte| **byte != 0)
                .map(|byte| byte.cast_unsigned())
                .collect(),
            port: 0,
        });
    }
    Err(io::Error::from_raw_os_error(ws::WSAEAFNOSUPPORT))
}

pub(crate) fn set_option_raw(
    descriptor: &Descriptor,
    level: i32,
    option: i32,
    value: i64,
) -> Result<()> {
    if option == -1 {
        return if value == 0 {
            Ok(())
        } else {
            Err(Error::Unsupported("SO_REUSEPORT"))
        };
    }
    descriptor
        .with_socket(|socket| {
            if option == ws::SO_LINGER && level == ws::SOL_SOCKET {
                return socket.set_linger(
                    (value != 0).then(|| Duration::from_secs(value.max(0).cast_unsigned())),
                );
            }
            let integer =
                i32::try_from(value).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            // SAFETY: socket is live and integer has the supplied size.
            if unsafe {
                ws::setsockopt(
                    socket.as_raw_socket() as usize,
                    level,
                    option,
                    (&raw const integer).cast(),
                    size_of::<i32>() as i32,
                )
            } != 0
            {
                Err(last_error())
            } else {
                Ok(())
            }
        })
        .map_err(|error| Error::new("setsockopt", error))
}
