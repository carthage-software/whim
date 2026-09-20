use crate::file;
use rustix::{io::fcntl_dupfd_cloexec, pipe::pipe};
use signal_hook::low_level::pipe::register;
use std::env::vars_os;
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io::{self, Write};
use std::mem::zeroed;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::{ffi::OsStrExt, net::UnixStream, process::CommandExt};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::thread::Builder;

use crate::platform::descriptor::{Resource as DescriptorResource, close_on_exec};
use crate::process::{Exit, Prepared, Spawn, Spawned};
use crate::{Descriptor, Error, Result};

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
type GroupCount = libc::c_int;
#[cfg(target_os = "linux")]
type GroupCount = libc::size_t;
#[cfg(target_os = "macos")]
type InitialGroup = libc::c_int;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
type InitialGroup = libc::gid_t;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
type Resource = libc::__rlimit_resource_t;
#[cfg(any(target_os = "macos", target_os = "freebsd", target_env = "musl"))]
type Resource = libc::c_int;

fn checked<T: TryFrom<i64>>(value: i64, call: &'static str) -> Result<T> {
    T::try_from(value).map_err(|_| Error::invalid(call))
}
fn status(value: i32, call: &'static str) -> Result<()> {
    if value < 0 {
        Err(Error::last(call))
    } else {
        Ok(())
    }
}
fn identity(value: i64, call: &'static str) -> Result<u32> {
    if value == -1 {
        Ok(u32::MAX)
    } else {
        checked(value, call)
    }
}

pub fn parent_id() -> Result<u32> {
    // SAFETY: getppid only reads the current process identity.
    Ok(unsafe { libc::getppid() }.max(0) as u32)
}

pub fn user() -> Result<(u32, u32)> {
    // SAFETY: these calls only read the current process identity.
    Ok(unsafe { (libc::getuid(), libc::geteuid()) })
}

pub fn group() -> Result<(u32, u32)> {
    // SAFETY: these calls only read the current process identity.
    Ok(unsafe { (libc::getgid(), libc::getegid()) })
}

pub fn supplementary_groups() -> Result<Vec<u32>> {
    // SAFETY: a zero count queries the required capacity without writing memory.
    let count = unsafe { libc::getgroups(0, null_mut()) };
    status(count, "getgroups")?;
    let mut groups = vec![0; count as usize];
    // SAFETY: groups has space for count entries.
    let count = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
    status(count, "getgroups")?;
    groups.truncate(count as usize);
    Ok(groups)
}

pub fn set_user(real: i64, effective: i64) -> Result<()> {
    let (real, effective) = (
        identity(real, "setreuid")?,
        identity(effective, "setreuid")?,
    );
    // SAFETY: both identifiers fit the platform types.
    status(unsafe { libc::setreuid(real, effective) }, "setreuid")
}

pub fn set_group(real: i64, effective: i64) -> Result<()> {
    let (real, effective) = (
        identity(real, "setregid")?,
        identity(effective, "setregid")?,
    );
    // SAFETY: both identifiers fit the platform types.
    status(unsafe { libc::setregid(real, effective) }, "setregid")
}

pub fn set_supplementary_groups(groups: &[i64]) -> Result<()> {
    let groups: Vec<libc::gid_t> = groups
        .iter()
        .map(|group| checked(*group, "setgroups"))
        .collect::<Result<_>>()?;
    let count = GroupCount::try_from(groups.len()).map_err(|_| Error::invalid("setgroups"))?;
    // SAFETY: groups contains exactly count readable identifiers.
    status(
        unsafe { libc::setgroups(count, groups.as_ptr()) },
        "setgroups",
    )
}

pub fn initialize_groups(user: &[u8], group: i64) -> Result<()> {
    let user = CString::new(user).map_err(|_| Error::invalid("initgroups"))?;
    let group: InitialGroup = checked(group, "initgroups")?;
    // SAFETY: user is null terminated and group fits the platform type.
    status(
        unsafe { libc::initgroups(user.as_ptr(), group) },
        "initgroups",
    )
}

