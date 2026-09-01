//! Unix path and filesystem primitives.

use std::env::temp_dir;
use std::ffi::CStr;
use std::ffi::CString;
use std::os::fd::AsFd;
use std::os::fd::AsRawFd;
use std::os::fd::BorrowedFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::ptr::null_mut;

use rustix::fs;
use rustix::io::Errno;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::Descriptor;
use crate::core::private::syscall::build_file_descriptor;
use crate::core::private::syscall::descriptor_of;
use crate::core::private::syscall::last_system_error;
use crate::core::private::syscall::set_close_on_exec;
use crate::core::private::syscall::system_error;
use crate::value::Value;

fn path(cx: &mut Context<'_, '_, '_>, bytes: &[u8], call: &'static str) -> Result<CString, Throw> {
    CString::new(bytes).map_err(|_| system_error(cx, call, libc::EINVAL))
}

fn mode(
    cx: &mut Context<'_, '_, '_>,
    value: i64,
    call: &'static str,
) -> Result<libc::mode_t, Throw> {
    libc::mode_t::try_from(value).map_err(|_| system_error(cx, call, libc::EINVAL))
}

fn rustix_result<T>(
    cx: &mut Context<'_, '_, '_>,
    call: &'static str,
    result: Result<T, Errno>,
) -> Result<T, Throw> {
    result.map_err(|error| system_error(cx, call, error.raw_os_error()))
}

fn signed_value(value: i128) -> Value {
    let value = i64::try_from(value).unwrap_or_else(|_| {
        if value.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    });
    Value::int(value)
}

fn unsigned_value(value: u128) -> Value {
    Value::int(i64::try_from(value).unwrap_or(i64::MAX))
}

fn metadata_value(cx: &Context<'_, '_, '_>, metadata: &fs::Stat) -> Value {
    let values = [
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
    ];
    cx.vec(values.into_iter().map(signed_value))
}

#[whim_function(
    "Whim\\_Private\\open_directory_descriptor((string&!'') $path): null|Whim\\OS\\FileDescriptor"
)]
pub(crate) fn open_directory_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let Ok(path) = CString::new(bytes) else {
        return Ok(Value::null());
    };
    let flags =
        fs::OFlags::RDONLY | fs::OFlags::DIRECTORY | fs::OFlags::NOFOLLOW | fs::OFlags::CLOEXEC;
    let Ok(descriptor) = fs::openat(fs::CWD, &path, flags, fs::Mode::empty()) else {
        return Ok(Value::null());
    };
    build_file_descriptor(cx, Descriptor::Raw(descriptor))
}

#[whim_function(
    "Whim\\_Private\\open_regular_file_beneath(Whim\\OS\\FileDescriptor $directory, string $path): null|(Whim\\OS\\FileDescriptor, vec<int>)"
)]
pub(crate) fn open_regular_file_beneath<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let directory = arguments.local(0);
    let root = descriptor_of(cx, &directory, "openat")?;
    // SAFETY: the handle owns this open descriptor.
    let root = unsafe { BorrowedFd::borrow_raw(root) };
    let bytes = arguments.bytes(1);
    if bytes.is_empty() || bytes[0] == b'/' {
        return Ok(Value::null());
    }

    let mut components = bytes.split(|byte| *byte == b'/');
    let Some(name) = components.next_back() else {
        return Ok(Value::null());
    };
    if name.is_empty() || name == b"." || name == b".." {
        return Ok(Value::null());
    }

    let mut opened_directory = None;
    let directory_flags =
        fs::OFlags::RDONLY | fs::OFlags::DIRECTORY | fs::OFlags::NOFOLLOW | fs::OFlags::CLOEXEC;
    for component in components {
        if component.is_empty() || component == b"." || component == b".." {
            return Ok(Value::null());
        }
        let Ok(component) = CString::new(component) else {
            return Ok(Value::null());
        };
        let parent = opened_directory.as_ref().map_or(root, OwnedFd::as_fd);
        let Ok(descriptor) = fs::openat(parent, &component, directory_flags, fs::Mode::empty())
        else {
            return Ok(Value::null());
        };
        opened_directory = Some(descriptor);
    }

    let Ok(name) = CString::new(name) else {
        return Ok(Value::null());
    };
    let parent = opened_directory.as_ref().map_or(root, OwnedFd::as_fd);
    let file_flags = fs::OFlags::RDONLY | fs::OFlags::NOFOLLOW | fs::OFlags::CLOEXEC;
    let Ok(descriptor) = fs::openat(parent, &name, file_flags, fs::Mode::empty()) else {
        return Ok(Value::null());
    };
    let Ok(metadata) = fs::fstat(&descriptor) else {
        return Ok(Value::null());
    };
    if fs::FileType::from_raw_mode(metadata.st_mode) != fs::FileType::RegularFile {
        return Ok(Value::null());
    }

    let metadata = metadata_value(cx, &metadata);
    let descriptor = build_file_descriptor(cx, Descriptor::Raw(descriptor))?;
    Ok(cx.tuple([descriptor, metadata]))
}

