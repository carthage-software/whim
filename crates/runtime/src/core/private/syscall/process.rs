//! Unix process, identity, signal, and child-watcher primitives.

use std::env::vars_os;
use std::ffi::CString;
use std::ffi::OsString;
use std::fs::File;
use std::io::Error;
use std::io::Write;
use std::mem::size_of;
use std::mem::zeroed;
use std::os::fd::AsRawFd;
use std::os::fd::FromRawFd;
use std::os::fd::IntoRawFd;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::process::Stdio;
use std::ptr::null;
use std::ptr::null_mut;
use std::thread::Builder;

use signal_hook::low_level::pipe::register;
use whim_macros::whim_closure;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::async_::task::task_value;
use crate::core::private::syscall::Descriptor;
use crate::core::private::syscall::SignalDescriptor;
use crate::core::private::syscall::build_file_descriptor;
use crate::core::private::syscall::descriptor::pipe_pair;
use crate::core::private::syscall::descriptor_of;
use crate::core::private::syscall::last_errno;
use crate::core::private::syscall::last_system_error;
use crate::core::private::syscall::set_close_on_exec;
use crate::core::private::syscall::set_non_blocking;
use crate::core::private::syscall::system_error;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::dict::DictObject;
use crate::value::heap::handle::ManagedRef;
use crate::value::vec::VecObject;

#[cfg(target_os = "macos")]
type GroupCount = libc::c_int;

#[cfg(target_os = "linux")]
type GroupCount = libc::size_t;

#[cfg(target_os = "macos")]
type InitialGroup = libc::c_int;

#[cfg(target_os = "linux")]
type InitialGroup = libc::gid_t;

#[cfg(target_os = "linux")]
type Resource = libc::__rlimit_resource_t;

#[cfg(target_os = "macos")]
type Resource = libc::c_int;

#[cfg(target_os = "macos")]
const CLOCK_ERROR: libc::clock_t = libc::clock_t::MAX;

#[cfg(target_os = "linux")]
const CLOCK_ERROR: libc::clock_t = -1;

fn c_string(
    cx: &mut Context<'_, '_, '_>,
    bytes: &[u8],
    call: &'static str,
) -> Result<CString, Throw> {
    CString::new(bytes).map_err(|_| system_error(cx, call, libc::EINVAL))
}

fn id(value: libc::c_uint) -> i64 {
    i64::from(value)
}

fn user_id(
    cx: &mut Context<'_, '_, '_>,
    value: i64,
    call: &'static str,
) -> Result<libc::uid_t, Throw> {
    if value == -1 {
        return Ok(libc::uid_t::MAX);
    }
    libc::uid_t::try_from(value).map_err(|_| system_error(cx, call, libc::EINVAL))
}

fn group_id(
    cx: &mut Context<'_, '_, '_>,
    value: i64,
    call: &'static str,
) -> Result<libc::gid_t, Throw> {
    if value == -1 {
        return Ok(libc::gid_t::MAX);
    }
    libc::gid_t::try_from(value).map_err(|_| system_error(cx, call, libc::EINVAL))
}

fn process_id(
    cx: &mut Context<'_, '_, '_>,
    value: i64,
    call: &'static str,
) -> Result<libc::pid_t, Throw> {
    libc::pid_t::try_from(value).map_err(|_| system_error(cx, call, libc::EINVAL))
}

#[cfg(target_os = "linux")]
fn clear_errno() {
    // SAFETY: libc returns a writable pointer to this thread's errno value.
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(target_os = "macos")]
fn clear_errno() {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    unsafe { *libc::__error() = 0 };
}

#[whim_function("Whim\\_Private\\parent_process_id(): (0..)")]
pub(crate) fn parent_process_id() -> Value {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    Value::int(i64::from(unsafe { libc::getppid() }.max(0)))
}

#[whim_function("Whim\\_Private\\process_user(): ((0..), (0..))")]
pub(crate) fn process_user(cx: &Context<'_, '_, '_>) -> Value {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let real = Value::int(id(unsafe { libc::getuid() }));
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let effective = Value::int(id(unsafe { libc::geteuid() }));
    cx.tuple([real, effective])
}