pub fn session_id(process: i64) -> Result<u32> {
    let process = checked(process, "getsid")?;
    // SAFETY: getsid validates the process identifier.
    let value = unsafe { libc::getsid(process) };
    status(value, "getsid")?;
    Ok(value as u32)
}

pub fn start_session() -> Result<u32> {
    // SAFETY: setsid does not borrow memory.
    let value = unsafe { libc::setsid() };
    status(value, "setsid")?;
    Ok(value as u32)
}

pub fn group_id(process: i64) -> Result<u32> {
    let process = checked(process, "getpgid")?;
    // SAFETY: getpgid validates the process identifier.
    let value = unsafe { libc::getpgid(process) };
    status(value, "getpgid")?;
    Ok(value as u32)
}

pub fn set_group_id(process: i64, group: i64) -> Result<()> {
    let (process, group) = (checked(process, "setpgid")?, checked(group, "setpgid")?);
    // SAFETY: both identifiers fit the platform types.
    status(unsafe { libc::setpgid(process, group) }, "setpgid")
}

pub fn priority(process: i64) -> Result<i32> {
    let process = checked(process, "getpriority")?;
    #[cfg(target_os = "linux")]
    // SAFETY: libc returns this thread's writable errno cell.
    unsafe {
        *libc::__errno_location() = 0
    };
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    // SAFETY: libc returns this thread's writable errno cell.
    unsafe {
        *libc::__error() = 0
    };
    // SAFETY: getpriority validates the identifier.
    let priority = unsafe { libc::getpriority(libc::PRIO_PROCESS, process) };
    let error = io::Error::last_os_error();
    if priority == -1 && error.raw_os_error().unwrap_or(0) != 0 {
        return Err(Error::new("getpriority", error));
    }
    Ok(priority)
}

pub fn set_priority(process: i64, priority: i64) -> Result<()> {
    let (process, priority) = (
        checked(process, "setpriority")?,
        checked(priority, "setpriority")?,
    );
    // SAFETY: both arguments fit the platform types.
    status(
        unsafe { libc::setpriority(libc::PRIO_PROCESS, process, priority) },
        "setpriority",
    )
}

pub fn resource_limit(resource: i64) -> Result<(i64, i64)> {
    let resource: Resource = checked(resource, "getrlimit")?;
    // SAFETY: an all-zero rlimit is valid output storage.
    let mut limit = unsafe { zeroed::<libc::rlimit>() };
    status(
        // SAFETY: limit is live output storage.
        unsafe { libc::getrlimit(resource, &raw mut limit) },
        "getrlimit",
    )?;
    let integer = |value| {
        if value == libc::RLIM_INFINITY {
            -1
        } else {
            i64::try_from(i128::from(value)).unwrap_or(i64::MAX)
        }
    };

    Ok((integer(limit.rlim_cur), integer(limit.rlim_max)))
}

pub fn set_resource_limit(resource: i64, soft: i64, hard: i64) -> Result<()> {
    let resource: Resource = checked(resource, "setrlimit")?;
    let limit = |value| {
        if value < 0 {
            Ok(libc::RLIM_INFINITY)
        } else {
            checked(value, "setrlimit")
        }
    };
    let limit = libc::rlimit {
        rlim_cur: limit(soft)?,
        rlim_max: limit(hard)?,
    };
    if limit.rlim_cur > limit.rlim_max {
        return Err(Error::invalid("setrlimit"));
    }
    // SAFETY: limit is live and both limits fit the platform type.
    status(
        unsafe { libc::setrlimit(resource, &raw const limit) },
        "setrlimit",
    )
}

#[cfg_attr(
    target_os = "linux",
    expect(clippy::useless_conversion, reason = "mode_t can be u16 or u32")
)]
pub fn exchange_creation_mask(mask: i64) -> Result<u32> {
    let mask = checked(mask, "umask")?;
    // SAFETY: mask fits the platform type.
    Ok(u32::from(unsafe { libc::umask(mask) }))
}

