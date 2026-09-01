//! Blocking file operations bridged into the event loop.

use std::cell::RefCell;
use std::ffi::CStr;
use std::ffi::CString;
use std::fs::File;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::os::fd::AsRawFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use rustix::fs;
use rustix::io as unix_io;
use rustix::io::Errno;
use whim_macros::whim_class;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::private::blocking::Operation;
use crate::core::private::syscall::Descriptor;
use crate::core::private::syscall::build_file_descriptor;
use crate::core::private::syscall::descriptor::duplicate_raw;
use crate::core::private::syscall::descriptor_of;
use crate::core::private::syscall::last_errno;
use crate::core::private::syscall::path::temporary_template;
use crate::core::private::syscall::set_close_on_exec;
use crate::core::private::syscall::system_error;
use crate::unwrap_result_invariant;
use crate::value::Value;

const FILE_OPERATION: &str = "Whim\\_Private\\FileOperation";

struct FileError {
    call: &'static str,
    errno: i32,
}

enum FileResult {
    Bytes(Vec<u8>),
    Integer(i64),
    Boolean(bool),
    Metadata([i64; 15]),
    Descriptor(OwnedFd),
}

type Shared = Operation<FileResult, FileError>;

#[whim_class("Whim\\_Private\\FileOperation", final)]
#[derive(Default)]
pub(crate) struct FileOperation {
    shared: RefCell<Option<Arc<Shared>>>,
}

default_built_in_state!(FileOperation);

#[whim_function(
    "Whim\\_Private\\read_file(string $path, (0..) $offset, null|(1..) $maximumBytes): string",
    must_use
)]
pub(crate) fn read_file<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path =
        CString::new(arguments.bytes(0)).map_err(|_| system_error(cx, "open", libc::EINVAL))?;
    let offset = arguments.int(1).cast_unsigned();
    let maximum = arguments.optional_int(2).map(i64::cast_unsigned);
    let shared = submit(cx, move || read_path_bytes(&path, offset, maximum))?;
    wait_for(cx, &shared)?;
    match take_shared(cx, &shared)? {
        Some(FileResult::Bytes(bytes)) => Ok(Value::from_string_vec(cx.vm.heap(), bytes)),
        Some(_) | None => Err(cx.type_error("the file operation does not contain bytes")),
    }
}