#[whim_function("Whim\\_Private\\path_exists((string&!'') $path, bool $follow): bool")]
pub(crate) fn path_exists<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "fstatat")?;
    let follow = arguments.bool(1);
    let flags = if follow {
        fs::AtFlags::empty()
    } else {
        fs::AtFlags::SYMLINK_NOFOLLOW
    };
    match fs::statat(fs::CWD, &path, flags) {
        Ok(_) => Ok(Value::bool(true)),
        Err(Errno::NOENT | Errno::NOTDIR) => Ok(Value::bool(false)),
        Err(error) => Err(system_error(cx, "fstatat", error.raw_os_error())),
    }
}

#[whim_function(
    "Whim\\_Private\\check_access((string&!'') $path, bool $read, bool $write, bool $execute, bool $effective): bool"
)]
pub(crate) fn check_access<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "faccessat")?;
    let mut access = fs::Access::EXISTS;
    if arguments.bool(1) {
        access |= fs::Access::READ_OK;
    }
    if arguments.bool(2) {
        access |= fs::Access::WRITE_OK;
    }
    if arguments.bool(3) {
        access |= fs::Access::EXEC_OK;
    }
    let flags = if arguments.bool(4) {
        fs::AtFlags::from_bits_retain(libc::AT_EACCESS.cast_unsigned())
    } else {
        fs::AtFlags::empty()
    };
    match fs::accessat(fs::CWD, &path, access, flags) {
        Ok(()) => Ok(Value::bool(true)),
        Err(Errno::ACCESS | Errno::PERM | Errno::NOENT | Errno::NOTDIR) => Ok(Value::bool(false)),
        Err(error) => Err(system_error(cx, "faccessat", error.raw_os_error())),
    }
}

#[whim_function("Whim\\_Private\\create_directory((string&!'') $path, 0..=4294967295 $mode): void")]
pub(crate) fn create_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "mkdir")?;
    let raw_mode = arguments.int(1);
    let mode = mode(cx, raw_mode, "mkdir")?;
    let mode = fs::Mode::from_raw_mode(mode);
    rustix_result(cx, "mkdir", fs::mkdirat(fs::CWD, &path, mode))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\remove_directory((string&!'') $path): void")]
pub(crate) fn remove_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "rmdir")?;
    rustix_result(
        cx,
        "rmdir",
        fs::unlinkat(fs::CWD, &path, fs::AtFlags::REMOVEDIR),
    )?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\remove_file((string&!'') $path): void")]
pub(crate) fn remove_file<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "unlink")?;
    rustix_result(
        cx,
        "unlink",
        fs::unlinkat(fs::CWD, &path, fs::AtFlags::empty()),
    )?;
    Ok(Value::null())
}

fn binary_path_call<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
    call: &'static str,
    operation: impl FnOnce(&CStr, &CStr) -> Result<(), Errno>,
) -> Result<Value, Throw> {
    let left = arguments.bytes(0);
    let left = path(cx, left, call)?;
    let right = arguments.bytes(1);
    let right = path(cx, right, call)?;
    rustix_result(cx, call, operation(&left, &right))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\rename_path((string&!'') $from, (string&!'') $to): void")]
pub(crate) fn rename_path<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    binary_path_call(cx, arguments, "rename", |from, to| {
        fs::renameat(fs::CWD, from, fs::CWD, to)
    })
}

#[whim_function("Whim\\_Private\\create_link((string&!'') $target, (string&!'') $path): void")]
pub(crate) fn create_link<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    binary_path_call(cx, arguments, "link", |target, path| {
        fs::linkat(fs::CWD, target, fs::CWD, path, fs::AtFlags::empty())
    })
}

#[whim_function(
    "Whim\\_Private\\create_symbolic_link((string&!'') $target, (string&!'') $path): void"
)]
pub(crate) fn create_symbolic_link<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    binary_path_call(cx, arguments, "symlink", |target, path| {
        fs::symlinkat(target, fs::CWD, path)
    })
}

