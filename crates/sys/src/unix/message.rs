//! Datagram message calls carrying local-address and ECN metadata.

use crate::platform::socket::family;
#[cfg(target_os = "freebsd")]
use crate::socket::bind;
use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::mem::zeroed;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::os::fd::RawFd;
use std::ptr::from_ref;
use std::ptr::read_unaligned;
use std::ptr::write_unaligned;
use std::str::from_utf8;

use crate::message::Message;
use crate::socket::{Address, address, local_address};
use crate::{Descriptor, Error};

fn last_errno() -> i32 {
    io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

fn error(call: &'static str, errno: i32) -> Error {
    Error::new(call, io::Error::from_raw_os_error(errno))
}

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

fn enable_raw(fd: RawFd, family: i32) -> Result<(), i32> {
    match family {
        libc::AF_INET => {
            #[cfg(not(target_os = "freebsd"))]
            set_option(fd, libc::IPPROTO_IP, libc::IP_PKTINFO)?;
            #[cfg(target_os = "freebsd")]
            {
                set_option(fd, libc::IPPROTO_IP, libc::IP_RECVDSTADDR)?;
                set_option(fd, libc::IPPROTO_IP, libc::IP_RECVIF)?;
            }
            set_option(fd, libc::IPPROTO_IP, libc::IP_RECVTOS)
        }
        libc::AF_INET6 => {
            #[cfg(target_os = "linux")]
            set_option(fd, libc::IPPROTO_IP, libc::IP_RECVTOS)?;
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

#[cfg(not(target_os = "freebsd"))]
fn local_ipv4(header: &libc::cmsghdr) -> Option<(Vec<u8>, i64)> {
    let information = read_control::<libc::in_pktinfo>(header)?;
    Some((
        Ipv4Addr::from(information.ipi_addr.s_addr.to_ne_bytes())
            .to_string()
            .into_bytes(),
        i64::from(information.ipi_ifindex),
    ))
}

#[cfg(target_os = "freebsd")]
fn local_ipv4(header: &libc::cmsghdr) -> Option<(Vec<u8>, i64)> {
    let address = read_control::<libc::in_addr>(header)?;
    Some((
        Ipv4Addr::from(address.s_addr.to_ne_bytes())
            .to_string()
            .into_bytes(),
        0,
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
    #[cfg(target_os = "freebsd")] descriptor: &Descriptor,
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
            #[cfg(not(target_os = "freebsd"))]
            let information = libc::in_pktinfo {
                ipi_ifindex: interface_index,
                ipi_spec_dst: libc::in_addr {
                    s_addr: u32::from_ne_bytes(source.octets()),
                },
                ipi_addr: libc::in_addr { s_addr: 0 },
            };
            #[cfg(not(target_os = "freebsd"))]
            append_control(
                control,
                used,
                libc::IPPROTO_IP,
                libc::IP_PKTINFO,
                information,
            )?;
            #[cfg(target_os = "freebsd")]
            {
                if interface_index != 0 {
                    return Err(libc::EOPNOTSUPP);
                }
                if !source.is_unspecified() {
                    let local = local_address(descriptor).map_err(|error| error.errno())?;
                    if local.host != source.to_string().as_bytes() {
                        if local.port == 0 {
                            bind(descriptor, b"0.0.0.0", 0).map_err(|error| error.errno())?;
                        }

                        append_control(
                            control,
                            used,
                            libc::IPPROTO_IP,
                            libc::IP_SENDSRCADDR,
                            libc::in_addr {
                                s_addr: u32::from_ne_bytes(source.octets()),
                            },
                        )?;
                    }
                }
            }
            #[cfg(target_os = "freebsd")]
            let explicit_congestion =
                u8::try_from(explicit_congestion).map_err(|_| libc::EINVAL)?;
            append_control(
                control,
                used,
                libc::IPPROTO_IP,
                libc::IP_TOS,
                explicit_congestion,
            )
        }
        SourceAddress::V6(source) => {
            #[cfg(target_os = "freebsd")]
            if source.to_ipv4_mapped().is_some()
                && interface_index == 0
                && local_address(descriptor)
                    .map_err(|error| error.errno())?
                    .host
                    == source.to_string().as_bytes()
            {
                return append_control(
                    control,
                    used,
                    libc::IPPROTO_IPV6,
                    libc::IPV6_TCLASS,
                    explicit_congestion,
                );
            }
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
            #[cfg(target_os = "linux")]
            if source.to_ipv4_mapped().is_some() {
                return append_control(
                    control,
                    used,
                    libc::IPPROTO_IP,
                    libc::IP_TOS,
                    explicit_congestion,
                );
            }
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

pub fn enable_metadata(descriptor: &Descriptor) -> crate::Result<()> {
    let family = descriptor
        .with_socket(family)
        .map_err(|error| Error::new("getsockname", error))?;
    enable_raw(descriptor.raw(), family).map_err(|errno| error("setsockopt", errno))
}

pub fn receive(
    descriptor: &Descriptor,
    maximum: usize,
    local_port: u16,
) -> crate::Result<Option<Message>> {
    let fd = descriptor.raw();

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
    message.msg_namelen =
        abi_length(size_of::<libc::sockaddr_storage>()).map_err(|errno| error("recvmsg", errno))?;
    message.msg_iov = &raw mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen =
        abi_length(size_of::<[usize; CONTROL_WORDS]>()).map_err(|errno| error("recvmsg", errno))?;

    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::recvmsg(fd, &raw mut message, 0) };
    if count < 0 {
        let errno = last_errno();
        if errno == libc::EAGAIN || errno == libc::EWOULDBLOCK {
            return Ok(None);
        }
        return Err(error("recvmsg", errno));
    }
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(error("recvmsg", libc::EOVERFLOW));
    }

    let mut local = None;
    #[cfg(target_os = "freebsd")]
    let mut ipv4_interface_index = 0_i64;
    let mut explicit_congestion = 0_i64;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let mut header = unsafe { libc::CMSG_FIRSTHDR(&raw const message) };
    while !header.is_null() {
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        let control = unsafe { &*header };
        match (control.cmsg_level, control.cmsg_type) {
            #[cfg(not(target_os = "freebsd"))]
            (libc::IPPROTO_IP, libc::IP_PKTINFO) => local = local_ipv4(control),
            #[cfg(target_os = "freebsd")]
            (libc::IPPROTO_IP, libc::IP_RECVDSTADDR) => local = local_ipv4(control),
            #[cfg(target_os = "freebsd")]
            (libc::IPPROTO_IP, libc::IP_RECVIF) => {
                const OFFSET: usize = std::mem::offset_of!(libc::sockaddr_dl, sdl_index);
                if let Some(bytes) = read_control::<[u8; OFFSET + size_of::<u16>()]>(control) {
                    ipv4_interface_index =
                        i64::from(u16::from_ne_bytes([bytes[OFFSET], bytes[OFFSET + 1]]));
                }
            }
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
        let host = local_address(descriptor)?.host;
        (host, 0)
    };
    #[cfg(target_os = "freebsd")]
    let interface_index = interface_index.max(ipv4_interface_index);
    let peer = decoded_address(&source)?;
    bytes.truncate(count.cast_unsigned());
    Ok(Some(Message {
        bytes,
        peer,
        local: Address {
            host: local_host,
            port: local_port,
        },
        congestion: explicit_congestion as u8,
        interface: interface_index as u32,
        truncated: message.msg_flags & libc::MSG_TRUNC != 0,
    }))
}

pub fn send(
    descriptor: &Descriptor,
    bytes: &[u8],
    host: Option<&[u8]>,
    port: u16,
    source_host: &[u8],
    interface_index: u32,
    congestion: u8,
) -> crate::Result<usize> {
    let fd = descriptor.raw();
    let source_text = from_utf8(source_host).map_err(|_| Error::invalid("sendmsg"))?;
    let source = if let Ok(source) = source_text.parse::<Ipv4Addr>() {
        SourceAddress::V4(source)
    } else if let Ok(source) = source_text.parse::<Ipv6Addr>() {
        SourceAddress::V6(source)
    } else {
        return Err(Error::invalid("sendmsg"));
    };
    let family = match source {
        SourceAddress::V4(_) => libc::AF_INET,
        SourceAddress::V6(_) => libc::AF_INET6,
    };
    let destination = host
        .map(|host| address(family, host, i64::from(port)))
        .transpose()
        .map_err(|error| Error::new("sendmsg", error))?;
    let explicit_congestion = i32::from(congestion);
    let mut control = [0_usize; CONTROL_WORDS];
    let mut control_length = 0;
    append_source_metadata(
        #[cfg(target_os = "freebsd")]
        descriptor,
        &mut control,
        &mut control_length,
        source,
        interface_index,
        explicit_congestion,
    )
    .map_err(|errno| error("sendmsg", errno))?;

    let mut vector = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: zero is valid for this C output type.
    let mut message = unsafe { zeroed::<libc::msghdr>() };
    if let Some(destination) = &destination {
        message.msg_name = destination.as_ptr().cast_mut().cast();
        message.msg_namelen = destination.len();
    }
    message.msg_iov = &raw mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = abi_length(control_length).map_err(|errno| error("sendmsg", errno))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::sendmsg(fd, &raw const message, 0) };
    if count < 0 {
        return Err(error("sendmsg", last_errno()));
    }
    Ok(count.cast_unsigned())
}

fn decoded_address(storage: &libc::sockaddr_storage) -> crate::Result<Address> {
    match i32::from(storage.ss_family) {
        libc::AF_INET => {
            // SAFETY: the family identifies an IPv4 address in this live storage.
            let address = unsafe { &*from_ref(storage).cast::<libc::sockaddr_in>() };
            Ok(Address {
                host: Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes())
                    .to_string()
                    .into_bytes(),
                port: u16::from_be(address.sin_port),
            })
        }
        libc::AF_INET6 => {
            // SAFETY: the family identifies an IPv6 address in this live storage.
            let address = unsafe { &*from_ref(storage).cast::<libc::sockaddr_in6>() };
            Ok(Address {
                host: Ipv6Addr::from(address.sin6_addr.s6_addr)
                    .to_string()
                    .into_bytes(),
                port: u16::from_be(address.sin6_port),
            })
        }
        _ => Err(error("recvmsg", libc::EAFNOSUPPORT)),
    }
}