#[whim_function("Whim\\_Private\\process_group(): ((0..), (0..))")]
pub(crate) fn process_group(cx: &Context<'_, '_, '_>) -> Value {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let real = Value::int(id(unsafe { libc::getgid() }));
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let effective = Value::int(id(unsafe { libc::getegid() }));
    cx.tuple([real, effective])
}

#[whim_function("Whim\\_Private\\process_supplementary_groups(): vec<(0..)>")]
pub(crate) fn process_supplementary_groups(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::getgroups(0, null_mut()) };
    if count < 0 {
        return Err(last_system_error(cx, "getgroups"));
    }
    let capacity =
        usize::try_from(count).map_err(|_| system_error(cx, "getgroups", libc::EOVERFLOW))?;
    let mut groups = vec![libc::gid_t::default(); capacity];
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let count = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
    if count < 0 {
        return Err(last_system_error(cx, "getgroups"));
    }
    let count =
        usize::try_from(count).map_err(|_| system_error(cx, "getgroups", libc::EOVERFLOW))?;
    groups.truncate(count);
    Ok(cx.vec(groups.into_iter().map(|group| Value::int(id(group)))))
}

#[whim_function("Whim\\_Private\\set_process_user(int $real, int $effective): void")]
pub(crate) fn set_process_user<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let real = user_id(cx, arguments.int(0), "setreuid")?;
    let effective = user_id(cx, arguments.int(1), "setreuid")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::setreuid(real, effective) } < 0 {
        return Err(last_system_error(cx, "setreuid"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\set_process_group(int $real, int $effective): void")]
pub(crate) fn set_process_group<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let real = group_id(cx, arguments.int(0), "setregid")?;
    let effective = group_id(cx, arguments.int(1), "setregid")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::setregid(real, effective) } < 0 {
        return Err(last_system_error(cx, "setregid"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\set_process_supplementary_groups(vec<(0..)> $groups): void")]
pub(crate) fn set_process_supplementary_groups<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let groups = arguments.vec(0);
    let groups = groups
        .iter()
        // SAFETY: built-in argument validation proves every group is an integer.
        .map(|group| unsafe { group.as_int_unchecked() })
        .map(libc::gid_t::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| system_error(cx, "setgroups", libc::EINVAL))?;
    let count = GroupCount::try_from(groups.len())
        .map_err(|_| system_error(cx, "setgroups", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::setgroups(count, groups.as_ptr()) } < 0 {
        return Err(last_system_error(cx, "setgroups"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\initialize_groups((string&!'') $user, (0..) $group): void")]
pub(crate) fn initialize_groups<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let user = arguments.bytes(0);
    let user = c_string(cx, user, "initgroups")?;
    let group = InitialGroup::try_from(arguments.int(1))
        .map_err(|_| system_error(cx, "initgroups", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::initgroups(user.as_ptr(), group) } < 0 {
        return Err(last_system_error(cx, "initgroups"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\session_id((0..) $process): (0..)")]
pub(crate) fn session_id<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = process_id(cx, arguments.int(0), "getsid")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let session = unsafe { libc::getsid(process) };
    if session < 0 {
        return Err(last_system_error(cx, "getsid"));
    }
    Ok(Value::int(i64::from(session)))
}

#[whim_function("Whim\\_Private\\start_session(): (0..)")]
pub(crate) fn start_session(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let session = unsafe { libc::setsid() };
    if session < 0 {
        return Err(last_system_error(cx, "setsid"));
    }
    Ok(Value::int(i64::from(session)))
}

#[whim_function("Whim\\_Private\\process_group_id((0..) $process): (0..)")]
pub(crate) fn process_group_id<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = process_id(cx, arguments.int(0), "getpgid")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let group = unsafe { libc::getpgid(process) };
    if group < 0 {
        return Err(last_system_error(cx, "getpgid"));
    }
    Ok(Value::int(i64::from(group)))
}

#[whim_function("Whim\\_Private\\set_process_group_id((0..) $process, (0..) $group): void")]
pub(crate) fn set_process_group_id<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = process_id(cx, arguments.int(0), "setpgid")?;
    let group = process_id(cx, arguments.int(1), "setpgid")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::setpgid(process, group) } < 0 {
        return Err(last_system_error(cx, "setpgid"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\process_priority((0..) $process): int")]
pub(crate) fn process_priority<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = libc::id_t::try_from(arguments.int(0))
        .map_err(|_| system_error(cx, "getpriority", libc::EINVAL))?;
    clear_errno();
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let priority = unsafe { libc::getpriority(libc::PRIO_PROCESS, process) };
    let errno = last_errno();
    if priority == -1 && errno != 0 {
        return Err(system_error(cx, "getpriority", errno));
    }
    Ok(Value::int(i64::from(priority)))
}

#[whim_function("Whim\\_Private\\set_process_priority((0..) $process, int $priority): void")]
pub(crate) fn set_process_priority<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = libc::id_t::try_from(arguments.int(0))
        .map_err(|_| system_error(cx, "setpriority", libc::EINVAL))?;
    let priority = i32::try_from(arguments.int(1))
        .map_err(|_| system_error(cx, "setpriority", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, process, priority) } < 0 {
        return Err(last_system_error(cx, "setpriority"));
    }
    Ok(Value::null())
}

fn limit_to_int(limit: libc::rlim_t) -> i64 {
    if limit == libc::RLIM_INFINITY {
        -1
    } else {
        i64::try_from(limit).unwrap_or(i64::MAX)
    }
}

fn int_to_limit(
    cx: &mut Context<'_, '_, '_>,
    value: i64,
    call: &'static str,
) -> Result<libc::rlim_t, Throw> {
    if value < 0 {
        Ok(libc::RLIM_INFINITY)
    } else {
        libc::rlim_t::try_from(value).map_err(|_| system_error(cx, call, libc::EINVAL))
    }
}

#[whim_function("Whim\\_Private\\resource_limit(int $resource): (int, int)")]
pub(crate) fn resource_limit<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let resource = Resource::try_from(arguments.int(0))
        .map_err(|_| system_error(cx, "getrlimit", libc::EINVAL))?;
    // SAFETY: zero is valid for this C output type.
    let mut limit = unsafe { zeroed::<libc::rlimit>() };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::getrlimit(resource, &raw mut limit) } < 0 {
        return Err(last_system_error(cx, "getrlimit"));
    }
    let soft = Value::int(limit_to_int(limit.rlim_cur));
    let hard = Value::int(limit_to_int(limit.rlim_max));
    Ok(cx.tuple([soft, hard]))
}

#[whim_function("Whim\\_Private\\set_resource_limit(int $resource, int $soft, int $hard): void")]
pub(crate) fn set_resource_limit<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let resource = Resource::try_from(arguments.int(0))
        .map_err(|_| system_error(cx, "setrlimit", libc::EINVAL))?;
    let limit = libc::rlimit {
        rlim_cur: int_to_limit(cx, arguments.int(1), "setrlimit")?,
        rlim_max: int_to_limit(cx, arguments.int(2), "setrlimit")?,
    };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::setrlimit(resource, &raw const limit) } < 0 {
        return Err(last_system_error(cx, "setrlimit"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\exchange_file_mode_mask(0..=511 $mask): 0..=511")]
pub(crate) fn exchange_file_mode_mask<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let mask = libc::mode_t::try_from(arguments.int(0))
        .map_err(|_| system_error(cx, "umask", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    Ok(Value::int(i64::from(unsafe { libc::umask(mask) })))
}

#[whim_function("Whim\\_Private\\process_times(): (int, int, int, int)")]
pub(crate) fn process_times(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    // SAFETY: zero is valid for this C output type.
    let mut times = unsafe { zeroed::<libc::tms>() };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::times(&raw mut times) } == CLOCK_ERROR {
        return Err(last_system_error(cx, "times"));
    }
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return Err(last_system_error(cx, "sysconf"));
    }
    let micros = |ticks: libc::clock_t| {
        let value = i128::from(ticks) * 1_000_000 / i128::from(ticks_per_second);
        i64::try_from(value).unwrap_or_else(|_| {
            if value.is_negative() {
                i64::MIN
            } else {
                i64::MAX
            }
        })
    };
    let user = Value::int(micros(times.tms_utime));
    let system = Value::int(micros(times.tms_stime));
    let child_user = Value::int(micros(times.tms_cutime));
    let child_system = Value::int(micros(times.tms_cstime));
    Ok(cx.tuple([user, system, child_user, child_system]))
}

fn strings_from_vec(
    cx: &mut Context<'_, '_, '_>,
    values: &ManagedRef<VecObject>,
    call: &'static str,
) -> Result<Vec<CString>, Throw> {
    values
        .iter()
        .map(|value| {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let bytes = unsafe {
                unwrap_option_invariant(
                    value.as_string_bytes(),
                    "a validated string vec contains strings",
                )
            };
            c_string(cx, bytes, call)
        })
        .collect()
}

fn environment_from_dict(
    cx: &mut Context<'_, '_, '_>,
    environment: &ManagedRef<DictObject>,
    call: &'static str,
) -> Result<Vec<CString>, Throw> {
    environment
        .iter()
        .map(|(name, value)| {
            let name = name.to_value();
            // SAFETY: the surrounding invariant proves this option contains a value.
            let name = unsafe {
                unwrap_option_invariant(
                    name.as_string_bytes(),
                    "a validated environment has string names",
                )
            };
            // SAFETY: the surrounding invariant proves this option contains a value.
            let value = unsafe {
                unwrap_option_invariant(
                    value.as_string_bytes(),
                    "a validated environment has string values",
                )
            };
            let mut pair = Vec::with_capacity(name.len() + value.len() + 1);
            pair.extend_from_slice(name);
            pair.push(b'=');
            pair.extend_from_slice(value);
            c_string(cx, &pair, call)
        })
        .collect()
}

fn inherited_environment(
    cx: &mut Context<'_, '_, '_>,
    call: &'static str,
) -> Result<Vec<CString>, Throw> {
    vars_os()
        .map(|(name, value)| {
            let name = name.as_os_str().as_bytes();
            let value = value.as_os_str().as_bytes();
            let mut pair = Vec::with_capacity(name.len() + value.len() + 1);
            pair.extend_from_slice(name);
            pair.push(b'=');
            pair.extend_from_slice(value);
            c_string(cx, &pair, call)
        })
        .collect()
}

#[whim_function(
    "Whim\\_Private\\replace_process((string&!'') $program, vec<string> $arguments, null|dict<(string&!''), string> $environment): never"
)]
pub(crate) fn replace_process<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let program = arguments.bytes(0);
    let program = c_string(cx, program, "execve")?;
    let argument_values = arguments.vec(1);
    let argument_strings = strings_from_vec(cx, &argument_values, "execve")?;
    let environment_value = arguments.local(2);
    let environment_strings = if environment_value.is_null() {
        inherited_environment(cx, "execve")?
    } else {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let environment = unsafe {
            unwrap_option_invariant(
                environment_value.as_dict(),
                "a validated non-null environment is a dict",
            )
        };
        environment_from_dict(cx, environment, "execve")?
    };
    let mut argument_pointers = Vec::with_capacity(argument_strings.len() + 2);
    argument_pointers.push(program.as_ptr());
    argument_pointers.extend(argument_strings.iter().map(|value| value.as_ptr()));
    argument_pointers.push(null());
    let mut environment_pointers = Vec::with_capacity(environment_strings.len() + 1);
    environment_pointers.extend(environment_strings.iter().map(|value| value.as_ptr()));
    environment_pointers.push(null());
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    unsafe {
        libc::execve(
            program.as_ptr(),
            argument_pointers.as_ptr(),
            environment_pointers.as_ptr(),
        )
    };
    Err(last_system_error(cx, "execve"))
}

#[whim_function("Whim\\_Private\\send_signal(int $process, (0..) $signal): void")]
pub(crate) fn send_signal<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = process_id(cx, arguments.int(0), "kill")?;
    let signal =
        i32::try_from(arguments.int(1)).map_err(|_| system_error(cx, "kill", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::kill(process, signal) } < 0 {
        return Err(last_system_error(cx, "kill"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\process_exists((1..) $process): bool", must_use)]
pub(crate) fn process_exists<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = libc::pid_t::try_from(arguments.int(0))
        .map_err(|_| system_error(cx, "kill", libc::EINVAL))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::kill(process, 0) } == 0 {
        return Ok(Value::bool(true));
    }

    match last_errno() {
        libc::ESRCH => Ok(Value::bool(false)),
        libc::EPERM => Ok(Value::bool(true)),
        errno => Err(system_error(cx, "kill", errno)),
    }
}

#[whim_closure("(): void")]
fn signal_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    let fd = descriptor_of(cx, &descriptor, "read")?;
    loop {
        cx.io_wait_until_readable(fd)?;
        let mut byte = 0_u8;
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        let result = unsafe { libc::read(fd, (&raw mut byte).cast(), 1) };
        if result == 1 {
            cx.vm.call_function_value(&callback, &[])?;
            continue;
        }
        if result == 0 {
            return Err(system_error(cx, "read", libc::EPIPE));
        }
        if !matches!(last_errno(), libc::EINTR | libc::EAGAIN) {
            return Err(last_system_error(cx, "read"));
        }
    }
}

#[whim_function(
    "Whim\\_Private\\watch_signal((1..) $signal, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_signal<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let signal =
        i32::try_from(arguments.int(0)).map_err(|_| system_error(cx, "sigaction", libc::EINVAL))?;
    if signal == libc::SIGKILL || signal == libc::SIGSTOP {
        return Err(system_error(cx, "sigaction", libc::EINVAL));
    }
    let callback = arguments.local(1);
    let (read, write) = UnixStream::pair().map_err(|error| {
        system_error(cx, "socketpair", error.raw_os_error().unwrap_or(libc::EIO))
    })?;
    read.set_nonblocking(true)
        .map_err(|error| system_error(cx, "fcntl", error.raw_os_error().unwrap_or(libc::EIO)))?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let read = unsafe { OwnedFd::from_raw_fd(read.into_raw_fd()) };
    set_close_on_exec(read.as_raw_fd()).map_err(|errno| system_error(cx, "fcntl", errno))?;
    let registration = register(signal, write).map_err(|error| {
        system_error(cx, "sigaction", error.raw_os_error().unwrap_or(libc::EIO))
    })?;
    let descriptor = build_file_descriptor(
        cx,
        Descriptor::Signal(SignalDescriptor { read, registration }),
    )?;
    let runner = cx.closure(signal_runner_spec(), &[descriptor, callback]);
    let task = cx.vm.loop_defer(runner)?;
    task_value(cx, task)
}

fn stream(cx: &mut Context<'_, '_, '_>, value: &Value, input: bool) -> Result<Stdio, Throw> {
    // SAFETY: the surrounding invariant proves this option contains a value.
    let elements = unsafe {
        unwrap_option_invariant(value.as_tuple(), "a validated process stream is a tuple")
    };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let disposition = unsafe {
        unwrap_option_invariant(
            elements.get(0).and_then(Value::as_int),
            "a validated process stream has an integer disposition",
        )
    };
    match disposition {
        0 => Ok(Stdio::inherit()),
        1 => Ok(Stdio::null()),
        2 => Ok(Stdio::piped()),
        3 => {
            let Some(descriptor) = elements.get(1) else {
                return Err(system_error(cx, "spawn", libc::EBADF));
            };
            let fd = descriptor_of(cx, descriptor, "spawn")?;
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
            if copy < 0 {
                return Err(last_system_error(cx, "fcntl"));
            }
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let copy = unsafe { OwnedFd::from_raw_fd(copy) };
            Ok(Stdio::from(copy))
        }
        4 => {
            let path = c"/dev/tty";
            let flags = if input {
                libc::O_RDONLY
            } else {
                libc::O_WRONLY
            };
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let fd = unsafe { libc::open(path.as_ptr(), flags | libc::O_CLOEXEC) };
            if fd < 0 {
                return Err(last_system_error(cx, "open"));
            }
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            Ok(Stdio::from(fd))
        }
        _ => Err(system_error(cx, "spawn", libc::EINVAL)),
    }
}

fn configure_arguments(
    cx: &mut Context<'_, '_, '_>,
    command: &mut Command,
    arguments: &ManagedRef<VecObject>,
) -> Result<(), Throw> {
    for argument in arguments.iter() {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let argument = unsafe {
            unwrap_option_invariant(
                argument.as_string_bytes(),
                "validated process arguments are strings",
            )
        };
        if argument.contains(&0) {
            return Err(system_error(cx, "spawn", libc::EINVAL));
        }
        command.arg(OsString::from_vec(argument.to_vec()));
    }
    Ok(())
}

fn configure_environment(
    cx: &mut Context<'_, '_, '_>,
    command: &mut Command,
    environment: &Value,
) -> Result<(), Throw> {
    if environment.is_null() {
        return Ok(());
    }

    command.env_clear();
    // SAFETY: the surrounding invariant proves this option contains a value.
    let environment = unsafe {
        unwrap_option_invariant(
            environment.as_dict(),
            "a validated non-null process environment is a dict",
        )
    };
    for (name, value) in environment.iter() {
        let name = name.to_value();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let name = unsafe {
            unwrap_option_invariant(
                name.as_string_bytes(),
                "validated process environment names are strings",
            )
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let value = unsafe {
            unwrap_option_invariant(
                value.as_string_bytes(),
                "validated process environment values are strings",
            )
        };
        if name.contains(&0) || name.contains(&b'=') || value.contains(&0) {
            return Err(system_error(cx, "spawn", libc::EINVAL));
        }
        command.env(
            OsString::from_vec(name.to_vec()),
            OsString::from_vec(value.to_vec()),
        );
    }
    Ok(())
}

fn configure_directory(
    cx: &mut Context<'_, '_, '_>,
    command: &mut Command,
    directory: &Value,
) -> Result<(), Throw> {
    if directory.is_null() {
        return Ok(());
    }
    // SAFETY: the surrounding invariant proves this option contains a value.
    let directory = unsafe {
        unwrap_option_invariant(
            directory.as_string_bytes(),
            "a validated non-null process directory is a string",
        )
    };
    if directory.contains(&0) {
        return Err(system_error(cx, "spawn", libc::EINVAL));
    }
    command.current_dir(OsString::from_vec(directory.to_vec()));
    Ok(())
}

fn inherited_mappings(
    cx: &mut Context<'_, '_, '_>,
    inherited: &ManagedRef<VecObject>,
) -> Result<Vec<(OwnedFd, libc::c_int)>, Throw> {
    let mappings = inherited
        .iter()
        .map(|mapping| {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let elements = unsafe {
                unwrap_option_invariant(
                    mapping.as_tuple(),
                    "validated inherited descriptor mappings are tuples",
                )
            };
            // SAFETY: the surrounding invariant proves this option contains a value.
            let source = unsafe {
                unwrap_option_invariant(
                    elements.get(0),
                    "an inherited descriptor mapping has a source",
                )
            };
            let source = descriptor_of(cx, source, "spawn")?;
            // SAFETY: the surrounding invariant proves this option contains a value.
            let target = unsafe {
                unwrap_option_invariant(
                    elements.get(1).and_then(Value::as_int),
                    "an inherited descriptor mapping has an integer target",
                )
            };
            let target = libc::c_int::try_from(target)
                .map_err(|_| system_error(cx, "spawn", libc::EINVAL))?;
            Ok((source, target))
        })
        .collect::<Result<Vec<_>, Throw>>()?;
    let minimum = mappings
        .iter()
        .map(|(_, target)| *target)
        .max()
        .unwrap_or(2)
        .checked_add(1)
        .ok_or_else(|| system_error(cx, "fcntl", libc::EINVAL))?;
    mappings
        .into_iter()
        .map(|(source, target)| {
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            let source = unsafe { libc::fcntl(source, libc::F_DUPFD_CLOEXEC, minimum) };
            if source < 0 {
                return Err(last_system_error(cx, "fcntl"));
            }
            // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
            Ok((unsafe { OwnedFd::from_raw_fd(source) }, target))
        })
        .collect()
}

fn child_descriptor<T>(cx: &mut Context<'_, '_, '_>, stream: Option<T>) -> Result<Value, Throw>
where
    T: IntoRawFd,
{
    let Some(stream) = stream else {
        return Ok(Value::null());
    };
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let descriptor = unsafe { OwnedFd::from_raw_fd(stream.into_raw_fd()) };
    set_non_blocking(descriptor.as_raw_fd(), true)
        .map_err(|errno| system_error(cx, "fcntl", errno))?;
    build_file_descriptor(cx, Descriptor::Raw(descriptor))
}

#[whim_function(
    "Whim\\_Private\\spawn_process((string&!'') $program, vec<string> $arguments, null|dict<(string&!''), string> $environment, null|(string&!'') $directory, vec<(int, null|Whim\\OS\\FileDescriptor)> $streams, vec<(Whim\\OS\\FileDescriptor, (0..))> $inherited, int $processGroup): ((0..), null|Whim\\OS\\FileDescriptor, null|Whim\\OS\\FileDescriptor, null|Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn spawn_process<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let program = arguments.bytes(0).to_vec();
    if program.contains(&0) {
        return Err(system_error(cx, "spawn", libc::EINVAL));
    }
    let mut command = Command::new(OsString::from_vec(program));

    configure_arguments(cx, &mut command, &arguments.vec(1))?;
    let environment = arguments.local(2);
    configure_environment(cx, &mut command, &environment)?;
    let directory = arguments.local(3);
    configure_directory(cx, &mut command, &directory)?;

    let streams = arguments.vec(4);
    if streams.len() != 3 {
        return Err(system_error(cx, "spawn", libc::EINVAL));
    }
    // SAFETY: the surrounding invariant proves this option contains a value.
    let input = unsafe {
        unwrap_option_invariant(
            streams.get(0),
            "three process streams include standard input",
        )
    };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let output = unsafe {
        unwrap_option_invariant(
            streams.get(1),
            "three process streams include standard output",
        )
    };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let error = unsafe {
        unwrap_option_invariant(
            streams.get(2),
            "three process streams include standard error",
        )
    };
    command.stdin(stream(cx, input, true)?);
    command.stdout(stream(cx, output, false)?);
    command.stderr(stream(cx, error, false)?);

    let mappings = inherited_mappings(cx, &arguments.vec(5))?;

    let process_group = arguments.int(6);
    if process_group >= 0 {
        command.process_group(process_id(cx, process_group, "spawn")?);
    }
    if !mappings.is_empty() {
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        unsafe {
            command.pre_exec(move || {
                for (source, target) in &mappings {
                    if libc::dup2(source.as_raw_fd(), *target) < 0 {
                        return Err(Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }

    let mut child = command
        .spawn()
        .map_err(|error| system_error(cx, "spawn", error.raw_os_error().unwrap_or(libc::EIO)))?;
    let process = Value::int(i64::from(child.id()));
    let input = child_descriptor(cx, child.stdin.take())?;
    let output = child_descriptor(cx, child.stdout.take())?;
    let error = child_descriptor(cx, child.stderr.take())?;
    Ok(cx.tuple([process, input, output, error]))
}

#[whim_closure("(): void")]
fn process_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    let fd = descriptor_of(cx, &descriptor, "read")?;
    let mut status = 0_i32;
    let mut offset = 0_usize;
    while offset < size_of::<i32>() {
        cx.io_wait_until_readable(fd)?;
        // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
        let result = unsafe {
            libc::read(
                fd,
                (&raw mut status).cast::<u8>().add(offset).cast(),
                size_of::<i32>() - offset,
            )
        };
        if result < 0 {
            match last_errno() {
                libc::EINTR | libc::EAGAIN => continue,
                _ => return Err(last_system_error(cx, "read")),
            }
        }
        if result == 0 {
            return Err(system_error(cx, "read", libc::EPIPE));
        }
        offset += result.cast_unsigned();
    }
    let status = Value::int(i64::from(status));
    cx.vm.call_function_value(&callback, &[status])
}

#[whim_function(
    "Whim\\_Private\\watch_process((0..) $process, (fn(int): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_process<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let process = process_id(cx, arguments.int(0), "waitpid")?;
    let callback = arguments.local(1);
    let (read, write) = pipe_pair().map_err(|errno| system_error(cx, "pipe", errno))?;
    set_non_blocking(read.as_raw_fd(), true).map_err(|errno| system_error(cx, "fcntl", errno))?;
    let watcher = Builder::new()
        .name(format!("whim-child-{process}"))
        .spawn(move || {
            let mut status = 0_i32;
            loop {
                // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
                let result = unsafe { libc::waitpid(process, &raw mut status, 0) };
                if result >= 0 {
                    break;
                }
                let errno = last_errno();
                if errno != libc::EINTR {
                    status = -errno;
                    break;
                }
            }
            let mut write = File::from(write);
            let _ = write.write_all(&status.to_ne_bytes());
        })
        .map_err(|error| {
            system_error(
                cx,
                "pthread_create",
                error.raw_os_error().unwrap_or(libc::EAGAIN),
            )
        })?;
    drop(watcher);
    let descriptor = build_file_descriptor(cx, Descriptor::Raw(read))?;
    let runner = cx.closure(process_runner_spec(), &[descriptor, callback]);
    let task = cx.vm.loop_defer(runner)?;
    task_value(cx, task)
}
