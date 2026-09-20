use std::io;
use std::mem::size_of;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::windows::io::AsRawSocket;
use std::ptr;
use std::str::from_utf8;

use socket2::SockAddr;
use windows_sys::Win32::Networking::WinSock as ws;

use crate::message::Message;
use crate::platform::socket::{family, last_error};
use crate::socket::{Address, address, decoded_address};
use crate::{Descriptor, Error};

fn raw_socket(descriptor: &Descriptor, call: &'static str) -> crate::Result<usize> {
    descriptor
        .with_socket(|socket| Ok(socket.as_raw_socket() as usize))
        .map_err(|error| Error::new(call, error))
}

const CONTROL_WORDS: usize = 32;

const fn align(length: usize) -> usize {
    length.next_multiple_of(size_of::<usize>())
}

fn append_control<T: Copy>(
    control: &mut [usize; CONTROL_WORDS],
    used: &mut usize,
    level: i32,
    kind: i32,
    value: T,
) {
    let length = align(size_of::<ws::CMSGHDR>()) + size_of::<T>();
    assert!(*used + align(length) <= size_of_val(control));
    // SAFETY: the bounds check covers the header and payload, and each header is word-aligned.
    unsafe {
        let start = control.as_mut_ptr().cast::<u8>().add(*used);
        start.cast::<ws::CMSGHDR>().write_unaligned(ws::CMSGHDR {
            cmsg_len: length,
            cmsg_level: level,
            cmsg_type: kind,
        });

        start
            .add(align(size_of::<ws::CMSGHDR>()))
            .cast::<T>()
            .write_unaligned(value);
    }

    *used += align(length);
}

fn set_option(socket: usize, level: i32, option: i32) -> io::Result<()> {
    let enabled = 1_i32;
    // SAFETY: enabled is a readable integer with the supplied length.
    if unsafe {
        ws::setsockopt(
            socket,
            level,
            option,
            (&raw const enabled).cast(),
            i32::try_from(size_of::<i32>()).unwrap(),
        )
    } != 0
    {
        return Err(last_error());
    }

    Ok(())
}

fn receive_function(socket: usize) -> io::Result<ws::LPFN_WSARECVMSG> {
    let mut function: ws::LPFN_WSARECVMSG = None;
    let identifier = ws::WSAID_WSARECVMSG;
    let mut returned = 0;
    // SAFETY: the extension identifier and function pointer have the sizes required by Winsock.
    if unsafe {
        ws::WSAIoctl(
            socket,
            ws::SIO_GET_EXTENSION_FUNCTION_POINTER,
            (&raw const identifier).cast(),
            u32::try_from(size_of_val(&identifier)).unwrap(),
            (&raw mut function).cast(),
            u32::try_from(size_of_val(&function)).unwrap(),
            &raw mut returned,
            ptr::null_mut(),
            None,
        )
    } != 0
    {
        return Err(last_error());
    }

    function
        .ok_or_else(|| io::Error::from(io::ErrorKind::Unsupported))
        .map(Some)
}

pub fn enable_metadata(descriptor: &Descriptor) -> crate::Result<()> {
    let socket = raw_socket(descriptor, "setsockopt")?;
    let (ipv4, ipv6) = descriptor
        .with_socket(|socket| {
            let local = socket
                .local_addr()?
                .as_socket()
                .ok_or_else(|| io::Error::from_raw_os_error(ws::WSAEAFNOSUPPORT))?;
            Ok(match local.ip() {
                IpAddr::V4(_) => (true, false),
                IpAddr::V6(address) if address.to_ipv4_mapped().is_some() => (true, false),
                IpAddr::V6(address) => (address.is_unspecified() && !socket.only_v6()?, true),
            })
        })
        .map_err(|error| Error::new("getsockname", error))?;

    let result = if ipv4 {
        set_option(socket, ws::IPPROTO_IP, ws::IP_PKTINFO)
            .and_then(|()| set_option(socket, ws::IPPROTO_IP, ws::IP_RECVECN))
    } else {
        Ok(())
    };

    result.map_err(|error| Error::new("setsockopt", error))?;
    let result = if ipv6 {
        set_option(socket, ws::IPPROTO_IPV6, ws::IPV6_PKTINFO)
            .and_then(|()| set_option(socket, ws::IPPROTO_IPV6, ws::IPV6_RECVECN))
    } else {
        Ok(())
    };

    result.map_err(|error| Error::new("setsockopt", error))
}

