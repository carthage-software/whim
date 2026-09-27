use std::ffi::OsString;
use std::io::ErrorKind;

use whim_macros::{whim_closure, whim_function};
use whim_sys::path::os_string_from_bytes;
use whim_sys::process::{self, Spawn, Stream};
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::async_::task::task_value;
use crate::core::private::syscall::descriptor::wait;
use crate::core::private::syscall::path::path;
use crate::core::private::syscall::{
    Descriptor, build_file_descriptor, io_error, system_error, with_descriptor,
};

#[whim_function("Whim\\Process\\get_parent_id(): uint", must_use)]
pub(crate) fn parent_process_id(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    process::parent_id()
        .map(|id| Value::uint(u64::from(id)))
        .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\process_user(): (uint, uint)")]
pub(crate) fn process_user(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let (real, effective) = process::user().map_err(|error| io_error(cx, error))?;
    Ok(cx.tuple([
        Value::uint(u64::from(real)),
        Value::uint(u64::from(effective)),
    ]))
}

#[whim_function("Whim\\_Private\\process_group(): (uint, uint)")]
pub(crate) fn process_group(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let (real, effective) = process::group().map_err(|error| io_error(cx, error))?;
    Ok(cx.tuple([
        Value::uint(u64::from(real)),
        Value::uint(u64::from(effective)),
    ]))
}

#[whim_function("Whim\\_Private\\process_supplementary_groups(): vec<uint>")]
pub(crate) fn process_supplementary_groups(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let groups = process::supplementary_groups().map_err(|error| io_error(cx, error))?;
    Ok(cx.vec(
        groups
            .into_iter()
            .map(|group| Value::uint(u64::from(group))),
    ))
}

#[whim_function("Whim\\_Private\\set_process_user(int $real, int $effective): void")]
pub(crate) fn set_process_user(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    process::set_user(arguments.int(0), arguments.int(1)).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\set_process_group(int $real, int $effective): void")]
pub(crate) fn set_process_group(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    process::set_group(arguments.int(0), arguments.int(1)).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\set_process_supplementary_groups(vec<uint> $groups): void")]
pub(crate) fn set_process_supplementary_groups(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let groups = arguments
        .vec(0)
        .iter()
        .map(|group| {
            // SAFETY: argument validation proves each group is an unsigned integer.
            let group = unsafe { group.as_uint_unchecked() };
            i64::try_from(group).map_err(|_| system_error(cx, "setgroups", libc::EINVAL))
        })
        .collect::<Result<Vec<_>, Throw>>()?;
    process::set_supplementary_groups(&groups).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\initialize_groups((string&!'') $user, uint $group): void")]
pub(crate) fn initialize_groups(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let group = i64::try_from(arguments.uint(1))
        .map_err(|_| system_error(cx, "initgroups", libc::EINVAL))?;
    process::initialize_groups(arguments.bytes(0), group).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\session_id(uint $process): uint")]
pub(crate) fn session_id(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process =
        i64::try_from(arguments.uint(0)).map_err(|_| system_error(cx, "getsid", libc::EINVAL))?;
    process::session_id(process)
        .map(|id| Value::uint(u64::from(id)))
        .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\start_session(): uint")]
pub(crate) fn start_session(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    process::start_session()
        .map(|id| Value::uint(u64::from(id)))
        .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\process_group_id(uint $process): uint")]
pub(crate) fn process_group_id(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process =
        i64::try_from(arguments.uint(0)).map_err(|_| system_error(cx, "getpgid", libc::EINVAL))?;
    process::group_id(process)
        .map(|id| Value::uint(u64::from(id)))
        .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\set_process_group_id(uint $process, uint $group): void")]
pub(crate) fn set_process_group_id(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process =
        i64::try_from(arguments.uint(0)).map_err(|_| system_error(cx, "setpgid", libc::EINVAL))?;
    let group =
        i64::try_from(arguments.uint(1)).map_err(|_| system_error(cx, "setpgid", libc::EINVAL))?;
    process::set_group_id(process, group).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\process_priority(uint $process): int")]
pub(crate) fn process_priority(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process = i64::try_from(arguments.uint(0))
        .map_err(|_| system_error(cx, "getpriority", libc::EINVAL))?;
    process::priority(process)
        .map(|value| Value::int(i64::from(value)))
        .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\set_process_priority(uint $process, int $priority): void")]
pub(crate) fn set_process_priority(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process = i64::try_from(arguments.uint(0))
        .map_err(|_| system_error(cx, "setpriority", libc::EINVAL))?;
    process::set_priority(process, arguments.int(1)).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\_Private\\resource_limit(int $resource): (int, int)")]
pub(crate) fn resource_limit(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let (soft, hard) =
        process::resource_limit(arguments.int(0)).map_err(|error| io_error(cx, error))?;
    Ok(cx.tuple([Value::int(soft), Value::int(hard)]))
}

#[whim_function("Whim\\_Private\\set_resource_limit(int $resource, int $soft, int $hard): void")]
pub(crate) fn set_resource_limit(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    process::set_resource_limit(arguments.int(0), arguments.int(1), arguments.int(2))
        .map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function(
    "Whim\\Filesystem\\exchange_creation_mask(0u..=511u $mask): 0u..=511u",
    must_use
)]
pub(crate) fn exchange_file_mode_mask(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    process::exchange_creation_mask(arguments.uint(0).cast_signed())
        .map(|mask| Value::uint(u64::from(mask)))
        .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\process_times(): (uint, uint, uint, uint)")]
pub(crate) fn process_times(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let times = cx
        .vm
        .engine
        .processes
        .times()
        .map_err(|error| io_error(cx, error))?;
    Ok(cx.tuple(times.map(|time| Value::uint(time.cast_unsigned()))))
}

#[whim_function(
    "Whim\\_Private\\replace_process((string&!'') $program, vec<string> $arguments, null|dict<(string&!''), string> $environment): never"
)]
pub(crate) fn replace_process(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let program = os_string(cx, arguments.bytes(0))?;
    let values = argument_strings(cx, arguments)?;
    let environment = environment(cx, &arguments.local(2))?;
    process::replace(&program, &values, environment.as_deref())
        .map_err(|error| io_error(cx, error))?;
    Err(cx.type_error("process replacement returned"))
}

#[whim_function("Whim\\_Private\\send_signal(int $process, (0..) $signal): void")]
pub(crate) fn send_signal(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    process::send_signal(arguments.int(0), arguments.int(1))
        .map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_function("Whim\\Process\\exists((1u..) $process): bool", must_use)]
pub(crate) fn process_exists(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process =
        i64::try_from(arguments.uint(0)).map_err(|_| system_error(cx, "kill", libc::EINVAL))?;
    process::exists(process)
        .map(Value::bool)
        .map_err(|error| io_error(cx, error))
}

#[whim_function("Whim\\_Private\\terminate_process((1u..) $process): void")]
pub(crate) fn terminate_process(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process =
        i64::try_from(arguments.uint(0)).map_err(|_| system_error(cx, "kill", libc::EINVAL))?;
    process::terminate(process).map_err(|error| io_error(cx, error))?;
    Ok(Value::null())
}

#[whim_closure("(): void")]
fn signal_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    loop {
        wait(cx, &descriptor, whim_sys::Interest::Readable)?;
        let bytes = with_descriptor(cx, &descriptor, "read", |descriptor| {
            match descriptor.read(1) {
                Err(whim_sys::Error::Io { source, .. })
                    if source.kind() == ErrorKind::Interrupted =>
                {
                    Ok(None)
                }
                result => result,
            }
        })?;
        match bytes {
            Some(bytes) if bytes.is_empty() => return Err(system_error(cx, "read", libc::EPIPE)),
            Some(_) => {
                cx.vm.call_function_value(&callback, &[])?;
            }
            None => {}
        }
    }
}

#[whim_function(
    "Whim\\_Private\\watch_signal((1..) $signal, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_signal(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let descriptor =
        process::watch_signal(arguments.int(0)).map_err(|error| io_error(cx, error))?;
    let descriptor = build_file_descriptor(cx, descriptor)?;
    let runner = cx.closure(signal_runner_spec(), &[descriptor, arguments.local(1)]);
    let task = cx.vm.loop_defer(runner)?;
    task_value(cx, task)
}

fn os_string(cx: &mut Context<'_, '_, '_>, bytes: &[u8]) -> Result<OsString, Throw> {
    if bytes.contains(&0) {
        return Err(system_error(cx, "spawn", libc::EINVAL));
    }
    os_string_from_bytes(bytes).map_err(|error| io_error(cx, whim_sys::Error::new("spawn", error)))
}

fn argument_strings(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Vec<OsString>, Throw> {
    arguments
        .vec(1)
        .iter()
        .map(|value| {
            let Some(bytes) = value.as_string_bytes() else {
                return Err(cx.type_error("process arguments must be strings"));
            };
            os_string(cx, bytes)
        })
        .collect()
}

fn environment(
    cx: &mut Context<'_, '_, '_>,
    value: &Value,
) -> Result<Option<Vec<(OsString, OsString)>>, Throw> {
    let Some(environment) = value.as_dict() else {
        return Ok(None);
    };
    environment
        .iter()
        .map(|(name, value)| {
            let name = name.to_value();
            let Some(name) = name.as_string_bytes() else {
                return Err(cx.type_error("environment names must be strings"));
            };
            let Some(value) = value.as_string_bytes() else {
                return Err(cx.type_error("environment values must be strings"));
            };
            Ok((os_string(cx, name)?, os_string(cx, value)?))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn stream(cx: &mut Context<'_, '_, '_>, value: &Value) -> Result<Stream, Throw> {
    let Some(elements) = value.as_tuple() else {
        return Err(cx.type_error("a process stream must be a tuple"));
    };
    match elements.get(0).and_then(Value::as_int) {
        Some(0) => Ok(Stream::Inherit),
        Some(1) => Ok(Stream::Null),
        Some(2) => Ok(Stream::Pipe),
        Some(4) => Ok(Stream::Terminal),
        Some(3) => {
            let Some(descriptor) = elements.get(1) else {
                return Err(cx.type_error("missing stream descriptor"));
            };
            with_descriptor(cx, descriptor, "spawn", Descriptor::try_clone_file).map(Stream::File)
        }
        _ => Err(cx.type_error("invalid stream disposition")),
    }
}

#[whim_function(
    "Whim\\_Private\\spawn_process((string&!'') $program, vec<string> $arguments, null|dict<(string&!''), string> $environment, null|(string&!'') $directory, vec<(int, null|Whim\\OS\\FileDescriptor)> $streams, vec<(Whim\\OS\\FileDescriptor, uint)> $inherited, int $processGroup): (uint, null|Whim\\OS\\FileDescriptor, null|Whim\\OS\\FileDescriptor, null|Whim\\OS\\FileDescriptor)"
)]
pub(crate) fn spawn_process(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let mappings = arguments.vec(5);
    let group = arguments.int(6);
    process::validate_spawn(!mappings.is_empty(), group >= 0)
        .map_err(|error| io_error(cx, error))?;
    let program = path(cx, arguments.bytes(0), "spawn")?;
    let values = argument_strings(cx, arguments)?;
    let environment = environment(cx, &arguments.local(2))?;
    let directory = arguments.local(3);
    let directory = directory
        .as_string_bytes()
        .map(|bytes| path(cx, bytes, "spawn"))
        .transpose()?;
    let streams = arguments.vec(4);
    let [input, output, error] = streams.as_slice() else {
        return Err(cx.type_error("three process streams are required"));
    };
    let streams = [stream(cx, input)?, stream(cx, output)?, stream(cx, error)?];
    let inherited = mappings
        .iter()
        .map(|mapping| {
            let Some(elements) = mapping.as_tuple() else {
                return Err(cx.type_error("an inherited descriptor mapping must be a tuple"));
            };
            let Some(source) = elements.get(0) else {
                return Err(cx.type_error("missing inherited descriptor"));
            };
            let Some(target) = elements.get(1).and_then(Value::as_uint) else {
                return Err(cx.type_error("missing inherited descriptor number"));
            };
            let target =
                i32::try_from(target).map_err(|_| system_error(cx, "spawn", libc::EINVAL))?;
            Ok((
                with_descriptor(cx, source, "spawn", Descriptor::try_clone_file)?,
                target,
            ))
        })
        .collect::<Result<Vec<_>, Throw>>()?;
    let group = if group < 0 {
        None
    } else {
        Some(i32::try_from(group).map_err(|_| system_error(cx, "spawn", libc::EINVAL))?)
    };
    let spawned = cx
        .vm
        .engine
        .processes
        .spawn(Spawn {
            program,
            arguments: values,
            environment,
            directory,
            streams,
            inherited,
            group,
        })
        .map_err(|error| io_error(cx, error))?;
    let input = child_descriptor(cx, spawned.input)?;
    let output = child_descriptor(cx, spawned.output)?;
    let error = child_descriptor(cx, spawned.error)?;
    Ok(cx.tuple([Value::uint(u64::from(spawned.id)), input, output, error]))
}

fn child_descriptor(
    cx: &mut Context<'_, '_, '_>,
    descriptor: Option<Descriptor>,
) -> Result<Value, Throw> {
    descriptor.map_or_else(
        || Ok(Value::null()),
        |descriptor| build_file_descriptor(cx, descriptor),
    )
}

#[whim_closure("(): void")]
fn process_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = cx.capture(0);
    let callback = cx.capture(1);
    let source = with_descriptor(cx, &descriptor, "wait", process::watch_descriptor)?;
    loop {
        cx.io_wait_until_readable(source)?;
        if let Some(exit) = with_descriptor(cx, &descriptor, "wait", process::read_exit)? {
            let status = cx.vm.engine.processes.record_exit(exit);
            return cx
                .vm
                .call_function_value(&callback, &[Value::uint(status.cast_unsigned())]);
        }
    }
}

#[whim_function(
    "Whim\\_Private\\watch_process(uint $process, (fn(uint): void) $callback): Whim\\_Private\\TaskId"
)]
pub(crate) fn watch_process(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let process =
        i64::try_from(arguments.uint(0)).map_err(|_| system_error(cx, "wait", libc::EINVAL))?;
    let descriptor = cx
        .vm
        .engine
        .processes
        .watch(process)
        .map_err(|error| io_error(cx, error))?;
    let descriptor = build_file_descriptor(cx, descriptor)?;
    let runner = cx.closure(process_runner_spec(), &[descriptor, arguments.local(1)]);
    let task = cx.vm.loop_defer(runner)?;
    task_value(cx, task)
}
