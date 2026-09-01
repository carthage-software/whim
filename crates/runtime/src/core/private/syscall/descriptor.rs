//! Owned descriptor IO, pipes, standard streams, and readiness watchers.

use std::ffi::c_void;
use std::os::fd::AsRawFd;
use std::os::fd::BorrowedFd;
use std::os::fd::OwnedFd;
use std::os::fd::RawFd;

use rustix::io as unix_io;
use rustix::io::Errno;
use rustix::net;
use rustix::pipe;
use whim_loop::Interest;
use whim_macros::whim_closure;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::spec::FunctionSpec;
use crate::builtin::throw::Throw;
use crate::core::async_::task::task_value;
use crate::core::private::syscall::Descriptor;
use crate::core::private::syscall::StandardStream;
use crate::core::private::syscall::build_file_descriptor;
use crate::core::private::syscall::descriptor_of;
use crate::core::private::syscall::last_errno;
use crate::core::private::syscall::set_close_on_exec;
use crate::core::private::syscall::set_non_blocking;
use crate::core::private::syscall::system_error;
use crate::core::private::syscall::with_descriptor;
use crate::value::Value;

fn size(value: i64) -> Result<usize, i32> {
    usize::try_from(value).map_err(|_| libc::EOVERFLOW)
}

const fn would_block(errno: i32) -> bool {
    errno == libc::EAGAIN || errno == libc::EWOULDBLOCK
}

fn integer_count(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

pub(crate) fn duplicate_raw(fd: RawFd) -> Result<OwnedFd, i32> {
    // SAFETY: the handle owns this open descriptor.
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    unix_io::fcntl_dupfd_cloexec(fd, 0).map_err(Errno::raw_os_error)
}

#[whim_function(
    "Whim\\_Private\\read_descriptor(Whim\\OS\\FileDescriptor $descriptor, (1..) $maxBytes): null|string"
)]
pub(crate) fn read_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    let maximum = size(arguments.int(1)).map_err(|errno| system_error(cx, "read", errno))?;
    let mut bytes = Vec::<u8>::with_capacity(maximum);
    let count = with_descriptor(cx, &descriptor, "read", |descriptor| {
        if let Descriptor::Standard(stream) = descriptor {
            let file = stream.file();
            let count =
                // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
                unsafe { libc::fread(bytes.as_mut_ptr().cast::<c_void>(), 1, maximum, file) };
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            if count == 0 && unsafe { libc::ferror(file) } != 0 {
                let errno = last_errno();
                // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
                unsafe { libc::clearerr(file) };
                if would_block(errno) {
                    return Ok(None);
                }
                return Err(errno);
            }
            return Ok(Some(count));
        }
        let fd = descriptor.number();
        // SAFETY: the handle owns this open descriptor.
        let fd = unsafe { BorrowedFd::borrow_raw(fd) };
        match unix_io::read(fd, bytes.spare_capacity_mut()) {
            Ok((initialized, _)) => Ok(Some(initialized.len())),
            Err(error) if would_block(error.raw_os_error()) => Ok(None),
            Err(error) => Err(error.raw_os_error()),
        }
    })?;
    let Some(count) = count else {
        return Ok(Value::null());
    };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    unsafe { bytes.set_len(count) };
    Ok(Value::from_string_vec(cx.vm.heap(), bytes))
}

#[whim_function(
    "Whim\\_Private\\write_descriptor(Whim\\OS\\FileDescriptor $descriptor, string $bytes): (0..)"
)]
pub(crate) fn write_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    let bytes = arguments.bytes(1);
    let count = with_descriptor(cx, &descriptor, "write", |descriptor| {
        if let Descriptor::Standard(stream) = descriptor {
            let file = stream.file();
            let count =
                // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
                unsafe { libc::fwrite(bytes.as_ptr().cast::<c_void>(), 1, bytes.len(), file) };
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            if unsafe { libc::ferror(file) } != 0 {
                let errno = last_errno();
                // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
                unsafe { libc::clearerr(file) };
                if count == 0 {
                    if would_block(errno) {
                        return Ok(0);
                    }
                    return Err(errno);
                }
            }
            return Ok(count);
        }
        let fd = descriptor.number();
        // SAFETY: the handle owns this open descriptor.
        let fd = unsafe { BorrowedFd::borrow_raw(fd) };
        match unix_io::write(fd, bytes) {
            Ok(count) => Ok(count),
            Err(error) if would_block(error.raw_os_error()) => Ok(0),
            Err(error) => Err(error.raw_os_error()),
        }
    })?;
    Ok(Value::int(integer_count(count)))
}

#[whim_function("Whim\\_Private\\flush_descriptor(Whim\\OS\\FileDescriptor $descriptor): void")]
pub(crate) fn flush_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    with_descriptor(cx, &descriptor, "fflush", |descriptor| {
        let Descriptor::Standard(stream) = descriptor else {
            return Ok(());
        };
        let file = stream.file();
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        if unsafe { libc::fflush(file) } != 0 {
            let errno = last_errno();
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            unsafe { libc::clearerr(file) };
            return Err(errno);
        }
        Ok(())
    })?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\set_descriptor_non_blocking(Whim\\OS\\FileDescriptor $descriptor, bool $enabled): void"
)]
pub(crate) fn set_descriptor_non_blocking<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    let enabled = arguments.bool(1);
    let fd = descriptor_of(cx, &descriptor, "fcntl")?;
    set_non_blocking(fd, enabled).map_err(|errno| system_error(cx, "fcntl", errno))?;
    Ok(Value::null())
}

