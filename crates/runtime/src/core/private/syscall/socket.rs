//! Non-blocking internet and Unix-domain socket primitives.

use std::ffi::c_void;
use std::mem::size_of;
use std::mem::zeroed;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::os::fd::AsRawFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use std::os::fd::RawFd;
use std::ptr::copy_nonoverlapping;
use std::ptr::from_ref;
use std::str::from_utf8;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::Descriptor;
use crate::core::private::syscall::build_file_descriptor;
use crate::core::private::syscall::descriptor_of;
use crate::core::private::syscall::last_errno;
use crate::core::private::syscall::last_system_error;
use crate::core::private::syscall::set_close_on_exec;
use crate::core::private::syscall::set_non_blocking;
use crate::core::private::syscall::system_error;
use crate::unwrap_result_invariant;
use crate::value::Value;

pub(crate) struct Address {
    pub(crate) storage: libc::sockaddr_storage,
    pub(crate) length: libc::socklen_t,
}

fn socket_length<T>() -> libc::socklen_t {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            libc::socklen_t::try_from(size_of::<T>()),
            "socket structure sizes fit socklen_t",
        )
    }
}

pub(crate) fn socket_family(fd: RawFd) -> Result<i32, i32> {
    // SAFETY: zero is valid for this C output type.
    let mut storage = unsafe { zeroed::<libc::sockaddr_storage>() };
    let mut length = socket_length::<libc::sockaddr_storage>();
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::getsockname(fd, (&raw mut storage).cast(), &raw mut length) } < 0 {
        return Err(last_errno());
    }
    Ok(i32::from(storage.ss_family))
}

pub(crate) fn address(family: i32, host: &[u8], port: i64) -> Result<Address, i32> {
    let port = u16::try_from(port).map_err(|_| libc::EINVAL)?;
    // SAFETY: zero is valid for this C output type.
    let mut storage = unsafe { zeroed::<libc::sockaddr_storage>() };
    match family {
        libc::AF_INET => {
            let host = from_utf8(host)
                .map_err(|_| libc::EINVAL)?
                .parse::<Ipv4Addr>()
                .map_err(|_| libc::EINVAL)?;
            let family = libc::sa_family_t::try_from(libc::AF_INET).map_err(|_| libc::EINVAL)?;
            let target = (&raw mut storage).cast::<libc::sockaddr_in>();
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            unsafe {
                (*target).sin_family = family;
                (*target).sin_port = port.to_be();
                (*target).sin_addr.s_addr = u32::from_ne_bytes(host.octets());
                #[cfg(target_os = "macos")]
                {
                    (*target).sin_len =
                        u8::try_from(size_of::<libc::sockaddr_in>()).map_err(|_| libc::EINVAL)?;
                }
            }
            Ok(Address {
                storage,
                length: socket_length::<libc::sockaddr_in>(),
            })
        }
        libc::AF_INET6 => {
            let host = from_utf8(host)
                .map_err(|_| libc::EINVAL)?
                .parse::<Ipv6Addr>()
                .map_err(|_| libc::EINVAL)?;
            let family = libc::sa_family_t::try_from(libc::AF_INET6).map_err(|_| libc::EINVAL)?;
            let target = (&raw mut storage).cast::<libc::sockaddr_in6>();
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            unsafe {
                (*target).sin6_family = family;
                (*target).sin6_port = port.to_be();
                (*target).sin6_addr.s6_addr = host.octets();
                #[cfg(target_os = "macos")]
                {
                    (*target).sin6_len =
                        u8::try_from(size_of::<libc::sockaddr_in6>()).map_err(|_| libc::EINVAL)?;
                }
            }
            Ok(Address {
                storage,
                length: socket_length::<libc::sockaddr_in6>(),
            })
        }
        libc::AF_UNIX => {
            if host.is_empty() || host.contains(&0) {
                return Err(libc::EINVAL);
            }
            let family = libc::sa_family_t::try_from(libc::AF_UNIX).map_err(|_| libc::EINVAL)?;
            let target = (&raw mut storage).cast::<libc::sockaddr_un>();
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let maximum = unsafe { (*target).sun_path.len() };
            if host.len() >= maximum {
                return Err(libc::ENAMETOOLONG);
            }
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            unsafe {
                (*target).sun_family = family;
                copy_nonoverlapping(
                    host.as_ptr(),
                    (*target).sun_path.as_mut_ptr().cast::<u8>(),
                    host.len(),
                );
            }
            let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + host.len() + 1;
            #[cfg(target_os = "macos")]
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            unsafe {
                (*target).sun_len = u8::try_from(length).map_err(|_| libc::EINVAL)?;
            }
            Ok(Address {
                storage,
                length: libc::socklen_t::try_from(length).map_err(|_| libc::EINVAL)?,
            })
        }
        _ => Err(libc::EAFNOSUPPORT),
    }
}

