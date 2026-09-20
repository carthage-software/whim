use std::path::PathBuf;

use whim_macros::whim_function;
use whim_sys::file::Timestamp;
use whim_sys::filesystem as fs;
use whim_sys::path::{path_bytes, path_from_bytes};

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::private::file::metadata_value;
use crate::core::private::syscall::{Descriptor, build_file_descriptor, io_error, with_descriptor};
use crate::value::Value;

pub(crate) fn path(
    cx: &mut Context<'_, '_, '_>,
    bytes: &[u8],
    call: &'static str,
) -> Result<PathBuf, Throw> {
    if bytes.contains(&0) {
        return Err(io_error(cx, whim_sys::Error::invalid(call)));
    }
    path_from_bytes(bytes).map_err(|error| io_error(cx, whim_sys::Error::new(call, error)))
}

#[whim_function(
    "Whim\\_Private\\open_directory_descriptor((string&!'') $path): null|Whim\\OS\\FileDescriptor"
)]
pub(crate) fn open_directory_descriptor<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let Ok(path) = path_from_bytes(arguments.bytes(0)) else {
        return Ok(Value::null());
    };
    fs::open_directory(&path).map_or_else(
        || Ok(Value::null()),
        |descriptor| build_file_descriptor(cx, descriptor),
    )
}

#[whim_function(
    "Whim\\_Private\\open_regular_file_beneath(Whim\\OS\\FileDescriptor $directory, string $path): null|(Whim\\OS\\FileDescriptor, vec<int>)"
)]
pub(crate) fn open_regular_file_beneath<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let result = with_descriptor(cx, &arguments.local(0), "openat", |descriptor| {
        fs::open_regular_file_beneath(descriptor, arguments.bytes(1))
    })?;
    let Some((descriptor, metadata)) = result else {
        return Ok(Value::null());
    };
    let descriptor = build_file_descriptor(cx, descriptor)?;
    Ok(cx.tuple([descriptor, metadata_value(cx, &metadata)]))
}

#[whim_function("Whim\\_Private\\path_exists((string&!'') $path, bool $follow): bool")]
pub(crate) fn path_exists<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "stat")?;
    fs::exists(&path, arguments.bool(1))
        .map(Value::bool)
        .map_err(|error| io_error(cx, error))
}

#[whim_function(
    "Whim\\_Private\\check_access((string&!'') $path, bool $read, bool $write, bool $execute, bool $effective): bool"
)]
pub(crate) fn check_access<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "access")?;
    fs::check_access(
        &path,
        arguments.bool(1),
        arguments.bool(2),
        arguments.bool(3),
        arguments.bool(4),
    )
    .map(Value::bool)
    .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\create_directory((string&!'') $path, 0..=4294967295 $mode): void")]
pub(crate) fn create_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "mkdir")?;
    fs::create_directory(&path, arguments.int(1)).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\remove_directory((string&!'') $path): void")]
pub(crate) fn remove_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "rmdir")?;
    fs::remove_directory(&path).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\remove_file((string&!'') $path): void")]
pub(crate) fn remove_file<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "unlink")?;
    fs::remove_file(&path).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\rename_path((string&!'') $from, (string&!'') $to): void")]
pub(crate) fn rename_path<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let from = path(cx, arguments.bytes(0), "rename")?;
    let to = path(cx, arguments.bytes(1), "rename")?;
    fs::rename(&from, &to).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\create_link((string&!'') $target, (string&!'') $path): void")]
pub(crate) fn create_link<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let target = path(cx, arguments.bytes(0), "link")?;
    let path = path(cx, arguments.bytes(1), "link")?;
    fs::hard_link(&target, &path).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\create_symbolic_link((string&!'') $target, (string&!'') $path): void"
)]
pub(crate) fn create_symbolic_link<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let target = path(cx, arguments.bytes(0), "symlink")?;
    let path = path(cx, arguments.bytes(1), "symlink")?;
    fs::symbolic_link(&target, &path).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\read_symbolic_link((string&!'') $path): (string&!'')")]
