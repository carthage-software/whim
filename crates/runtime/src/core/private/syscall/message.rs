//! Datagram message calls carrying local-address and ECN metadata.

use std::ffi::c_void;
use std::mem::size_of;
use std::mem::zeroed;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::os::fd::RawFd;
use std::ptr::read_unaligned;
use std::ptr::write_unaligned;
use std::str::from_utf8;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::descriptor_of;
use crate::core::private::syscall::last_errno;
use crate::core::private::syscall::socket::address;
use crate::core::private::syscall::socket::decoded_address;
use crate::core::private::syscall::socket::socket_address_raw;
use crate::core::private::syscall::socket::socket_family;
use crate::core::private::syscall::system_error;
use crate::value::Value;

const CONTROL_WORDS: usize = 32;

#[derive(Clone, Copy)]
enum SourceAddress {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

fn abi_length<T: TryFrom<usize>>(length: usize) -> Result<T, i32> {
    T::try_from(length).map_err(|_| libc::EOVERFLOW)
}

fn abi_usize<T: TryInto<usize>>(length: T) -> usize {
    length.try_into().unwrap_or(0)
}

fn set_option(fd: RawFd, level: i32, option: i32) -> Result<(), i32> {
    let enabled = 1_i32;
    let length = abi_length(size_of::<i32>())?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe {
        libc::setsockopt(
            fd,
            level,
            option,
            (&raw const enabled).cast::<c_void>(),
            length,
        )
    } < 0
    {
        return Err(last_errno());
    }
    Ok(())
}

fn enable_metadata(fd: RawFd, family: i32) -> Result<(), i32> {
    match family {
        libc::AF_INET => {
            set_option(fd, libc::IPPROTO_IP, libc::IP_PKTINFO)?;
            set_option(fd, libc::IPPROTO_IP, libc::IP_RECVTOS)
        }
        libc::AF_INET6 => {
            set_option(fd, libc::IPPROTO_IPV6, libc::IPV6_RECVPKTINFO)?;
            set_option(fd, libc::IPPROTO_IPV6, libc::IPV6_RECVTCLASS)
        }
        _ => Err(libc::EAFNOSUPPORT),
    }
}

fn payload_length(header: &libc::cmsghdr) -> usize {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let header_length = unsafe { libc::CMSG_LEN(0) } as usize;
    abi_usize(header.cmsg_len).saturating_sub(header_length)
}

fn read_control<T: Copy>(header: &libc::cmsghdr) -> Option<T> {
    if payload_length(header) < size_of::<T>() {
        return None;
    }
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    Some(unsafe { read_unaligned(libc::CMSG_DATA(header).cast::<T>()) })
}

fn congestion_bits(header: &libc::cmsghdr) -> Option<i64> {
    let value = if payload_length(header) >= size_of::<i32>() {
        i64::from(read_control::<i32>(header)?)
    } else {
        i64::from(read_control::<u8>(header)?)
    };
    Some(value & 0b11)
}

fn local_ipv4(header: &libc::cmsghdr) -> Option<(Vec<u8>, i64)> {
    let information = read_control::<libc::in_pktinfo>(header)?;
    Some((
        Ipv4Addr::from(information.ipi_addr.s_addr.to_ne_bytes())
            .to_string()
            .into_bytes(),
        i64::from(information.ipi_ifindex),
    ))
}

fn local_ipv6(header: &libc::cmsghdr) -> Option<(Vec<u8>, i64)> {
    let information = read_control::<libc::in6_pktinfo>(header)?;
    Some((
        Ipv6Addr::from(information.ipi6_addr.s6_addr)
            .to_string()
            .into_bytes(),
        i64::from(information.ipi6_ifindex),
    ))
}

fn append_control<T: Copy>(
    control: &mut [usize; CONTROL_WORDS],
    used: &mut usize,
    level: i32,
    kind: i32,
    value: T,
) -> Result<(), i32> {
    let payload = u32::try_from(size_of::<T>()).map_err(|_| libc::EOVERFLOW)?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let space = unsafe { libc::CMSG_SPACE(payload) } as usize;
    let end = used.checked_add(space).ok_or(libc::EOVERFLOW)?;
    if end > size_of::<[usize; CONTROL_WORDS]>() {
        return Err(libc::EOVERFLOW);
    }
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let header = unsafe {
        control
            .as_mut_ptr()
            .cast::<u8>()
            .add(*used)
            .cast::<libc::cmsghdr>()
    };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    unsafe {
        (*header).cmsg_len = abi_length(libc::CMSG_LEN(payload) as usize)?;
        (*header).cmsg_level = level;
        (*header).cmsg_type = kind;
        write_unaligned(libc::CMSG_DATA(header).cast::<T>(), value);
    }
    *used = end;
    Ok(())
}

fn append_source_metadata(
    control: &mut [usize; CONTROL_WORDS],
    used: &mut usize,
    source: SourceAddress,
    interface_index: u32,
    explicit_congestion: i32,
) -> Result<(), i32> {
    match source {
        SourceAddress::V4(source) => {
            #[cfg(target_os = "linux")]
            let interface_index = i32::try_from(interface_index).map_err(|_| libc::EOVERFLOW)?;
            let information = libc::in_pktinfo {
                ipi_ifindex: interface_index,
                ipi_spec_dst: libc::in_addr {
                    s_addr: u32::from_ne_bytes(source.octets()),
                },
                ipi_addr: libc::in_addr { s_addr: 0 },
            };
            append_control(
                control,
                used,
                libc::IPPROTO_IP,
                libc::IP_PKTINFO,
                information,
            )?;
            append_control(
                control,
                used,
                libc::IPPROTO_IP,
                libc::IP_TOS,
                explicit_congestion,
            )
        }
        SourceAddress::V6(source) => {
            let information = libc::in6_pktinfo {
                ipi6_addr: libc::in6_addr {
                    s6_addr: source.octets(),
                },
                ipi6_ifindex: interface_index,
            };
            append_control(
                control,
                used,
                libc::IPPROTO_IPV6,
                libc::IPV6_PKTINFO,
                information,
            )?;
            append_control(
                control,
                used,
                libc::IPPROTO_IPV6,
                libc::IPV6_TCLASS,
                explicit_congestion,
            )
        }
    }
}

#[whim_function("Whim\\_Private\\enable_message_metadata(Whim\\OS\\FileDescriptor $socket): void")]
pub(crate) fn enable_message_metadata<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "setsockopt")?;
    let family = socket_family(fd).map_err(|errno| system_error(cx, "getsockname", errno))?;
    enable_metadata(fd, family).map_err(|errno| system_error(cx, "setsockopt", errno))?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\receive_message(Whim\\OS\\FileDescriptor $socket, (1..) $maxBytes, (0..) $localPort): null|(string, (string, (0..)), (string, (0..)), 0..=3, (0..), bool)"
)]
pub(crate) fn receive_message<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "recvmsg")?;
    let maximum =
        usize::try_from(arguments.int(1)).map_err(|_| system_error(cx, "recvmsg", libc::EINVAL))?;
    let local_port = arguments.int(2);

    let mut bytes = vec![0_u8; maximum];
    // SAFETY: zero is valid for this C output type.
    let mut source = unsafe { zeroed::<libc::sockaddr_storage>() };
    let mut control = [0_usize; CONTROL_WORDS];
    let mut vector = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: zero is valid for this C output type.
    let mut message = unsafe { zeroed::<libc::msghdr>() };
    message.msg_name = (&raw mut source).cast();
    message.msg_namelen = abi_length(size_of::<libc::sockaddr_storage>())
        .map_err(|errno| system_error(cx, "recvmsg", errno))?;
    message.msg_iov = &raw mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = abi_length(size_of::<[usize; CONTROL_WORDS]>())
        .map_err(|errno| system_error(cx, "recvmsg", errno))?;

    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::recvmsg(fd, &raw mut message, 0) };
    if count < 0 {
        let errno = last_errno();
        if errno == libc::EAGAIN || errno == libc::EWOULDBLOCK {
            return Ok(Value::null());
        }
        return Err(system_error(cx, "recvmsg", errno));
    }
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(system_error(cx, "recvmsg", libc::EOVERFLOW));
    }

    let mut local = None;
    let mut explicit_congestion = 0_i64;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let mut header = unsafe { libc::CMSG_FIRSTHDR(&raw const message) };
    while !header.is_null() {
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        let control = unsafe { &*header };
        match (control.cmsg_level, control.cmsg_type) {
            (libc::IPPROTO_IP, libc::IP_PKTINFO) => local = local_ipv4(control),
            (libc::IPPROTO_IPV6, libc::IPV6_PKTINFO) => local = local_ipv6(control),
            (libc::IPPROTO_IP, libc::IP_TOS | libc::IP_RECVTOS)
            | (libc::IPPROTO_IPV6, libc::IPV6_TCLASS | libc::IPV6_RECVTCLASS) => {
                explicit_congestion = congestion_bits(control).unwrap_or(0);
            }
            _ => {}
        }
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        header = unsafe { libc::CMSG_NXTHDR(&raw const message, header) };
    }

    let (local_host, interface_index) = if let Some(local) = local {
        local
    } else {
        let host = socket_address_raw(fd, false)
            .map_err(|errno| system_error(cx, "getsockname", errno))?
            .0;
        (host, 0)
    };
    let (peer_host, peer_port) =
        decoded_address(&source).map_err(|errno| system_error(cx, "recvmsg", errno))?;
    let bytes = cx.string(&bytes[..count.cast_unsigned()]);
    let peer_host = cx.string(&peer_host);
    let peer_port = Value::int(peer_port);
    let peer = cx.tuple([peer_host, peer_port]);
    let local_host = cx.string(&local_host);
    let local_port = Value::int(local_port);
    let local = cx.tuple([local_host, local_port]);
    let explicit_congestion = Value::int(explicit_congestion);
    let interface_index = Value::int(interface_index);
    let truncated = Value::bool(message.msg_flags & libc::MSG_TRUNC != 0);
    Ok(cx.tuple([
        bytes,
        peer,
        local,
        explicit_congestion,
        interface_index,
        truncated,
    ]))
}