#[whim_methods]
impl FileOperation {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "read(Whim\\OS\\FileDescriptor $descriptor, (1..) $maximumBytes): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn read<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor = duplicate_for_operation(cx, arguments, "read")?;
        let maximum = usize::try_from(arguments.int(1))
            .map_err(|_| system_error(cx, "read", libc::EOVERFLOW))?;
        start(cx, move || {
            let mut bytes = Vec::<u8>::with_capacity(maximum);
            let (initialized, _) = unix_io::read(&descriptor, bytes.spare_capacity_mut())
                .map_err(|error| rustix_file_error("read", error))?;
            let count = initialized.len();
            // SAFETY: `read` initialized exactly `count` bytes within capacity.
            unsafe { bytes.set_len(count) };
            Ok(FileResult::Bytes(bytes))
        })
    }

    #[whim_method(
        "readPath(string $path, (0..) $offset, null|(1..) $maximumBytes): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn read_path<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let path =
            CString::new(arguments.bytes(0)).map_err(|_| system_error(cx, "open", libc::EINVAL))?;
        let offset = arguments.int(1).cast_unsigned();
        let maximum = arguments.optional_int(2).map(i64::cast_unsigned);
        start(cx, move || read_path_bytes(&path, offset, maximum))
    }

    #[whim_method(
        "write(Whim\\OS\\FileDescriptor $descriptor, string $bytes): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn write<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor = duplicate_for_operation(cx, arguments, "write")?;
        let bytes = arguments.bytes(1).to_vec();
        start(cx, move || {
            let count = unix_io::write(&descriptor, &bytes)
                .map_err(|error| rustix_file_error("write", error))?;
            Ok(FileResult::Integer(
                i64::try_from(count).unwrap_or(i64::MAX),
            ))
        })
    }

    #[whim_method(
        "synchronize(Whim\\OS\\FileDescriptor $descriptor): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn synchronize<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor = duplicate_for_operation(cx, arguments, "fsync")?;
        start(cx, move || {
            fs::fsync(&descriptor).map_err(|error| rustix_file_error("fsync", error))?;
            Ok(FileResult::Boolean(true))
        })
    }

    #[whim_method(
        "truncate(Whim\\OS\\FileDescriptor $descriptor, (0..) $length): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn truncate<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor = duplicate_for_operation(cx, arguments, "ftruncate")?;
        let length = arguments.int(1).cast_unsigned();
        start(cx, move || {
            fs::ftruncate(&descriptor, length)
                .map_err(|error| rustix_file_error("ftruncate", error))?;
            Ok(FileResult::Boolean(true))
        })
    }

    #[whim_method(
        "metadata(Whim\\OS\\FileDescriptor $descriptor): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn metadata<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor = duplicate_for_operation(cx, arguments, "fstat")?;
        start(cx, move || {
            let metadata =
                fs::fstat(&descriptor).map_err(|error| rustix_file_error("fstat", error))?;
            Ok(FileResult::Metadata(metadata_values(&metadata)))
        })
    }

    #[whim_method(
        "pathMetadata(string $path, bool $follow): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn path_metadata<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let path =
            CString::new(arguments.bytes(0)).map_err(|_| system_error(cx, "stat", libc::EINVAL))?;
        let follow = arguments.bool(1);
        start(cx, move || {
            let flags = if follow {
                fs::AtFlags::empty()
            } else {
                fs::AtFlags::SYMLINK_NOFOLLOW
            };
            let call = if follow { "stat" } else { "lstat" };
            let metadata = fs::statat(fs::CWD, &path, flags)
                .map_err(|error| rustix_file_error(call, error))?;
            Ok(FileResult::Metadata(metadata_values(&metadata)))
        })
    }

    #[whim_method(
        "open(string $path, int $flags, int $mode): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn open<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let path =
            CString::new(arguments.bytes(0)).map_err(|_| system_error(cx, "open", libc::EINVAL))?;
        let flags =
            i32::try_from(arguments.int(1)).map_err(|_| system_error(cx, "open", libc::EINVAL))?;
        let mode = libc::mode_t::try_from(arguments.int(2))
            .map_err(|_| system_error(cx, "open", libc::EINVAL))?;
        start(cx, move || {
            let flags = fs::OFlags::from_bits_retain(flags.cast_unsigned()) | fs::OFlags::CLOEXEC;
            let mode = fs::Mode::from_raw_mode(mode);
            let descriptor = fs::openat(fs::CWD, &path, flags, mode)
                .map_err(|error| rustix_file_error("open", error))?;
            Ok(FileResult::Descriptor(descriptor))
        })
    }

    #[whim_method(
        "temporary(string $directory): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn temporary<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let directory = arguments.bytes(0).to_vec();
        if directory.is_empty() || directory.contains(&0) {
            return Err(system_error(cx, "mkstemp", libc::EINVAL));
        }

        start(cx, move || {
            let mut template = temporary_template(&directory, b"whim-spool-");
            // SAFETY: the template is writable, null-terminated, and lives through the call.
            let raw = unsafe { libc::mkstemp(template.as_mut_ptr().cast()) };
            if raw < 0 {
                return Err(file_error("mkstemp"));
            }

            // SAFETY: `mkstemp` returned a new descriptor that this value now owns.
            let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };
            // SAFETY: the surrounding invariant proves this result is successful.
            let path = unsafe {
                unwrap_result_invariant(
                    CStr::from_bytes_with_nul(&template),
                    "a temporary file template has one trailing null byte",
                )
            };
            if let Err(error) = fs::fchmod(&descriptor, fs::Mode::from_raw_mode(0o600)) {
                let error = rustix_file_error("fchmod", error);
                _ = fs::unlinkat(fs::CWD, path, fs::AtFlags::empty());
                return Err(error);
            }

            if let Err(errno) = set_close_on_exec(raw) {
                _ = fs::unlinkat(fs::CWD, path, fs::AtFlags::empty());
                return Err(FileError {
                    call: "fcntl",
                    errno,
                });
            }

            fs::unlinkat(fs::CWD, path, fs::AtFlags::empty())
                .map_err(|error| rustix_file_error("unlink", error))?;

            Ok(FileResult::Descriptor(descriptor))
        })
    }

    #[whim_method(
        "seek(Whim\\OS\\FileDescriptor $descriptor, int $offset, int $origin): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn seek<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor = duplicate_for_operation(cx, arguments, "lseek")?;
        let offset = libc::off_t::try_from(arguments.int(1))
            .map_err(|_| system_error(cx, "lseek", libc::EOVERFLOW))?;
        let origin =
            i32::try_from(arguments.int(2)).map_err(|_| system_error(cx, "lseek", libc::EINVAL))?;
        start(cx, move || {
            // SAFETY: `descriptor` stays open for the call.
            let position = unsafe { libc::lseek(descriptor.as_raw_fd(), offset, origin) };
            if position < 0 {
                return Err(file_error("lseek"));
            }
            Ok(FileResult::Integer(position as i64))
        })
    }

    #[whim_method(
        "lock(Whim\\OS\\FileDescriptor $descriptor, int $kind, bool $wait): Whim\\_Private\\FileOperation",
        static,
        must_use
    )]
    fn lock<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let descriptor = duplicate_for_operation(cx, arguments, "flock")?;
        let mut kind =
            i32::try_from(arguments.int(1)).map_err(|_| system_error(cx, "flock", libc::EINVAL))?;
        let wait = arguments.bool(2);
        if !wait {
            kind |= libc::LOCK_NB;
        }
        let operation =
            flock_operation(kind).ok_or_else(|| system_error(cx, "flock", libc::EINVAL))?;
        start(cx, move || match fs::flock(&descriptor, operation) {
            Ok(()) => Ok(FileResult::Boolean(true)),
            Err(error) if !wait && would_block(error.raw_os_error()) => {
                Ok(FileResult::Boolean(false))
            }
            Err(error) => Err(rustix_file_error("flock", error)),
        })
    }

    #[whim_method("wait(): void")]
    fn wait(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let shared = operation(cx)?;
        wait_for(cx, &shared)?;
        Ok(Value::null())
    }

    #[whim_method("takeBytes(): null|string", must_use)]
    fn take_bytes(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Bytes(bytes)) => Ok(Value::from_string_vec(cx.vm.heap(), bytes)),
            None => Ok(Value::null()),
            Some(_) => Err(cx.type_error("the file operation does not contain bytes")),
        }
    }

    #[whim_method("takeInt(): null|int", must_use)]
    fn take_int(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Integer(value)) => Ok(Value::int(value)),
            None => Ok(Value::null()),
            Some(_) => Err(cx.type_error("the file operation does not contain an integer")),
        }
    }

    #[whim_method("takeBool(): null|bool", must_use)]
    fn take_bool(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Boolean(value)) => Ok(Value::bool(value)),
            None => Ok(Value::null()),
            Some(_) => Err(cx.type_error("the file operation does not contain a boolean")),
        }
    }

    #[whim_method("takeMetadata(): null|vec<int>", must_use)]
    fn take_metadata(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Metadata(values)) => Ok(cx.vec(values.into_iter().map(Value::int))),
            None => Ok(Value::null()),
            Some(_) => Err(cx.type_error("the file operation does not contain metadata")),
        }
    }

    #[whim_method("takeDescriptor(): null|Whim\\OS\\FileDescriptor", must_use)]
    fn take_descriptor(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Descriptor(descriptor)) => {
                build_file_descriptor(cx, Descriptor::Raw(descriptor))
            }
            None => Ok(Value::null()),
            Some(_) => Err(cx.type_error("the file operation does not contain a descriptor")),
        }
    }

    #[whim_method("cancel(): void", no_track_caller, no_trace_boundary)]
    fn cancel(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        operation(cx)?.cancel();
        Ok(Value::null())
    }
}