#[whim_function("Whim\\_Private\\read_symbolic_link((string&!'') $path): (string&!'')")]
pub(crate) fn read_symbolic_link<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "readlink")?;
    let target = rustix_result(cx, "readlink", fs::readlinkat(fs::CWD, &path, Vec::new()))?;
    Ok(cx.string(target.to_bytes()))
}

#[whim_function("Whim\\_Private\\resolve_path((string&!'') $path): (string&!'')")]
pub(crate) fn resolve_path<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "realpath")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let resolved = unsafe { libc::realpath(path.as_ptr(), null_mut()) };
    if resolved.is_null() {
        return Err(last_system_error(cx, "realpath"));
    }
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let bytes = unsafe { CStr::from_ptr(resolved) }.to_bytes().to_vec();
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    unsafe { libc::free(resolved.cast()) };
    Ok(cx.string(&bytes))
}

#[whim_function(
    "Whim\\_Private\\create_named_pipe((string&!'') $path, 0..=4294967295 $mode): void"
)]
pub(crate) fn create_named_pipe<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "mkfifo")?;
    let raw_mode = arguments.int(1);
    let mode = mode(cx, raw_mode, "mkfifo")?;
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::mkfifo(path.as_ptr(), mode) } < 0 {
        return Err(last_system_error(cx, "mkfifo"));
    }
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\set_path_mode((string&!'') $path, 0..=4294967295 $mode): void")]
pub(crate) fn set_path_mode<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "chmod")?;
    let raw_mode = arguments.int(1);
    let mode = mode(cx, raw_mode, "chmod")?;
    let mode = fs::Mode::from_raw_mode(mode);
    rustix_result(
        cx,
        "chmod",
        fs::chmodat(fs::CWD, &path, mode, fs::AtFlags::empty()),
    )?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\set_path_owner((string&!'') $path, int $user, int $group, bool $follow): void"
)]
pub(crate) fn set_path_owner<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "fchownat")?;
    let user = arguments.int(1);
    let user = if user == -1 || user == i64::from(libc::uid_t::MAX) {
        None
    } else {
        let user =
            libc::uid_t::try_from(user).map_err(|_| system_error(cx, "fchownat", libc::EINVAL))?;
        Some(fs::Uid::from_raw(user))
    };
    let group = arguments.int(2);
    let group = if group == -1 || group == i64::from(libc::gid_t::MAX) {
        None
    } else {
        let group =
            libc::gid_t::try_from(group).map_err(|_| system_error(cx, "fchownat", libc::EINVAL))?;
        Some(fs::Gid::from_raw(group))
    };
    let flags = if arguments.bool(3) {
        fs::AtFlags::empty()
    } else {
        fs::AtFlags::SYMLINK_NOFOLLOW
    };
    rustix_result(
        cx,
        "fchownat",
        fs::chownat(fs::CWD, &path, user, group, flags),
    )?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\set_path_times((string&!'') $path, int $accessedSeconds, int $accessedNanoseconds, int $modifiedSeconds, int $modifiedNanoseconds, bool $follow): void"
)]
pub(crate) fn set_path_times<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "utimensat")?;
    let accessed_nanoseconds = fs::Nsecs::try_from(arguments.int(2))
        .map_err(|_| system_error(cx, "utimensat", libc::EINVAL))?;
    let modified_nanoseconds = fs::Nsecs::try_from(arguments.int(4))
        .map_err(|_| system_error(cx, "utimensat", libc::EINVAL))?;
    let times = fs::Timestamps {
        last_access: fs::Timespec {
            tv_sec: arguments.int(1),
            tv_nsec: accessed_nanoseconds,
        },
        last_modification: fs::Timespec {
            tv_sec: arguments.int(3),
            tv_nsec: modified_nanoseconds,
        },
    };
    let flags = if arguments.bool(5) {
        fs::AtFlags::empty()
    } else {
        fs::AtFlags::SYMLINK_NOFOLLOW
    };
    rustix_result(
        cx,
        "utimensat",
        fs::utimensat(fs::CWD, &path, &times, flags),
    )?;
    Ok(Value::null())
}

const fn directory_type(kind: fs::FileType) -> fs::RawMode {
    match kind {
        fs::FileType::Unknown => 0,
        _ => kind.as_raw_mode(),
    }
}