pub fn replace(
    program: &OsStr,
    arguments: &[OsString],
    environment: Option<&[(OsString, OsString)]>,
) -> Result<()> {
    let cstring = |bytes: &[u8]| CString::new(bytes).map_err(|_| Error::invalid("execve"));
    let program = cstring(program.as_bytes())?;
    let arguments = arguments
        .iter()
        .map(|value| cstring(value.as_bytes()))
        .collect::<Result<Vec<_>>>()?;
    let inherited;
    let environment = match environment {
        Some(environment) => environment,
        None => {
            inherited = vars_os().collect::<Vec<_>>();
            &inherited
        }
    };
    let environment = environment
        .iter()
        .map(|(name, value)| {
            let mut bytes = name.as_bytes().to_vec();
            bytes.push(b'=');
            bytes.extend_from_slice(value.as_bytes());
            cstring(&bytes)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut argv = vec![program.as_ptr()];
    argv.extend(arguments.iter().map(|value| value.as_ptr()));
    argv.push(null());
    let mut envp = environment
        .iter()
        .map(|value| value.as_ptr())
        .collect::<Vec<_>>();
    envp.push(null());
    // SAFETY: all strings remain live and both pointer arrays end in null.
    unsafe { libc::execve(program.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
    Err(Error::last("execve"))
}

pub fn send_signal(process: i64, signal: i64) -> Result<()> {
    let (process, signal) = (checked(process, "kill")?, checked(signal, "kill")?);
    // SAFETY: kill validates these identifiers.
    status(unsafe { libc::kill(process, signal) }, "kill")
}

pub fn exists(process: i64) -> Result<bool> {
    let process = checked(process, "kill")?;
    // SAFETY: signal zero only checks the process and permissions.
    if unsafe { libc::kill(process, 0) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(Error::new("kill", error)),
    }
}

pub fn terminate(process: i64) -> Result<()> {
    send_signal(process, i64::from(libc::SIGKILL))
}

pub fn watch_signal(signal: i64) -> Result<Descriptor> {
    let signal = checked(signal, "sigaction")?;
    if matches!(signal, libc::SIGKILL | libc::SIGSTOP) {
        return Err(Error::invalid("sigaction"));
    }
    let (read, write) = UnixStream::pair().map_err(|error| Error::new("socketpair", error))?;
    read.set_nonblocking(true)
        .map_err(|error| Error::new("fcntl", error))?;
    let read: OwnedFd = read.into();
    close_on_exec(read.as_raw_fd())?;
    let registration = register(signal, write).map_err(|error| Error::new("sigaction", error))?;
    Ok(Descriptor {
        inner: DescriptorResource::Signal { read, registration },
    })
}

pub(crate) fn terminal_stream(input: bool) -> Result<File> {
    file::open(
        Path::new("/dev/tty"),
        i64::from(if input {
            libc::O_RDONLY
        } else {
            libc::O_WRONLY
        }),
        0,
    )
}

pub fn validate_spawn(_inherited: bool, _group: bool) -> Result<()> {
    Ok(())
}

#[derive(Default)]
pub struct Processes {}

impl Processes {
    pub fn spawn(&mut self, request: Spawn) -> Result<Spawned> {
        let Prepared {
            mut command,
            inherited: mappings,
            group,
        } = request.command()?;
        if let Some(group) = group {
            command.process_group(group);
        }
        if !mappings.is_empty() {
            let minimum = mappings
                .iter()
                .map(|(_, target)| *target)
                .max()
                .unwrap_or(2)
                .checked_add(1)
                .ok_or_else(|| Error::invalid("fcntl"))?;
            let mappings = mappings
                .into_iter()
                .map(|(file, target)| {
                    fcntl_dupfd_cloexec(&file, minimum)
                        .map(|file| (file, target))
                        .map_err(|error| Error::new("fcntl", error))
                })
                .collect::<Result<Vec<_>>>()?;
            // SAFETY: the child callback uses only async-signal-safe dup2 and borrows live descriptors.
            unsafe {
                command.pre_exec(move || {
                    for (source, target) in &mappings {
                        if libc::dup2(source.as_raw_fd(), *target) < 0 {
                            return Err(io::Error::last_os_error());
                        }
                    }
                    Ok(())
                });
            }
        }
        let mut child = command
            .spawn()
            .map_err(|error| Error::new("spawn", error))?;
        let result = (|| {
            Ok(Spawned {
                id: child.id(),
                input: child_descriptor(child.stdin.take())?,
                output: child_descriptor(child.stdout.take())?,
                error: child_descriptor(child.stderr.take())?,
            })
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }

    pub fn watch(&mut self, process: i64) -> Result<Descriptor> {
        let process = checked(process, "waitpid")?;
        let (read, write) = pipe().map_err(|error| Error::new("pipe", error))?;
        close_on_exec(read.as_raw_fd())?;
        close_on_exec(write.as_raw_fd())?;
        let read = Descriptor::owned(read);
        read.set_non_blocking(true)?;
        Builder::new()
            .name(format!("whim-child-{process}"))
            .spawn(move || {
                let mut status = 0;
                loop {
                    // SAFETY: waitpid validates process and writes live status storage.
                    if unsafe { libc::waitpid(process, &raw mut status, 0) } >= 0 {
                        break;
                    }
                    let errno = io::Error::last_os_error()
                        .raw_os_error()
                        .unwrap_or(libc::EIO);
                    if errno != libc::EINTR {
                        status = -errno;
                        break;
                    }
                }
                let _ = File::from(write).write_all(&status.to_ne_bytes());
            })
            .map_err(|error| Error::new("pthread_create", error))?;
        Ok(read)
    }

    pub fn record_exit(&mut self, exit: Exit) -> i64 {
        exit.status
    }

    pub fn times(&self) -> Result<[i64; 4]> {
        // SAFETY: all-zero tms is valid output storage.
        let mut times = unsafe { zeroed::<libc::tms>() };
        #[cfg(target_os = "macos")]
        let failure = libc::clock_t::MAX;
        #[cfg(not(target_os = "macos"))]
        let failure = -1;
        // SAFETY: times is live output storage.
        if unsafe { libc::times(&raw mut times) } == failure {
            return Err(Error::last("times"));
        }
        // SAFETY: this sysconf key does not borrow memory.
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if ticks <= 0 {
            return Err(Error::last("sysconf"));
        }
        Ok([
            times.tms_utime,
            times.tms_stime,
            times.tms_cutime,
            times.tms_cstime,
        ]
        .map(|value| {
            (i128::from(value) * 1_000_000 / i128::from(ticks))
                .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
        }))
    }
}

fn child_descriptor<T: IntoRawFd>(stream: Option<T>) -> Result<Option<Descriptor>> {
    stream
        .map(|stream| {
            // SAFETY: ownership transfers from the child stream to OwnedFd.
            let descriptor =
                Descriptor::owned(unsafe { OwnedFd::from_raw_fd(stream.into_raw_fd()) });
            descriptor.set_non_blocking(true)?;
            Ok(descriptor)
        })
        .transpose()
}

pub fn watch_descriptor(descriptor: &Descriptor) -> Result<crate::RawDescriptor> {
    Ok(descriptor.raw())
}

pub fn read_exit(descriptor: &Descriptor) -> Result<Option<Exit>> {
    let bytes = match descriptor.read(size_of::<i32>()) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(Error::Io { source, .. }) if source.kind() == io::ErrorKind::Interrupted => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let bytes =
        <[u8; 4]>::try_from(bytes).map_err(|_| Error::new("read", io::ErrorKind::BrokenPipe))?;
    Ok(Some(Exit {
        status: i64::from(i32::from_ne_bytes(bytes)),
        user_time: 0,
        system_time: 0,
    }))
}
