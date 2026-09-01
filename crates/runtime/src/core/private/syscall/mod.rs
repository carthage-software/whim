//! Thin Unix system-call primitives used by the Whim-written standard library.

use std::cell::Cell;
use std::cell::RefCell;
use std::io::Error;
use std::os::fd::AsRawFd;
use std::os::fd::BorrowedFd;
use std::os::fd::OwnedFd;
use std::os::fd::RawFd;

use rustix::fs;
use rustix::io as unix_io;
use rustix::io::Errno;
use signal_hook::low_level::unregister;
use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::descriptor::duplicate_raw;
use crate::unwrap_option_invariant;
use crate::value::Value;

pub(crate) mod constants;
pub(crate) mod descriptor;
pub(crate) mod message;
pub(crate) mod path;
pub(crate) mod process;
pub(crate) mod socket;
pub(crate) mod system;
pub(crate) mod terminal;

const SYSTEM_ERROR: &str = "Whim\\_Private\\SystemError";
const FILE_DESCRIPTOR: &str = "Whim\\OS\\FileDescriptor";

pub(crate) enum Descriptor {
    Raw(OwnedFd),
    Signal(SignalDescriptor),
    Standard(StandardStream),
}

/// A C standard stream owned by libc.
#[derive(Clone, Copy)]
pub(crate) enum StandardStream {
    Input,
    Output,
    Error,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    static mut __stdinp: *mut libc::FILE;
    static mut __stdoutp: *mut libc::FILE;
    static mut __stderrp: *mut libc::FILE;
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    static mut stdin: *mut libc::FILE;
    static mut stdout: *mut libc::FILE;
    static mut stderr: *mut libc::FILE;
}

impl StandardStream {
    pub(crate) fn file(self) -> *mut libc::FILE {
        #[cfg(target_os = "macos")]
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        unsafe {
            match self {
                Self::Input => __stdinp,
                Self::Output => __stdoutp,
                Self::Error => __stderrp,
            }
        }
        #[cfg(target_os = "linux")]
        // SAFETY: libc owns these process-wide stream pointers for the program's lifetime.
        unsafe {
            match self {
                Self::Input => stdin,
                Self::Output => stdout,
                Self::Error => stderr,
            }
        }
    }

    pub(crate) const fn number(self) -> RawFd {
        match self {
            Self::Input => libc::STDIN_FILENO,
            Self::Output => libc::STDOUT_FILENO,
            Self::Error => libc::STDERR_FILENO,
        }
    }
}

pub(crate) struct SignalDescriptor {
    pub(crate) read: OwnedFd,
    pub(crate) registration: signal_hook::SigId,
}

impl Drop for SignalDescriptor {
    fn drop(&mut self) {
        unregister(self.registration);
    }
}

impl Descriptor {
    pub(crate) fn number(&self) -> RawFd {
        match self {
            Self::Raw(descriptor) => descriptor.as_raw_fd(),
            Self::Signal(descriptor) => descriptor.read.as_raw_fd(),
            Self::Standard(stream) => stream.number(),
        }
    }
}

#[whim_class("Whim\\_Private\\SystemError", final)]
#[whim_extends("Whim\\Unwind\\Error")]
pub(crate) struct SystemError;

#[whim_methods]
impl SystemError {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("errno(): int")]
    fn errno(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        cx.get_property(&receiver, "code")
    }

    #[whim_method("call(): (string&!'')")]
    fn call(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        cx.get_property(&receiver, "message")
    }
}

#[whim_class("Whim\\OS\\FileDescriptor", final)]
#[derive(Default)]
pub(crate) struct FileDescriptor {
    descriptor: RefCell<Option<Descriptor>>,
    number: Cell<RawFd>,
}

default_built_in_state!(FileDescriptor);

