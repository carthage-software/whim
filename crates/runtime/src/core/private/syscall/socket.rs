use whim_macros::whim_function;
use whim_sys::socket;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::{build_file_descriptor, io_error, with_descriptor};

#[whim_function("Whim\\_Private\\create_socket(int $family, int $kind): Whim\\OS\\FileDescriptor")]
pub(crate) fn create_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor =
        socket::create(arguments.int(0), arguments.int(1)).map_err(|error| io_error(cx, error))?;
    build_file_descriptor(cx, descriptor)
}

#[whim_function(
    "Whim\\_Private\\bind_socket(Whim\\OS\\FileDescriptor $socket, string $host, (0..) $port): void"
)]
pub(crate) fn bind_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "bind", |descriptor| {
        socket::bind(descriptor, arguments.bytes(1), arguments.int(2))
    })?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\listen_socket(Whim\\OS\\FileDescriptor $socket, (1..) $backlog): void"
)]
pub(crate) fn listen_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "listen", |descriptor| {
        socket::listen(descriptor, arguments.int(1))
    })?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\accept_socket(Whim\\OS\\FileDescriptor $socket): null|(Whim\\OS\\FileDescriptor, string, (0..))"
)]
pub(crate) fn accept_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let Some((descriptor, address)) =
        with_descriptor(cx, &arguments.local(0), "accept", socket::accept)?
    else {
        return Ok(Value::null());
    };
    let descriptor = build_file_descriptor(cx, descriptor)?;
    Ok(cx.tuple([
        descriptor,
        cx.string(&address.host),
        Value::int(i64::from(address.port)),
    ]))
}

#[whim_function(
    "Whim\\_Private\\connect_socket(Whim\\OS\\FileDescriptor $socket, string $host, (0..) $port): bool"
)]
pub(crate) fn connect_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "connect", |descriptor| {
        socket::connect(descriptor, arguments.bytes(1), arguments.int(2))
    })
    .map(Value::bool)
}

#[whim_function(
    "Whim\\_Private\\complete_socket_connection(Whim\\OS\\FileDescriptor $socket): void"
)]
pub(crate) fn complete_socket_connection<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(
        cx,
        &arguments.local(0),
        "connect",
        socket::complete_connection,
    )?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\socket_address(Whim\\OS\\FileDescriptor $socket): (string, (0..))"
)]
pub(crate) fn socket_address<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let address = with_descriptor(
        cx,
        &arguments.local(0),
        "getsockname",
        socket::local_address,
    )?;
    Ok(cx.tuple([
        cx.string(&address.host),
        Value::int(i64::from(address.port)),
    ]))
}

#[whim_function(
    "Whim\\_Private\\set_socket_option(Whim\\OS\\FileDescriptor $socket, int $level, int $option, int $value): void"
)]
pub(crate) fn set_socket_option<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "setsockopt", |descriptor| {
        socket::set_option(
            descriptor,
            arguments.int(1),
            arguments.int(2),
            arguments.int(3),
        )
    })?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\send_to(Whim\\OS\\FileDescriptor $socket, string $bytes, string $host, (0..) $port): (0..)"
)]
pub(crate) fn send_to<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let count = with_descriptor(cx, &arguments.local(0), "sendto", |descriptor| {
        socket::send_to(
            descriptor,
            arguments.bytes(1),
            arguments.bytes(2),
            arguments.int(3),
        )
    })?;
    Ok(Value::int(i64::try_from(count).unwrap_or(i64::MAX)))
}

#[whim_function(
    "Whim\\_Private\\shutdown_socket(Whim\\OS\\FileDescriptor $socket, int $direction): void"
)]
pub(crate) fn shutdown_socket<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "shutdown", |descriptor| {
        socket::shutdown(descriptor, arguments.int(1))
    })?;
    Ok(Value::null())
}