pub(crate) fn decoded_address(storage: &libc::sockaddr_storage) -> Result<(Vec<u8>, i64), i32> {
    match i32::from(storage.ss_family) {
        libc::AF_INET => {
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let address = unsafe { &*from_ref(storage).cast::<libc::sockaddr_in>() };
            let host = Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes())
                .to_string()
                .into_bytes();
            Ok((host, i64::from(u16::from_be(address.sin_port))))
        }
        libc::AF_INET6 => {
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let address = unsafe { &*from_ref(storage).cast::<libc::sockaddr_in6>() };
            let host = Ipv6Addr::from(address.sin6_addr.s6_addr)
                .to_string()
                .into_bytes();
            Ok((host, i64::from(u16::from_be(address.sin6_port))))
        }
        libc::AF_UNIX => {
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let address = unsafe { &*from_ref(storage).cast::<libc::sockaddr_un>() };
            let length = address
                .sun_path
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(address.sun_path.len());
            let host = address.sun_path[..length]
                .iter()
                .map(|byte| byte.to_ne_bytes()[0])
                .collect();
            Ok((host, 0))
        }
        _ => Err(libc::EAFNOSUPPORT),
    }
}

pub(crate) fn socket_address_raw(fd: RawFd, peer: bool) -> Result<(Vec<u8>, i64), i32> {
    // SAFETY: zero is valid for this C output type.
    let mut storage = unsafe { zeroed::<libc::sockaddr_storage>() };
    let mut length = socket_length::<libc::sockaddr_storage>();
    let result = if peer {
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        unsafe { libc::getpeername(fd, (&raw mut storage).cast(), &raw mut length) }
    } else {
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        unsafe { libc::getsockname(fd, (&raw mut storage).cast(), &raw mut length) }
    };
    if result < 0 {
        return Err(last_errno());
    }
    decoded_address(&storage)
}

#[whim_function("Whim\\_Private\\create_socket(int $family, int $kind): Whim\\OS\\FileDescriptor")]
pub(crate) fn create_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let family =
        i32::try_from(arguments.int(0)).map_err(|_| system_error(cx, "socket", libc::EINVAL))?;
    let kind =
        i32::try_from(arguments.int(1)).map_err(|_| system_error(cx, "socket", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let descriptor = unsafe { libc::socket(family, kind, 0) };
    if descriptor < 0 {
        return Err(last_system_error(cx, "socket"));
    }
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    set_close_on_exec(descriptor.as_raw_fd()).map_err(|errno| system_error(cx, "fcntl", errno))?;
    set_non_blocking(descriptor.as_raw_fd(), true)
        .map_err(|errno| system_error(cx, "fcntl", errno))?;
    build_file_descriptor(cx, Descriptor::Raw(descriptor))
}

fn address_argument(
    cx: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    fd: RawFd,
    call: &'static str,
) -> Result<Address, Throw> {
    let family = socket_family(fd).map_err(|errno| system_error(cx, call, errno))?;
    let host = arguments.bytes(1);
    let port = arguments.int(2);
    address(family, host, port).map_err(|errno| system_error(cx, call, errno))
}

#[whim_function(
    "Whim\\_Private\\bind_socket(Whim\\OS\\FileDescriptor $socket, string $host, (0..) $port): void"
)]
pub(crate) fn bind_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "bind")?;
    let address = address_argument(cx, &arguments, fd, "bind")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::bind(fd, (&raw const address.storage).cast(), address.length) } < 0 {
        return Err(last_system_error(cx, "bind"));
    }
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\listen_socket(Whim\\OS\\FileDescriptor $socket, (1..) $backlog): void"
)]
pub(crate) fn listen_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "listen")?;
    let backlog =
        i32::try_from(arguments.int(1)).map_err(|_| system_error(cx, "listen", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::listen(fd, backlog) } < 0 {
        return Err(last_system_error(cx, "listen"));
    }
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\accept_socket(Whim\\OS\\FileDescriptor $socket): null|(Whim\\OS\\FileDescriptor, string, (0..))"
)]
pub(crate) fn accept_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "accept")?;
    // SAFETY: zero is valid for this C output type.
    let mut storage = unsafe { zeroed::<libc::sockaddr_storage>() };
    let mut length = socket_length::<libc::sockaddr_storage>();
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let accepted = unsafe { libc::accept(fd, (&raw mut storage).cast(), &raw mut length) };
    if accepted < 0 {
        let errno = last_errno();
        if errno == libc::EAGAIN || errno == libc::EWOULDBLOCK {
            return Ok(Value::null());
        }
        return Err(system_error(cx, "accept", errno));
    }
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let accepted = unsafe { OwnedFd::from_raw_fd(accepted) };
    set_close_on_exec(accepted.as_raw_fd()).map_err(|errno| system_error(cx, "fcntl", errno))?;
    set_non_blocking(accepted.as_raw_fd(), true)
        .map_err(|errno| system_error(cx, "fcntl", errno))?;
    let (host, port) =
        decoded_address(&storage).map_err(|errno| system_error(cx, "accept", errno))?;
    let accepted = build_file_descriptor(cx, Descriptor::Raw(accepted))?;
    let host = cx.string(&host);
    let port = Value::int(port);
    Ok(cx.tuple([accepted, host, port]))
}