pub fn receive(
    descriptor: &Descriptor,
    maximum: usize,
    local_port: u16,
) -> crate::Result<Option<Message>> {
    let socket = raw_socket(descriptor, "WSARecvMsg")?;
    let maximum = u32::try_from(maximum).map_err(|_| Error::invalid("WSARecvMsg"))?;
    let receive = receive_function(socket)
        .map_err(|error| Error::new("WSAIoctl", error))?
        .unwrap();
    let mut bytes = vec![0_u8; maximum as usize];
    let mut control = [0_usize; CONTROL_WORDS];
    let mut buffer = ws::WSABUF {
        len: maximum,
        buf: bytes.as_mut_ptr(),
    };

    let mut received = 0;
    // SAFETY: SockAddr supplies a full address buffer; all other buffers remain live until the synchronous call returns.
    let received_message = unsafe {
        SockAddr::try_init(|name, length| {
            let mut message = ws::WSAMSG {
                name: name.cast(),
                namelen: *length,
                lpBuffers: &raw mut buffer,
                dwBufferCount: 1,
                Control: ws::WSABUF {
                    len: u32::try_from(size_of_val(&control)).unwrap(),
                    buf: control.as_mut_ptr().cast(),
                },
                dwFlags: 0,
            };

            if receive(
                socket,
                &raw mut message,
                &raw mut received,
                ptr::null_mut(),
                None,
            ) != 0
            {
                let error = last_error();
                if error.raw_os_error() != Some(ws::WSAEMSGSIZE) {
                    return Err(error);
                }
                message.dwFlags |= ws::MSG_TRUNC;
            }

            *length = message.namelen;
            Ok((message.dwFlags, message.Control.len as usize))
        })
    };

    let ((flags, control_length), peer) = match received_message {
        Ok(message) => message,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
        Err(error) => return Err(Error::new("WSARecvMsg", error)),
    };

    if flags & ws::MSG_CTRUNC != 0 || control_length > size_of_val(&control) {
        return Err(Error::new("WSARecvMsg", io::ErrorKind::InvalidData));
    }

    let (local, congestion) =
        message_metadata(&control, control_length, peer.family() == ws::AF_INET6);
    let (local_host, interface) = if let Some(local) = local {
        local
    } else {
        (
            descriptor
                .with_socket(|socket| decoded_address(&socket.local_addr()?))
                .map_err(|error| Error::new("getsockname", error))?
                .host,
            0,
        )
    };

    let peer = decoded_address(&peer).map_err(|error| Error::new("WSARecvMsg", error))?;
    bytes.truncate(received as usize);
    Ok(Some(Message {
        bytes,
        peer,
        local: Address {
            host: local_host,
            port: local_port,
        },
        congestion: congestion as u8,
        interface: interface as u32,
        truncated: flags & ws::MSG_TRUNC != 0,
    }))
}

fn message_metadata(
    control: &[usize; CONTROL_WORDS],
    control_length: usize,
    ipv6: bool,
) -> (Option<(Vec<u8>, i64)>, i64) {
    let mut local = None;
    let mut congestion = 0;
    let mut offset = 0;
    while offset + size_of::<ws::CMSGHDR>() <= control_length.min(size_of_val(control)) {
        // SAFETY: offset is aligned, and the loop checked the complete header lies in the control buffer.
        let header = unsafe {
            control
                .as_ptr()
                .cast::<u8>()
                .add(offset)
                .cast::<ws::CMSGHDR>()
                .read_unaligned()
        };

        let payload_offset = offset + align(size_of::<ws::CMSGHDR>());
        if header.cmsg_len < align(size_of::<ws::CMSGHDR>())
            || header.cmsg_len > control_length - offset
        {
            break;
        }

        let payload_length = header.cmsg_len - align(size_of::<ws::CMSGHDR>());
        // SAFETY: each read below checks the payload length before reading a plain C data structure.
        unsafe {
            let payload = control.as_ptr().cast::<u8>().add(payload_offset);
            match (header.cmsg_level, header.cmsg_type) {
                (ws::IPPROTO_IP, ws::IP_PKTINFO)
                    if payload_length >= size_of::<ws::IN_PKTINFO>() =>
                {
                    let info = payload.cast::<ws::IN_PKTINFO>().read_unaligned();
                    let address = Ipv4Addr::from(info.ipi_addr.S_un.S_addr.to_ne_bytes());
                    local = Some((
                        if ipv6 {
                            address.to_ipv6_mapped().to_string()
                        } else {
                            address.to_string()
                        }
                        .into_bytes(),
                        i64::from(info.ipi_ifindex),
                    ));
                }
                (ws::IPPROTO_IPV6, ws::IPV6_PKTINFO)
                    if payload_length >= size_of::<ws::IN6_PKTINFO>() =>
                {
                    let info = payload.cast::<ws::IN6_PKTINFO>().read_unaligned();
                    local = Some((
                        Ipv6Addr::from(info.ipi6_addr.u.Byte)
                            .to_string()
                            .into_bytes(),
                        i64::from(info.ipi6_ifindex),
                    ));
                }
                (ws::IPPROTO_IP, ws::IP_ECN) | (ws::IPPROTO_IPV6, ws::IPV6_ECN)
                    if payload_length >= size_of::<u32>() =>
                {
                    congestion = i64::from(payload.cast::<u32>().read_unaligned() & 3);
                }
                _ => {}
            }
        }

        offset += align(header.cmsg_len);
    }

    (local, congestion)
}