#[whim_methods]
impl FileDescriptor {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("duplicate(int $number): Whim\\OS\\FileDescriptor", static)]
    fn duplicate<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let number = RawFd::try_from(arguments.int(0))
            .map_err(|_| system_error(cx, "fcntl", libc::EBADF))?;
        let descriptor = duplicate_raw(number).map_err(|errno| system_error(cx, "fcntl", errno))?;
        build_file_descriptor(cx, Descriptor::Raw(descriptor))
    }

    #[whim_method("toInt(): (0..)")]
    fn to_int(cx: &Context<'_, '_, '_>) -> Value {
        let receiver = cx.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let state = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a file descriptor method receives its built-in state",
            )
        };
        let number = state.number.get();
        Value::int(i64::from(number.max(0)))
    }

    #[whim_method("isClosed(): bool")]
    fn is_closed(cx: &Context<'_, '_, '_>) -> Value {
        let receiver = cx.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let state = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a file descriptor method receives its built-in state",
            )
        };
        Value::bool(state.descriptor.borrow().is_none())
    }

    #[whim_method("close(): void")]
    fn close(cx: &Context<'_, '_, '_>) -> Value {
        let receiver = cx.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let state = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a file descriptor method receives its built-in state",
            )
        };
        state.descriptor.borrow_mut().take();
        Value::null()
    }
}

pub(crate) fn system_error(cx: &mut Context<'_, '_, '_>, call: &'static str, errno: i32) -> Throw {
    let class = cx.vm.intern(SYSTEM_ERROR.as_bytes());
    cx.vm.throw(class, call, i64::from(errno))
}

pub(crate) fn last_system_error(cx: &mut Context<'_, '_, '_>, call: &'static str) -> Throw {
    let errno = Error::last_os_error().raw_os_error().unwrap_or(libc::EIO);
    system_error(cx, call, errno)
}

pub(crate) fn build_file_descriptor(
    cx: &mut Context<'_, '_, '_>,
    descriptor: Descriptor,
) -> Result<Value, Throw> {
    let number = descriptor.number();
    let object = cx.new_built_in_instance(FILE_DESCRIPTOR)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<FileDescriptor>(&object),
            "a new file descriptor has built-in state",
        )
    };
    state.number.set(number);
    *state.descriptor.borrow_mut() = Some(descriptor);
    Ok(object)
}

pub(crate) fn descriptor_of(
    cx: &mut Context<'_, '_, '_>,
    value: &Value,
    call: &'static str,
) -> Result<RawFd, Throw> {
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<FileDescriptor>(value),
            "a validated descriptor argument has built-in state",
        )
    };
    let descriptor = state.descriptor.borrow();
    let Some(descriptor) = descriptor.as_ref() else {
        return Err(system_error(cx, call, libc::EBADF));
    };
    Ok(descriptor.number())
}

pub(crate) fn with_descriptor<T>(
    cx: &mut Context<'_, '_, '_>,
    value: &Value,
    call: &'static str,
    operation: impl FnOnce(&Descriptor) -> Result<T, i32>,
) -> Result<T, Throw> {
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<FileDescriptor>(value),
            "a validated descriptor argument has built-in state",
        )
    };
    let descriptor = state.descriptor.borrow();
    let Some(descriptor) = descriptor.as_ref() else {
        return Err(system_error(cx, call, libc::EBADF));
    };
    operation(descriptor).map_err(|errno| system_error(cx, call, errno))
}

pub(crate) fn set_close_on_exec(fd: RawFd) -> Result<(), i32> {
    // SAFETY: the handle owns this open descriptor.
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let flags = unix_io::fcntl_getfd(fd).map_err(Errno::raw_os_error)?;
    unix_io::fcntl_setfd(fd, flags | unix_io::FdFlags::CLOEXEC).map_err(Errno::raw_os_error)
}

pub(crate) fn set_non_blocking(fd: RawFd, enabled: bool) -> Result<(), i32> {
    // SAFETY: the handle owns this open descriptor.
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let flags = fs::fcntl_getfl(fd).map_err(Errno::raw_os_error)?;
    let flags = if enabled {
        flags | fs::OFlags::NONBLOCK
    } else {
        flags & !fs::OFlags::NONBLOCK
    };
    fs::fcntl_setfl(fd, flags).map_err(Errno::raw_os_error)
}

pub(crate) fn last_errno() -> i32 {
    Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)
}