pub(crate) fn pipe_pair() -> Result<(OwnedFd, OwnedFd), i32> {
    let (first, second) = pipe::pipe().map_err(Errno::raw_os_error)?;
    set_close_on_exec(first.as_raw_fd())?;
    set_close_on_exec(second.as_raw_fd())?;
    Ok((first, second))
}

#[whim_function(
    "Whim\\_Private\\create_pipe(): (Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn create_pipe(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let (read, write) = pipe_pair().map_err(|errno| system_error(cx, "pipe", errno))?;
    let read = build_file_descriptor(cx, Descriptor::Raw(read))?;
    let write = build_file_descriptor(cx, Descriptor::Raw(write))?;
    Ok(cx.tuple([read, write]))
}

#[whim_function(
    "Whim\\_Private\\create_socket_pair(): (Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn create_socket_pair(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let (first, second) = net::socketpair(
        net::AddressFamily::UNIX,
        net::SocketType::STREAM,
        net::SocketFlags::empty(),
        None,
    )
    .map_err(|error| system_error(cx, "socketpair", error.raw_os_error()))?;
    set_close_on_exec(first.as_raw_fd()).map_err(|errno| system_error(cx, "fcntl", errno))?;
    set_close_on_exec(second.as_raw_fd()).map_err(|errno| system_error(cx, "fcntl", errno))?;
    set_non_blocking(first.as_raw_fd(), true).map_err(|errno| system_error(cx, "fcntl", errno))?;
    set_non_blocking(second.as_raw_fd(), true).map_err(|errno| system_error(cx, "fcntl", errno))?;
    let first = build_file_descriptor(cx, Descriptor::Raw(first))?;
    let second = build_file_descriptor(cx, Descriptor::Raw(second))?;
    Ok(cx.tuple([first, second]))
}

#[whim_function(
    "Whim\\_Private\\standard_descriptors(): (Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor, Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn standard_descriptors(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    for stream in [
        StandardStream::Input,
        StandardStream::Output,
        StandardStream::Error,
    ] {
        set_non_blocking(stream.number(), true)
            .map_err(|errno| system_error(cx, "fcntl", errno))?;
    }
    let input = build_file_descriptor(cx, Descriptor::Standard(StandardStream::Input))?;
    let output = build_file_descriptor(cx, Descriptor::Standard(StandardStream::Output))?;
    let error = build_file_descriptor(cx, Descriptor::Standard(StandardStream::Error))?;
    Ok(cx.tuple([input, output, error]))
}

fn park<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
    interest: Interest,
) -> Result<Value, Throw> {
    if cx.vm.loop_current_task().is_none() {
        return Ok(Value::bool(false));
    }

    let descriptor = arguments.local(0);
    let fd = descriptor_of(cx, &descriptor, "poll")?;
    cx.vm.loop_park_on_fd(fd, interest)?;
    Ok(Value::bool(true))
}

fn arm<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
    interest: Interest,
) -> Result<Value, Throw> {
    if cx.vm.loop_current_task().is_none() {
        return Ok(Value::bool(false));
    }

    let descriptor = arguments.local(0);
    let fd = descriptor_of(cx, &descriptor, "poll")?;
    cx.vm.loop_arm_fd(fd, interest)?;
    Ok(Value::bool(true))
}

#[whim_function("Whim\\_Private\\arm_readable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn arm_readable<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    arm(cx, arguments, Interest::Readable)
}

#[whim_function("Whim\\_Private\\arm_writable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn arm_writable<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    arm(cx, arguments, Interest::Writable)
}

#[whim_function("Whim\\_Private\\park_readable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn park_readable<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    park(cx, arguments, Interest::Readable)
}

#[whim_function("Whim\\_Private\\park_writable(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn park_writable<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    park(cx, arguments, Interest::Writable)
}

#[whim_closure("(): void")]
fn readable_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    let fd = descriptor_of(cx, &descriptor, "poll")?;
    cx.io_wait_until_readable(fd)?;
    cx.vm.call_function_value(&callback, &[])
}

#[whim_closure("(): void")]
fn writable_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    let fd = descriptor_of(cx, &descriptor, "poll")?;
    cx.io_wait_until_writable(fd)?;
    cx.vm.call_function_value(&callback, &[])
}

fn watch<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
    runner: FunctionSpec,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    let callback = arguments.local(1);
    let runner = cx.closure(runner, &[descriptor, callback]);
    let task = cx.vm.loop_defer(runner)?;
    task_value(cx, task)
}

#[whim_function(
    "Whim\\_Private\\watch_readable(Whim\\OS\\FileDescriptor $descriptor, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_readable<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    watch(cx, arguments, readable_runner_spec())
}

#[whim_function(
    "Whim\\_Private\\watch_writable(Whim\\OS\\FileDescriptor $descriptor, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_writable<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    watch(cx, arguments, writable_runner_spec())
}