fn send_control(
    control: &mut [usize; CONTROL_WORDS],
    source: IpAddr,
    interface: u32,
    congestion: u32,
) -> usize {
    let source = match source {
        IpAddr::V6(address) => address.to_ipv4_mapped().map_or(source, IpAddr::V4),
        source @ IpAddr::V4(_) => source,
    };

    let mut used = 0;
    match source {
        IpAddr::V4(source) => {
            let mut info = ws::IN_PKTINFO::default();
            info.ipi_addr.S_un.S_addr = u32::from_ne_bytes(source.octets());
            info.ipi_ifindex = interface;
            append_control(control, &mut used, ws::IPPROTO_IP, ws::IP_PKTINFO, info);
            append_control(control, &mut used, ws::IPPROTO_IP, ws::IP_ECN, congestion);
        }
        IpAddr::V6(source) => {
            let mut info = ws::IN6_PKTINFO::default();
            info.ipi6_addr.u.Byte = source.octets();
            info.ipi6_ifindex = interface;
            append_control(control, &mut used, ws::IPPROTO_IPV6, ws::IPV6_PKTINFO, info);
            append_control(
                control,
                &mut used,
                ws::IPPROTO_IPV6,
                ws::IPV6_ECN,
                congestion,
            );
        }
    }

    used
}

pub fn send(
    descriptor: &Descriptor,
    bytes: &[u8],
    host: Option<&[u8]>,
    port: u16,
    source_host: &[u8],
    interface: u32,
    congestion: u8,
) -> crate::Result<usize> {
    let socket = raw_socket(descriptor, "WSASendMsg")?;
    let source = from_utf8(source_host)
        .ok()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .ok_or_else(|| Error::invalid("WSASendMsg"))?;
    if congestion == 3 {
        return Err(Error::Unsupported(
            "sending UDP packets marked as congested",
        ));
    }

    let family = descriptor
        .with_socket(family)
        .map_err(|error| Error::new("getsockopt", error))?;
    let mut control = [0_usize; CONTROL_WORDS];
    let used = send_control(&mut control, source, interface, u32::from(congestion));
    let destination = host
        .map(|host| address(family, host, i64::from(port)))
        .transpose()
        .map_err(|error| Error::new("WSASendMsg", error))?;
    let mut buffer = ws::WSABUF {
        len: u32::try_from(bytes.len()).map_err(|_| Error::invalid("WSASendMsg"))?,
        buf: bytes.as_ptr().cast_mut(),
    };

    let message = ws::WSAMSG {
        name: destination.as_ref().map_or(ptr::null_mut(), |address| {
            address.as_ptr().cast_mut().cast()
        }),
        namelen: destination.as_ref().map_or(0, SockAddr::len),
        lpBuffers: &raw mut buffer,
        dwBufferCount: 1,
        Control: ws::WSABUF {
            len: u32::try_from(used).unwrap(),
            buf: control.as_mut_ptr().cast(),
        },
        dwFlags: 0,
    };

    let mut sent = 0;
    // SAFETY: the synchronous call reads live address, payload, and control buffers and writes only sent.
    if unsafe {
        ws::WSASendMsg(
            socket,
            &raw const message,
            0,
            &raw mut sent,
            ptr::null_mut(),
            None,
        )
    } != 0
    {
        return Err(Error::new("WSASendMsg", last_error()));
    }

    Ok(sent as usize)
}