fn operation(cx: &mut Context<'_, '_, '_>) -> Result<Arc<Shared>, Throw> {
    let operation = cx.state::<FileOperation>()?.shared.borrow().clone();
    operation.ok_or_else(|| cx.type_error("the file operation is not initialized"))
}

fn take(cx: &mut Context<'_, '_, '_>) -> Result<Option<FileResult>, Throw> {
    let shared = operation(cx)?;
    take_shared(cx, &shared)
}

fn take_shared(cx: &mut Context<'_, '_, '_>, shared: &Shared) -> Result<Option<FileResult>, Throw> {
    match shared.take() {
        Some(Ok(result)) => Ok(result),
        Some(Err(error)) => Err(system_error(cx, error.call, error.errno)),
        None => Ok(None),
    }
}

fn duplicate_for_operation(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    call: &'static str,
) -> Result<OwnedFd, Throw> {
    let descriptor = arguments.local(0);
    let descriptor = descriptor_of(cx, &descriptor, call)?;
    duplicate_raw(descriptor).map_err(|errno| system_error(cx, "fcntl", errno))
}

fn read_path_bytes(
    path: &CStr,
    offset: u64,
    maximum: Option<u64>,
) -> Result<FileResult, FileError> {
    let flags = fs::OFlags::RDONLY | fs::OFlags::CLOEXEC;
    let descriptor = fs::openat(fs::CWD, path, flags, fs::Mode::empty())
        .map_err(|error| rustix_file_error("open", error))?;
    let metadata = fs::fstat(&descriptor).map_err(|error| rustix_file_error("fstat", error))?;
    if fs::FileType::from_raw_mode(metadata.st_mode) != fs::FileType::RegularFile {
        return Err(FileError {
            call: "fstat",
            errno: libc::EISDIR,
        });
    }

    let mut file = File::from(descriptor);
    if offset != 0 {
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| standard_file_error("lseek", &error))?;
    }

    let size = u64::try_from(metadata.st_size).unwrap_or(0);
    let available = size.saturating_sub(offset);
    let expected = maximum.map_or(available, |maximum| available.min(maximum));
    let capacity = usize::try_from(expected).unwrap_or(usize::MAX);
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|_| FileError {
        call: "read",
        errno: libc::ENOMEM,
    })?;

    match maximum {
        Some(maximum) => file
            .take(maximum)
            .read_to_end(&mut bytes)
            .map_err(|error| standard_file_error("read", &error))?,
        None => file
            .read_to_end(&mut bytes)
            .map_err(|error| standard_file_error("read", &error))?,
    };

    Ok(FileResult::Bytes(bytes))
}

