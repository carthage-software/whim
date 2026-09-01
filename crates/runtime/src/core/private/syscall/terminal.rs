//! Terminal inspection primitives.

use std::os::fd::BorrowedFd;

use rustix::io::Errno;
use rustix::termios;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::descriptor_of;
use crate::core::private::syscall::system_error;
use crate::value::Value;

#[whim_function("Whim\\_Private\\is_terminal(Whim\\OS\\FileDescriptor $descriptor): bool")]
pub(crate) fn is_terminal<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    let descriptor = descriptor_of(cx, &descriptor, "isatty")?;
    // SAFETY: the handle owns this open descriptor.
    let descriptor = unsafe { BorrowedFd::borrow_raw(descriptor) };
    Ok(Value::bool(termios::isatty(descriptor)))
}

#[whim_function(
    "Whim\\_Private\\terminal_path(Whim\\OS\\FileDescriptor $descriptor): null|(string&!'')"
)]
pub(crate) fn terminal_path<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    let descriptor = descriptor_of(cx, &descriptor, "ttyname_r")?;
    // SAFETY: the handle owns this open descriptor.
    let descriptor = unsafe { BorrowedFd::borrow_raw(descriptor) };
    match termios::ttyname(descriptor, Vec::new()) {
        Ok(path) => Ok(cx.string(path.to_bytes())),
        Err(Errno::NOTTY) => Ok(Value::null()),
        Err(error) => Err(system_error(cx, "ttyname_r", error.raw_os_error())),
    }
}

#[whim_function(
    "Whim\\_Private\\terminal_size(Whim\\OS\\FileDescriptor $descriptor): null|((1..), (1..))"
)]
pub(crate) fn terminal_size<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let descriptor = arguments.local(0);
    let descriptor = descriptor_of(cx, &descriptor, "ioctl")?;
    // SAFETY: the handle owns this open descriptor.
    let descriptor = unsafe { BorrowedFd::borrow_raw(descriptor) };
    let size = match termios::tcgetwinsize(descriptor) {
        Ok(size) => size,
        Err(Errno::NOTTY) => return Ok(Value::null()),
        Err(error) => return Err(system_error(cx, "ioctl", error.raw_os_error())),
    };

    if size.ws_col == 0 || size.ws_row == 0 {
        return Ok(Value::null());
    }
    let columns = Value::int(i64::from(size.ws_col));
    let rows = Value::int(i64::from(size.ws_row));
    Ok(cx.tuple([columns, rows]))
}