pub(crate) fn read_symbolic_link<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "readlink")?;
    let target = fs::read_link(&path).map_err(|error| io_error(cx, error))?;
    Ok(cx.string(&path_bytes(&target)))
}

#[whim_function("Whim\\_Private\\resolve_path((string&!'') $path): (string&!'')")]
pub(crate) fn resolve_path<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "realpath")?;
    let path = fs::canonicalize(&path).map_err(|error| io_error(cx, error))?;
    Ok(cx.string(&path_bytes(&path)))
}

#[whim_function(
    "Whim\\_Private\\create_named_pipe((string&!'') $path, 0..=4294967295 $mode): void"
)]
pub(crate) fn create_named_pipe<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "mkfifo")?;
    fs::create_named_pipe(&path, arguments.int(1)).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\set_path_mode((string&!'') $path, 0..=4294967295 $mode): void")]
pub(crate) fn set_path_mode<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "chmod")?;
    fs::set_mode(&path, arguments.int(1)).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\set_path_owner((string&!'') $path, int $user, int $group, bool $follow): void"
)]
pub(crate) fn set_path_owner<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "chown")?;
    fs::set_owner(&path, arguments.int(1), arguments.int(2), arguments.bool(3))
        .map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\_Private\\set_path_times((string&!'') $path, int $accessedSeconds, int $accessedNanoseconds, int $modifiedSeconds, int $modifiedNanoseconds, bool $follow): void"
)]
pub(crate) fn set_path_times<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "utimensat")?;
    let accessed = Timestamp {
        seconds: arguments.int(1),
        nanoseconds: arguments.int(2),
    };
    let modified = Timestamp {
        seconds: arguments.int(3),
        nanoseconds: arguments.int(4),
    };
    fs::set_times(&path, accessed, modified, arguments.bool(5))
        .map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\read_directory((string&!'') $path): vec<((string&!''), int)>")]
pub(crate) fn read_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "opendir")?;
    let entries = fs::read_directory(&path).map_err(|error| io_error(cx, error))?;
    Ok(cx.vec(
        entries
            .into_iter()
            .map(|entry| cx.tuple([cx.string(&entry.name), Value::int(entry.mode)])),
    ))
}

#[whim_function("Whim\\_Private\\filesystem_space((string&!'') $path): (int, int, int, int)")]
pub(crate) fn filesystem_space<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let path = path(cx, arguments.bytes(0), "statvfs")?;
    let space = fs::space(&path).map_err(|error| io_error(cx, error))?;
    Ok(cx.tuple(
        [space.block_size, space.blocks, space.free, space.available]
            .map(|value| Value::int(i64::try_from(value).unwrap_or(i64::MAX))),
    ))
}

#[whim_function("Whim\\_Private\\temporary_directory(): (string&!'')")]
pub(crate) fn temporary_directory(cx: &Context<'_, '_, '_>) -> Value {
    cx.string(&path_bytes(&fs::temporary_directory()))
}

#[whim_function(
    "Whim\\_Private\\create_temporary_file((string&!'') $directory, string $prefix, 0..=4294967295 $mode): (Whim\\OS\\FileDescriptor, (string&!''))"
)]
pub(crate) fn create_temporary_file<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let directory = path(cx, arguments.bytes(0), "mkstemp")?;
    let (file, path) = fs::create_temporary_file(&directory, arguments.bytes(1), arguments.int(2))
        .map_err(|error| io_error(cx, error))?;
    let descriptor = build_file_descriptor(cx, Descriptor::from_file(file))?;
    Ok(cx.tuple([descriptor, cx.string(&path_bytes(&path))]))
}

#[whim_function(
    "Whim\\_Private\\create_temporary_directory((string&!'') $directory, string $prefix): (string&!'')"
)]
pub(crate) fn create_temporary_directory<'call>(
    cx: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
) -> Result<Value, Throw> {
    let directory = path(cx, arguments.bytes(0), "mkdtemp")?;
    let path = fs::create_temporary_directory(&directory, arguments.bytes(1))
        .map_err(|error| io_error(cx, error))?;
    Ok(cx.string(&path_bytes(&path)))
}