#[whim_function(
    "Whim\\_Private\\connect_socket(Whim\\OS\\FileDescriptor $socket, string $host, (0..) $port): bool"
)]
pub(crate) fn connect_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "connect")?;
    let address = address_argument(cx, &arguments, fd, "connect")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::connect(fd, (&raw const address.storage).cast(), address.length) } == 0 {
        return Ok(Value::bool(true));
    }
    let errno = last_errno();
    if matches!(
        errno,
        libc::EINPROGRESS | libc::EALREADY | libc::EWOULDBLOCK
    ) {
        return Ok(Value::bool(false));
    }
    Err(system_error(cx, "connect", errno))
}

#[whim_function(
    "Whim\\_Private\\complete_socket_connection(Whim\\OS\\FileDescriptor $socket): void"
)]
pub(crate) fn complete_socket_connection<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "getsockopt")?;
    let mut error = 0_i32;
    let mut length = socket_length::<i32>();
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            (&raw mut error).cast(),
            &raw mut length,
        )
    } < 0
    {
        return Err(last_system_error(cx, "getsockopt"));
    }
    if error != 0 {
        return Err(system_error(cx, "connect", error));
    }
    Ok(Value::null())
}

fn address_result(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    peer: bool,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let call = if peer { "getpeername" } else { "getsockname" };
    let fd = descriptor_of(cx, &socket, call)?;
    let (host, port) =
        socket_address_raw(fd, peer).map_err(|errno| system_error(cx, call, errno))?;
    let host = cx.string(&host);
    let port = Value::int(port);
    Ok(cx.tuple([host, port]))
}

#[whim_function(
    "Whim\\_Private\\socket_address(Whim\\OS\\FileDescriptor $socket): (string, (0..))"
)]
pub(crate) fn socket_address<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    address_result(cx, arguments, false)
}

#[whim_function(
    "Whim\\_Private\\set_socket_option(Whim\\OS\\FileDescriptor $socket, int $level, int $option, int $value): void"
)]
pub(crate) fn set_socket_option<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "setsockopt")?;
    let level = i32::try_from(arguments.int(1))
        .map_err(|_| system_error(cx, "setsockopt", libc::EINVAL))?;
    let option = i32::try_from(arguments.int(2))
        .map_err(|_| system_error(cx, "setsockopt", libc::EINVAL))?;
    let value = arguments.int(3);
    let result = if option == libc::SO_LINGER {
        let linger = libc::linger {
            l_onoff: i32::from(value != 0),
            l_linger: i32::try_from(value.max(0))
                .map_err(|_| system_error(cx, "setsockopt", libc::EINVAL))?,
        };
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        unsafe {
            libc::setsockopt(
                fd,
                level,
                option,
                (&raw const linger).cast::<c_void>(),
                socket_length::<libc::linger>(),
            )
        }
    } else {
        let integer =
            i32::try_from(value).map_err(|_| system_error(cx, "setsockopt", libc::EINVAL))?;
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        unsafe {
            libc::setsockopt(
                fd,
                level,
                option,
                (&raw const integer).cast::<c_void>(),
                socket_length::<i32>(),
            )
        }
    };
    if result < 0 {
        return Err(last_system_error(cx, "setsockopt"));
    }
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\send_to(Whim\\OS\\FileDescriptor $socket, string $bytes, string $host, (0..) $port): (0..)"
)]
pub(crate) fn send_to<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "sendto")?;
    let family = socket_family(fd).map_err(|errno| system_error(cx, "sendto", errno))?;
    let bytes = arguments.bytes(1);
    let host = arguments.bytes(2);
    let port = arguments.int(3);
    let address = address(family, host, port).map_err(|errno| system_error(cx, "sendto", errno))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe {
        libc::sendto(
            fd,
            bytes.as_ptr().cast(),
            bytes.len(),
            0,
            (&raw const address.storage).cast(),
            address.length,
        )
    };
    if count < 0 {
        return Err(last_system_error(cx, "sendto"));
    }
    let count = i64::try_from(count).map_err(|_| system_error(cx, "sendto", libc::EOVERFLOW))?;
    Ok(Value::int(count))
}

#[whim_function(
    "Whim\\_Private\\shutdown_socket(Whim\\OS\\FileDescriptor $socket, int $direction): void"
)]
pub(crate) fn shutdown_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "shutdown")?;
    let direction =
        i32::try_from(arguments.int(1)).map_err(|_| system_error(cx, "shutdown", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::shutdown(fd, direction) } < 0 {
        return Err(last_system_error(cx, "shutdown"));
    }
    Ok(Value::null())
}
