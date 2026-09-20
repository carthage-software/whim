use std::cell::RefCell;
use std::fs::File;
use std::sync::Arc;

use whim_macros::{whim_class, whim_function, whim_methods};
use whim_sys::file::{self, Metadata};
use whim_sys::operation::Operation;
use whim_value::Value;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::private::syscall::path::path;
use crate::core::private::syscall::{
    Descriptor, build_file_descriptor, io_error, system_error, with_descriptor,
};

enum FileResult {
    Bytes(Vec<u8>),
    Integer(i64),
    Boolean(bool),
    Metadata(Metadata),
    Descriptor(File),
}

type Shared = Operation<FileResult, whim_sys::Error>;

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
    let path = path(cx, arguments.bytes(0), "open")?;
    let offset = arguments.int(1).cast_unsigned();
    let maximum = arguments.optional_int(2).map(i64::cast_unsigned);
    let shared = submit(cx, move || {
        file::read_path(&path, offset, maximum).map(FileResult::Bytes)
    })?;
    wait_for(cx, &shared)?;
    match take_shared(cx, &shared)? {
        Some(FileResult::Bytes(bytes)) => Ok(Value::from_string_vec(cx.vm.heap(), bytes)),
        _ => Err(cx.type_error("the file operation does not contain bytes")),
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
        let descriptor = duplicate(cx, arguments, "read")?;
        let maximum = usize::try_from(arguments.int(1))
            .map_err(|_| system_error(cx, "read", libc::EOVERFLOW))?;
        start(cx, move || {
            file::read(&descriptor, maximum).map(FileResult::Bytes)
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
        let path = path(cx, arguments.bytes(0), "open")?;
        let offset = arguments.int(1).cast_unsigned();
        let maximum = arguments.optional_int(2).map(i64::cast_unsigned);
        start(cx, move || {
            file::read_path(&path, offset, maximum).map(FileResult::Bytes)
        })
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
        let descriptor = duplicate(cx, arguments, "write")?;
        let bytes = arguments.bytes(1).to_vec();
        start(cx, move || {
            file::write(&descriptor, &bytes)
                .map(|count| FileResult::Integer(i64::try_from(count).unwrap_or(i64::MAX)))
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
        let descriptor = duplicate(cx, arguments, "fsync")?;
        start(cx, move || {
            file::synchronize(&descriptor).map(|()| FileResult::Boolean(true))
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
        let descriptor = duplicate(cx, arguments, "ftruncate")?;
        let length = arguments.int(1).cast_unsigned();
        start(cx, move || {
            file::truncate(&descriptor, length).map(|()| FileResult::Boolean(true))
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
        let descriptor = duplicate(cx, arguments, "fstat")?;
        start(cx, move || {
            file::metadata(&descriptor).map(FileResult::Metadata)
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
        let path = path(cx, arguments.bytes(0), "stat")?;
        let follow = arguments.bool(1);
        start(cx, move || {
            file::path_metadata(&path, follow).map(FileResult::Metadata)
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
        let path = path(cx, arguments.bytes(0), "open")?;
        let (flags, mode) = (arguments.int(1), arguments.int(2));
        start(cx, move || {
            file::open(&path, flags, mode).map(FileResult::Descriptor)
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
        let directory = path(cx, arguments.bytes(0), "mkstemp")?;
        start(cx, move || {
            file::temporary(&directory).map(FileResult::Descriptor)
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
        let descriptor = duplicate(cx, arguments, "lseek")?;
        let (offset, origin) = (arguments.int(1), arguments.int(2));
        start(cx, move || {
            file::seek(&descriptor, offset, origin)
                .map(|position| FileResult::Integer(i64::try_from(position).unwrap_or(i64::MAX)))
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
        let descriptor =
            with_descriptor(cx, &arguments.local(0), "flock", Descriptor::file_for_lock)?;
        let (kind, wait) = (arguments.int(1), arguments.bool(2));
        start(cx, move || {
            file::lock(&descriptor, kind, wait).map(FileResult::Boolean)
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
            _ => Err(cx.type_error("the file operation does not contain bytes")),
        }
    }

    #[whim_method("takeInt(): null|int", must_use)]
    fn take_int(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Integer(value)) => Ok(Value::int(value)),
            None => Ok(Value::null()),
            _ => Err(cx.type_error("the file operation does not contain an integer")),
        }
    }

    #[whim_method("takeBool(): null|bool", must_use)]
    fn take_bool(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Boolean(value)) => Ok(Value::bool(value)),
            None => Ok(Value::null()),
            _ => Err(cx.type_error("the file operation does not contain a boolean")),
        }
    }

    #[whim_method("takeMetadata(): null|vec<int>", must_use)]
    fn take_metadata(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Metadata(metadata)) => Ok(metadata_value(cx, &metadata)),
            None => Ok(Value::null()),
            _ => Err(cx.type_error("the file operation does not contain metadata")),
        }
    }

    #[whim_method("takeDescriptor(): null|Whim\\OS\\FileDescriptor", must_use)]
    fn take_descriptor(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        match take(cx)? {
            Some(FileResult::Descriptor(file)) => {
                build_file_descriptor(cx, Descriptor::from_file(file))
            }
            None => Ok(Value::null()),
            _ => Err(cx.type_error("the file operation does not contain a descriptor")),
        }
    }

    #[whim_method("cancel(): void", no_track_caller, no_trace_boundary)]
    fn cancel(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        operation(cx)?.cancel();
        Ok(Value::null())
    }
}

fn duplicate(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    call: &'static str,
) -> Result<File, Throw> {
    with_descriptor(cx, &arguments.local(0), call, Descriptor::try_clone_file)
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
        Some(Err(error)) => Err(io_error(cx, error)),
        None => Ok(None),
    }
}

fn start(
    cx: &mut Context<'_, '_, '_>,
    operation: impl FnOnce() -> whim_sys::Result<FileResult> + Send + 'static,
) -> Result<Value, Throw> {
    let shared = submit(cx, operation)?;
    let object = cx.new_built_in_instance("Whim\\_Private\\FileOperation")?;
    let Some(state) = state_ref::<FileOperation>(&object) else {
        return Err(cx.type_error("the file operation has no built-in state"));
    };
    *state.shared.borrow_mut() = Some(shared);
    Ok(object)
}

fn submit(
    cx: &mut Context<'_, '_, '_>,
    operation: impl FnOnce() -> whim_sys::Result<FileResult> + Send + 'static,
) -> Result<Arc<Shared>, Throw> {
    let shared =
        Shared::new().map_err(|error| io_error(cx, whim_sys::Error::new("event", error)))?;
    let worker = Arc::clone(&shared);
    cx.vm
        .engine
        .blocking
        .submit(Box::new(move || worker.complete(operation())))
        .map_err(|error| io_error(cx, whim_sys::Error::new("thread", error)))?;
    Ok(shared)
}

fn wait_for(cx: &mut Context<'_, '_, '_>, shared: &Shared) -> Result<(), Throw> {
    while !shared.is_complete() {
        if cx.vm.loop_has_tasks() {
            cx.io_wait_until_readable(shared.descriptor())?;
        } else {
            shared
                .wait()
                .map_err(|error| io_error(cx, whim_sys::Error::new("wait", error)))?;
        }
        shared.drain();
    }
    shared.drain();
    Ok(())
}

pub(crate) fn metadata_value(cx: &Context<'_, '_, '_>, metadata: &Metadata) -> Value {
    cx.vec(
        [
            metadata.mode,
            metadata.links,
            metadata.user,
            metadata.group,
            metadata.size,
            metadata.block_size,
            metadata.blocks,
            metadata.device,
            metadata.inode,
            metadata.accessed.seconds,
            metadata.accessed.nanoseconds,
            metadata.modified.seconds,
            metadata.modified.nanoseconds,
            metadata.changed.seconds,
            metadata.changed.nanoseconds,
        ]
        .map(Value::int),
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
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
