use whim_macros::{whim_closure, whim_function};
use whim_sys::{Interest, Readiness};

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::spec::FunctionSpec;
use crate::builtin::throw::Throw;
use crate::core::async_::task::task_value;
use crate::core::private::syscall::{
    Descriptor, StandardStream, build_file_descriptor, io_error, system_error, with_descriptor,
};
use crate::value::Value;

#[whim_function(
    "Whim\\_Private\\read_descriptor(Whim\\OS\\FileDescriptor $descriptor, (1..) $maxBytes): null|string"
)]
pub(crate) fn read_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let maximum =
        usize::try_from(arguments.int(1)).map_err(|_| system_error(cx, "read", libc::EOVERFLOW))?;
    let bytes = with_descriptor(cx, &arguments.local(0), "read", |descriptor| {
        descriptor.read(maximum)
    })?;
    Ok(bytes.map_or_else(Value::null, |bytes| {
        Value::from_string_vec(cx.vm.heap(), bytes)
    }))
}

#[whim_function(
    "Whim\\_Private\\write_descriptor(Whim\\OS\\FileDescriptor $descriptor, string $bytes): (0..)"
)]
pub(crate) fn write_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let count = with_descriptor(cx, &arguments.local(0), "write", |descriptor| {
        descriptor.write(arguments.bytes(1))
    })?;
    Ok(Value::int(i64::try_from(count).unwrap_or(i64::MAX)))
}

#[whim_function("Whim\\_Private\\flush_descriptor(Whim\\OS\\FileDescriptor $descriptor): void")]
pub(crate) fn flush_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "fflush", Descriptor::flush)?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\set_descriptor_non_blocking(Whim\\OS\\FileDescriptor $descriptor, bool $enabled): void"
)]
pub(crate) fn set_descriptor_non_blocking<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    with_descriptor(cx, &arguments.local(0), "set_non_blocking", |descriptor| {
        descriptor.set_non_blocking(arguments.bool(1))
    })?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\create_pipe(): (Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn create_pipe(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let (read, write) = Descriptor::pipe().map_err(|error| io_error(cx, error))?;
    let read = build_file_descriptor(cx, read)?;
    let write = build_file_descriptor(cx, write)?;
    Ok(cx.tuple([read, write]))
}

#[whim_function(
    "Whim\\_Private\\create_socket_pair(): (Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn create_socket_pair(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let (first, second) = Descriptor::socket_pair().map_err(|error| io_error(cx, error))?;
    let first = build_file_descriptor(cx, first)?;
    let second = build_file_descriptor(cx, second)?;
    Ok(cx.tuple([first, second]))
}

#[whim_function(
    "Whim\\_Private\\standard_descriptors(): (Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn standard_descriptors(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let input = Descriptor::standard(StandardStream::Input).map_err(|error| io_error(cx, error))?;
    let output =
        Descriptor::standard(StandardStream::Output).map_err(|error| io_error(cx, error))?;
    let error = Descriptor::standard(StandardStream::Error).map_err(|error| io_error(cx, error))?;
    let input = build_file_descriptor(cx, input)?;
    let output = build_file_descriptor(cx, output)?;
    let error = build_file_descriptor(cx, error)?;
    Ok(cx.tuple([input, output, error]))
}

fn readiness(cx: &mut Context<'_, '_, '_>, value: &Value) -> Result<Readiness, Throw> {
    with_descriptor(cx, value, "poll", |descriptor| Ok(descriptor.readiness()))
}

pub(crate) fn wait(
    cx: &mut Context<'_, '_, '_>,
    value: &Value,
    interest: Interest,
) -> Result<(), Throw> {
    match readiness(cx, value)? {
        Readiness::Descriptor(source) => match interest {
            Interest::Readable => cx.io_wait_until_readable(source),
            Interest::Writable => cx.io_wait_until_writable(source),
            Interest::ReadableOrWritable => cx.io_wait_until(source, interest),
        },
        Readiness::Poll(interval) => loop {
            cx.vm.loop_park_current_for(interval);
            cx.vm.loop_suspend()?;
            if with_descriptor(cx, value, "poll", |descriptor| {
                descriptor.is_ready(interest)
            })? {
                return Ok(());
            }
        },
    }
}

fn park(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    interest: Interest,
) -> Result<Value, Throw> {
    if cx.vm.loop_current_task().is_none() {
        return Ok(Value::bool(false));
    }
    let value = arguments.local(0);
    match readiness(cx, &value)? {
        Readiness::Descriptor(source) => cx.vm.loop_park_on_fd(source, interest)?,
        Readiness::Poll(_) => wait(cx, &value, interest)?,
    }
    Ok(Value::bool(true))
}

fn arm(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    interest: Interest,
) -> Result<Value, Throw> {
    if cx.vm.loop_current_task().is_none() {
        return Ok(Value::bool(false));
    }
    let Readiness::Descriptor(source) = readiness(cx, &arguments.local(0))? else {
        return Ok(Value::bool(false));
    };
    cx.vm.loop_arm_fd(source, interest)?;
    Ok(Value::bool(true))
}

#[whim_function("Whim\\_Private\\arm_readable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn arm_readable(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    arm(cx, arguments, Interest::Readable)
}

#[whim_function("Whim\\_Private\\arm_writable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn arm_writable(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    arm(cx, arguments, Interest::Writable)
}

#[whim_function("Whim\\_Private\\park_readable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn park_readable(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    park(cx, arguments, Interest::Readable)
}

#[whim_function("Whim\\_Private\\park_writable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn park_writable(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    park(cx, arguments, Interest::Writable)
}

#[whim_closure("(): void")]
fn readable_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    wait(cx, &descriptor, Interest::Readable)?;
    cx.vm.call_function_value(&callback, &[])
}

#[whim_closure("(): void")]
fn writable_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    wait(cx, &descriptor, Interest::Writable)?;
    cx.vm.call_function_value(&callback, &[])
}

fn watch(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    runner: FunctionSpec,
) -> Result<Value, Throw> {
    let runner = cx.closure(runner, &[arguments.local(0), arguments.local(1)]);
    let task = cx.vm.loop_defer(runner)?;
    task_value(cx, task)
}

#[whim_function(
    "Whim\\_Private\\watch_readable(Whim\\OS\\FileDescriptor $descriptor, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_readable(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    watch(cx, arguments, readable_runner_spec())
}

#[whim_function(
    "Whim\\_Private\\watch_writable(Whim\\OS\\FileDescriptor $descriptor, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_writable(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    watch(cx, arguments, writable_runner_spec())
}