fn start(
    cx: &mut Context<'_, '_, '_>,
    operation: impl FnOnce() -> Result<FileResult, FileError> + Send + 'static,
) -> Result<Value, Throw> {
    let shared = submit(cx, operation)?;
    let object = cx.new_built_in_instance(FILE_OPERATION)?;
    let Some(state) = state_ref::<FileOperation>(&object) else {
        return Err(cx.type_error("the file operation has no built-in state"));
    };

    *state.shared.borrow_mut() = Some(shared);
    Ok(object)
}

fn submit(
    cx: &mut Context<'_, '_, '_>,
    operation: impl FnOnce() -> Result<FileResult, FileError> + Send + 'static,
) -> Result<Arc<Shared>, Throw> {
    let shared = Shared::new().map_err(|error| {
        system_error(cx, "socketpair", error.raw_os_error().unwrap_or(libc::EIO))
    })?;
    let worker = Arc::clone(&shared);
    cx.vm
        .engine
        .blocking
        .submit(Box::new(move || worker.complete(operation())))
        .map_err(|error| {
            system_error(cx, "thread", error.raw_os_error().unwrap_or(libc::EAGAIN))
        })?;

    Ok(shared)
}

fn wait_for(cx: &mut Context<'_, '_, '_>, shared: &Shared) -> Result<(), Throw> {
    while !shared.is_complete() {
        if cx.vm.loop_has_tasks() {
            cx.io_wait_until_readable(shared.descriptor())?;
        } else {
            shared.wait().map_err(|error| {
                system_error(cx, "poll", error.raw_os_error().unwrap_or(libc::EIO))
            })?;
        }
        shared.drain();
    }
    shared.drain();
    Ok(())
}