#[whim_function("Whim\\_Private\\read_directory((string&!'') $path): vec<((string&!''), int)>")]
pub(crate) fn read_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "opendir")?;
    let descriptor = rustix_result(
        cx,
        "opendir",
        fs::openat(
            fs::CWD,
            &path,
            fs::OFlags::RDONLY | fs::OFlags::DIRECTORY | fs::OFlags::CLOEXEC,
            fs::Mode::empty(),
        ),
    )?;
    let directory = rustix_result(cx, "opendir", fs::Dir::new(descriptor))?;

    let mut entries = Vec::new();
    for entry in directory {
        let entry = rustix_result(cx, "readdir", entry)?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." || name.is_empty() {
            continue;
        }
        let name = cx.string(name);
        let kind = Value::int(i64::from(directory_type(entry.file_type())));
        entries.push(cx.tuple([name, kind]));
    }
    Ok(cx.vec(entries))
}

#[whim_function("Whim\\_Private\\filesystem_space((string&!'') $path): (int, int, int, int)")]
pub(crate) fn filesystem_space<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let bytes = arguments.bytes(0);
    let path = path(cx, bytes, "statvfs")?;
    let info = rustix_result(cx, "statvfs", fs::statvfs(&path))?;
    let block = unsigned_value(u128::from(info.f_frsize));
    let blocks = unsigned_value(u128::from(info.f_blocks));
    let free = unsigned_value(u128::from(info.f_bfree));
    let available = unsigned_value(u128::from(info.f_bavail));
    Ok(cx.tuple([block, blocks, free, available]))
}

#[whim_function("Whim\\_Private\\temporary_directory(): (string&!'')")]
pub(crate) fn temporary_directory(cx: &Context<'_, '_, '_>) -> Value {
    let directory = temp_dir();
    cx.string(directory.as_os_str().as_bytes())
}

pub(crate) fn temporary_template(directory: &[u8], prefix: &[u8]) -> Vec<u8> {
    let mut template = Vec::with_capacity(directory.len() + prefix.len() + 8);
    template.extend_from_slice(directory);
    if !directory.ends_with(b"/") {
        template.push(b'/');
    }
    template.extend_from_slice(prefix);
    template.extend_from_slice(b"XXXXXX");
    template.push(0);
    template
}

fn valid_temporary_prefix(prefix: &[u8]) -> bool {
    !prefix.contains(&0) && !prefix.contains(&b'/')
}

#[whim_function(
    "Whim\\_Private\\create_temporary_file((string&!'') $directory, string $prefix, 0..=4294967295 $mode): (Whim\\OS\\FileDescriptor, (string&!''))"
)]
pub(crate) fn create_temporary_file<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let directory = arguments.bytes(0);
    let prefix = arguments.bytes(1);
    if directory.contains(&0) || !valid_temporary_prefix(prefix) {
        return Err(system_error(cx, "mkstemp", libc::EINVAL));
    }
    let raw_mode = arguments.int(2);
    let mode = mode(cx, raw_mode, "fchmod")?;
    let mut template = temporary_template(directory, prefix);
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let descriptor = unsafe { libc::mkstemp(template.as_mut_ptr().cast()) };
    if descriptor < 0 {
        return Err(last_system_error(cx, "mkstemp"));
    }
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let mode = fs::Mode::from_raw_mode(mode);
    rustix_result(cx, "fchmod", fs::fchmod(&descriptor, mode))?;
    set_close_on_exec(descriptor.as_raw_fd()).map_err(|errno| system_error(cx, "fcntl", errno))?;
    let length = template.len().saturating_sub(1);
    let path = cx.string(&template[..length]);
    let descriptor = build_file_descriptor(cx, Descriptor::Raw(descriptor))?;
    Ok(cx.tuple([descriptor, path]))
}

#[whim_function(
    "Whim\\_Private\\create_temporary_directory((string&!'') $directory, string $prefix): (string&!'')"
)]
pub(crate) fn create_temporary_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let directory = arguments.bytes(0);
    let prefix = arguments.bytes(1);
    if directory.contains(&0) || !valid_temporary_prefix(prefix) {
        return Err(system_error(cx, "mkdtemp", libc::EINVAL));
    }
    let mut template = temporary_template(directory, prefix);
    // SAFETY: the arguments follow the platform ABI; pointers and descriptors stay valid.
    if unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) }.is_null() {
        return Err(last_system_error(cx, "mkdtemp"));
    }
    let length = template.len().saturating_sub(1);
    Ok(cx.string(&template[..length]))
}

#[cfg(test)]
mod tests {
    use super::valid_temporary_prefix;

    #[test]
    fn temporary_prefixes_are_single_path_components() {
        assert!(valid_temporary_prefix(b""));
        assert!(valid_temporary_prefix(b"whim-"));
        assert!(!valid_temporary_prefix(b"../escaped-"));
        assert!(!valid_temporary_prefix(b"nested/prefix-"));
        assert!(!valid_temporary_prefix(b"nul\0prefix"));
    }
}