#[whim_function(
    "Whim\\_Private\\send_message(Whim\\OS\\FileDescriptor $socket, string $bytes, null|string $host, (0..) $port, string $sourceHost, (0..) $interfaceIndex, 0..=3 $explicitCongestion): (0..)"
)]
pub(crate) fn send_message<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let socket = arguments.local(0);
    let fd = descriptor_of(cx, &socket, "sendmsg")?;
    let bytes = arguments.bytes(1);
    let source_host = arguments.bytes(4);
    let source_text =
        from_utf8(source_host).map_err(|_| system_error(cx, "sendmsg", libc::EINVAL))?;
    let source = if let Ok(source) = source_text.parse::<Ipv4Addr>() {
        SourceAddress::V4(source)
    } else if let Ok(source) = source_text.parse::<Ipv6Addr>() {
        SourceAddress::V6(source)
    } else {
        return Err(system_error(cx, "sendmsg", libc::EINVAL));
    };
    let family = match source {
        SourceAddress::V4(_) => libc::AF_INET,
        SourceAddress::V6(_) => libc::AF_INET6,
    };
    let host = arguments.local(2);
    let destination = if host.is_null() {
        None
    } else {
        let host = arguments.bytes(2);
        let port = arguments.int(3);
        Some(address(family, host, port).map_err(|errno| system_error(cx, "sendmsg", errno))?)
    };
    let interface_index =
        u32::try_from(arguments.int(5)).map_err(|_| system_error(cx, "sendmsg", libc::EINVAL))?;
    let explicit_congestion =
        i32::try_from(arguments.int(6)).map_err(|_| system_error(cx, "sendmsg", libc::EINVAL))?;
    let mut control = [0_usize; CONTROL_WORDS];
    let mut control_length = 0;
    append_source_metadata(
        &mut control,
        &mut control_length,
        source,
        interface_index,
        explicit_congestion,
    )
    .map_err(|errno| system_error(cx, "sendmsg", errno))?;

    let mut vector = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: zero is valid for this C output type.
    let mut message = unsafe { zeroed::<libc::msghdr>() };
    if let Some(destination) = &destination {
        message.msg_name = (&raw const destination.storage).cast_mut().cast();
        message.msg_namelen = destination.length;
    }
    message.msg_iov = &raw mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen =
        abi_length(control_length).map_err(|errno| system_error(cx, "sendmsg", errno))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::sendmsg(fd, &raw const message, 0) };
    if count < 0 {
        return Err(system_error(cx, "sendmsg", last_errno()));
    }
    Ok(Value::int(count as i64))
}