fn file_error(call: &'static str) -> FileError {
    FileError {
        call,
        errno: last_errno(),
    }
}

const fn rustix_file_error(call: &'static str, error: Errno) -> FileError {
    FileError {
        call,
        errno: error.raw_os_error(),
    }
}

fn standard_file_error(call: &'static str, error: &Error) -> FileError {
    FileError {
        call,
        errno: error.raw_os_error().unwrap_or(libc::EIO),
    }
}

const fn would_block(errno: i32) -> bool {
    errno == libc::EAGAIN || errno == libc::EWOULDBLOCK
}

const fn flock_operation(kind: i32) -> Option<fs::FlockOperation> {
    match kind {
        libc::LOCK_SH => Some(fs::FlockOperation::LockShared),
        libc::LOCK_EX => Some(fs::FlockOperation::LockExclusive),
        libc::LOCK_UN => Some(fs::FlockOperation::Unlock),
        value if value == libc::LOCK_SH | libc::LOCK_NB => {
            Some(fs::FlockOperation::NonBlockingLockShared)
        }
        value if value == libc::LOCK_EX | libc::LOCK_NB => {
            Some(fs::FlockOperation::NonBlockingLockExclusive)
        }
        value if value == libc::LOCK_UN | libc::LOCK_NB => {
            Some(fs::FlockOperation::NonBlockingUnlock)
        }
        _ => None,
    }
}

fn metadata_values(metadata: &fs::Stat) -> [i64; 15] {
    [
        i128::from(metadata.st_mode),
        i128::from(metadata.st_nlink),
        i128::from(metadata.st_uid),
        i128::from(metadata.st_gid),
        i128::from(metadata.st_size),
        i128::from(metadata.st_blksize),
        i128::from(metadata.st_blocks),
        i128::from(metadata.st_dev),
        i128::from(metadata.st_ino),
        i128::from(metadata.st_atime),
        i128::from(metadata.st_atime_nsec),
        i128::from(metadata.st_mtime),
        i128::from(metadata.st_mtime_nsec),
        i128::from(metadata.st_ctime),
        i128::from(metadata.st_ctime_nsec),
    ]
    .map(saturating_i64)
}

fn saturating_i64(value: i128) -> i64 {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            i64::try_from(value.clamp(i128::from(i64::MIN), i128::from(i64::MAX))),
            "an i128 clamped to the i64 range fits i64",
        )
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;

    use crate::blocking::BlockingPool;

    #[test]
    fn blocking_jobs_run_away_from_the_caller() -> Result<(), Box<dyn Error>> {
        let pool = BlockingPool::new();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        pool.submit(Box::new(move || {
            thread::sleep(Duration::from_millis(20));
            worker_finished.store(true, Ordering::Release);
        }))?;

        assert!(!finished.load(Ordering::Acquire));
        while !finished.load(Ordering::Acquire) {
            thread::yield_now();
        }
        Ok(())
    }
}
