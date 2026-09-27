use whim_macros::whim_function;
use whim_sys::message;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::{system_error, with_descriptor};

#[whim_function("Whim\\_Private\\enable_message_metadata(Whim\\OS\\FileDescriptor $socket): void")]
pub(crate) fn enable_message_metadata<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(
        cx,
        &arguments.local(0),
        "setsockopt",
        message::enable_metadata,
    )?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\receive_message(Whim\\OS\\FileDescriptor $socket, (1u..) $maxBytes, (0u..=65535u) $localPort): null|(string, (string, (0u..=65535u)), (string, (0u..=65535u)), 0u..=3u, uint, bool)"
)]
pub(crate) fn receive_message<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let maximum = isize::try_from(arguments.uint(1))
        .map_err(|_| system_error(cx, "recvmsg", libc::EINVAL))?
        .cast_unsigned();
    let port =
        u16::try_from(arguments.uint(2)).map_err(|_| system_error(cx, "recvmsg", libc::EINVAL))?;
    let Some(message) = with_descriptor(cx, &arguments.local(0), "recvmsg", |descriptor| {
        message::receive(descriptor, maximum, port)
    })?
    else {
        return Ok(Value::null());
    };
    Ok(cx.tuple([
        Value::from_string_vec(cx.vm.heap(), message.bytes),
        cx.tuple([
            cx.string(&message.peer.host),
            Value::uint(u64::from(message.peer.port)),
        ]),
        cx.tuple([
            cx.string(&message.local.host),
            Value::uint(u64::from(message.local.port)),
        ]),
        Value::uint(u64::from(message.congestion)),
        Value::uint(u64::from(message.interface)),
        Value::bool(message.truncated),
    ]))
}

#[whim_function(
    "Whim\\_Private\\send_message(Whim\\OS\\FileDescriptor $socket, string $bytes, null|string $host, (0u..=65535u) $port, string $sourceHost, uint $interfaceIndex, 0u..=3u $explicitCongestion): uint"
)]
pub(crate) fn send_message<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let host = arguments.local(2);
    let port =
        u16::try_from(arguments.uint(3)).map_err(|_| system_error(cx, "sendmsg", libc::EINVAL))?;
    let interface =
        u32::try_from(arguments.uint(5)).map_err(|_| system_error(cx, "sendmsg", libc::EINVAL))?;
    let congestion =
        u8::try_from(arguments.uint(6)).map_err(|_| system_error(cx, "sendmsg", libc::EINVAL))?;
    let count = with_descriptor(cx, &arguments.local(0), "sendmsg", |descriptor| {
        message::send(
            descriptor,
            arguments.bytes(1),
            host.as_string_bytes(),
            port,
            arguments.bytes(4),
            interface,
            congestion,
        )
    })?;
    Ok(Value::uint(count as u64))
}
