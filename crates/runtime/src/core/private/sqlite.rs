//! `SQLite` driver values exposed to `Whim\Database`.

use std::cell::RefCell;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use whim_macros::whim_class;
use whim_macros::whim_methods;
use whim_sqlite::Configuration;
use whim_sqlite::Connection as DriverConnection;
use whim_sqlite::Error as DriverError;
use whim_sqlite::Executor;
use whim_sqlite::Operation as DriverOperation;
use whim_sqlite::ResultSet as DriverResult;
use whim_sqlite::Value as DriverValue;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::symbols::SymbolKind;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::heap::handle::ManagedRef;
use crate::value::newtype::NewtypeId;
use crate::value::object::TypeEnvironmentId;
use crate::value::vec::VecObject;

const SQLITE_CONNECTION: &str = "Whim\\_Private\\SQLiteConnection";
const SQLITE_OPERATION: &str = "Whim\\_Private\\SQLiteOperation";
const SQLITE_RESULT: &str = "Whim\\_Private\\SQLiteResult";
const SQLITE_ERROR: &str = "Whim\\_Private\\SQLiteError";
const SQLITE_CONCURRENT_OPERATION_ERROR: &str = "Whim\\_Private\\SQLiteConcurrentOperationError";
const DATABASE_BLOB: &[u8] = b"Whim\\Database\\Blob";

type DriverConnectionState = (Arc<DriverConnection>, Arc<dyn Executor>);

fn sqlite_error(cx: &mut Context<'_, '_, '_>, error: &DriverError) -> Throw {
    let class = if error.is_concurrent_operation() {
        SQLITE_CONCURRENT_OPERATION_ERROR
    } else {
        SQLITE_ERROR
    };
    let class = cx.vm.intern(class.as_bytes());
    cx.vm
        .throw(class, error.message_text(), i64::from(error.code()))
}

fn operation(cx: &mut Context<'_, '_, '_>) -> Result<Arc<DriverOperation>, Throw> {
    let operation = cx.state::<SQLiteOperation>()?.operation.borrow().clone();
    operation.ok_or_else(|| cx.type_error("the SQLite operation is not initialized"))
}

fn connection(cx: &mut Context<'_, '_, '_>) -> Result<Arc<DriverConnection>, Throw> {
    let connection = cx
        .state::<SQLiteConnection>()?
        .connection
        .borrow()
        .as_ref()
        .map(|(connection, _)| Arc::clone(connection));
    connection.ok_or_else(|| cx.type_error("the SQLite connection is not initialized"))
}

fn result(cx: &mut Context<'_, '_, '_>) -> Result<Rc<DriverResult>, Throw> {
    let result = cx.state::<SQLiteResult>()?.result.borrow().clone();
    result.ok_or_else(|| cx.type_error("the SQLite result is not initialized"))
}

fn close_result(cx: &mut Context<'_, '_, '_>) -> Result<(), Throw> {
    let result = result(cx)?;
    result.close();
    while !result.is_retired() {
        cx.io_wait_until_readable(result.descriptor())?;
        result.drain_notification();
    }
    Ok(())
}

fn build_operation(
    cx: &mut Context<'_, '_, '_>,
    operation: DriverOperation,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(SQLITE_OPERATION)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<SQLiteOperation>(&object),
            "a SQLite operation has built-in state",
        )
    };
    *state.operation.borrow_mut() = Some(Arc::new(operation));
    Ok(object)
}

fn build_result(cx: &mut Context<'_, '_, '_>, result: DriverResult) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(SQLITE_RESULT)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<SQLiteResult>(&object),
            "a SQLite result has built-in state",
        )
    };

    *state.result.borrow_mut() = Some(Rc::new(result));
    Ok(object)
}

fn build_connection(
    cx: &mut Context<'_, '_, '_>,
    connection: DriverConnection,
    executor: Arc<dyn Executor>,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(SQLITE_CONNECTION)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<SQLiteConnection>(&object),
            "a SQLite connection has built-in state",
        )
    };
    *state.connection.borrow_mut() = Some((Arc::new(connection), executor));
    Ok(object)
}

fn value_is_blob(cx: &Context<'_, '_, '_>, value: &Value) -> bool {
    let Some(id) = value.newtype_id() else {
        return false;
    };
    let tagged = cx.vm.engine.tables.newtype_value(id);
    cx.vm.engine.tables.newtypes[tagged.declaration.0 as usize]
        .name
        .as_bytes()
        == DATABASE_BLOB
}

fn collect_parameters(
    cx: &Context<'_, '_, '_>,
    values: &ManagedRef<VecObject>,
) -> Vec<DriverValue> {
    values
        .iter()
        .map(|value| match value.transparent() {
            ValueView::Null => DriverValue::Null,
            ValueView::Bool(value) => DriverValue::Integer(i64::from(*value)),
            ValueView::Int(value) => DriverValue::Integer(*value),
            ValueView::Float(value) => DriverValue::Real(*value),
            ValueView::String(_) | ValueView::ShortString(_) if value_is_blob(cx, value) => {
                // SAFETY: the surrounding invariant proves this option contains a value.
                DriverValue::Blob(unsafe {
                    unwrap_option_invariant(
                        value.as_string_bytes().map(<[u8]>::to_vec),
                        "a database blob has string backing",
                    )
                })
            }
            // SAFETY: the surrounding invariant proves this option contains a value.
            ValueView::String(_) | ValueView::ShortString(_) => DriverValue::Text(unsafe {
                unwrap_option_invariant(
                    value.as_string_bytes().map(<[u8]>::to_vec),
                    "a database text value has string backing",
                )
            }),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe {
                unreachable_invariant("database parameters are validated before dispatch")
            },
        })
        .collect()
}

fn blob_value(cx: &mut Context<'_, '_, '_>, bytes: Vec<u8>) -> Value {
    let name = cx.vm.intern(DATABASE_BLOB);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let entry = unsafe {
        unwrap_option_invariant(
            cx.vm.engine.tables.symbols.get(&name).copied(),
            "the database blob newtype is declared",
        )
    };
    if entry.kind != SymbolKind::Newtype {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe {
            unreachable_invariant("the database blob symbol is a newtype");
        }
    }
    let tag = cx.vm.engine.tables.intern_newtype_value(
        NewtypeId(entry.index),
        TypeEnvironmentId::default(),
        None,
    );
    let value = Value::from_string_vec(cx.vm.heap(), bytes);
    Value::newtype(value, tag)
}

fn whim_value(cx: &mut Context<'_, '_, '_>, value: DriverValue) -> Value {
    match value {
        DriverValue::Null => Value::null(),
        DriverValue::Integer(value) => Value::int(value),
        DriverValue::Real(value) => Value::float(value),
        DriverValue::Text(value) => Value::from_string_vec(cx.vm.heap(), value),
        DriverValue::Blob(value) => blob_value(cx, value),
    }
}

#[whim_class("Whim\\_Private\\SQLiteError")]
#[whim_extends("Whim\\Unwind\\Error")]
pub(crate) struct SQLiteError;

#[whim_methods]
impl SQLiteError {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}
}

#[whim_class("Whim\\_Private\\SQLiteConcurrentOperationError", final)]
#[whim_extends("Whim\\_Private\\SQLiteError")]
pub(crate) struct SQLiteConcurrentOperationError;

#[whim_methods]
impl SQLiteConcurrentOperationError {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}
}

#[whim_class("Whim\\_Private\\SQLiteOperation", final)]
#[derive(Default)]
pub(crate) struct SQLiteOperation {
    operation: RefCell<Option<Arc<DriverOperation>>>,
}

default_built_in_state!(SQLiteOperation);

#[whim_methods]
impl SQLiteOperation {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("wait(): void")]
    fn wait(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let operation = operation(cx)?;
        loop {
            if let Some(response) = operation.poll() {
                operation.drain_notification();
                return response
                    .map(|()| Value::null())
                    .map_err(|error| sqlite_error(cx, &error));
            }
            cx.io_wait_until_readable(operation.descriptor())?;
            operation.drain_notification();
        }
    }

    #[whim_method("cancel(): void", no_track_caller, no_trace_boundary)]
    fn cancel(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        operation(cx)?.cancel();
        Ok(Value::null())
    }
}

#[whim_class("Whim\\_Private\\SQLiteConnection", final)]
#[derive(Default)]
pub(crate) struct SQLiteConnection {
    connection: RefCell<Option<DriverConnectionState>>,
}

default_built_in_state!(SQLiteConnection);

#[whim_methods]
impl SQLiteConnection {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "open(string $path, bool $readOnly, bool $create, bool $uri, (0..) $busyTimeoutMilliseconds, (0..) $statementCacheCapacity, bool $foreignKeys): (Whim\\_Private\\SQLiteConnection, Whim\\_Private\\SQLiteOperation)",
        static,
        must_use
    )]
    fn open(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let path = PathBuf::from(OsString::from_vec(arguments.bytes(0).to_vec()));
        let busy_timeout = u64::try_from(arguments.int(4))
            .map(Duration::from_millis)
            .map_err(|_| DriverError::message("invalid SQLite busy timeout"))
            .map_err(|error| sqlite_error(cx, &error))?;
        let statement_cache_capacity = usize::try_from(arguments.int(5))
            .map_err(|_| DriverError::message("invalid SQLite statement cache capacity"))
            .map_err(|error| sqlite_error(cx, &error))?;
        let configuration = Configuration {
            path,
            read_only: arguments.bool(1),
            create: arguments.bool(2),
            uri: arguments.bool(3),
            busy_timeout,
            statement_cache_capacity,
            foreign_keys: arguments.bool(6),
        };
        let executor = cx
            .vm
            .engine
            .blocking
            .sqlite_executor()
            .map_err(DriverError::from)
            .map_err(|error| sqlite_error(cx, &error))?;
        let (connection, operation) = DriverConnection::open(configuration, &executor)
            .map_err(|error| sqlite_error(cx, &error))?;
        let connection = build_connection(cx, connection, executor)?;
        let operation = build_operation(cx, operation)?;
        Ok(cx.tuple([connection, operation]))
    }

    #[whim_method("ping(): Whim\\_Private\\SQLiteOperation", must_use)]
    fn ping(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let operation = connection(cx)?
            .ping()
            .map_err(|error| sqlite_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method("prepare(string $sql): Whim\\_Private\\SQLiteOperation", must_use)]
    fn prepare(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let sql = String::from_utf8(arguments.bytes(0).to_vec())
            .map_err(|_| DriverError::message("the SQL for SQLite must be valid UTF-8"))
            .map_err(|error| sqlite_error(cx, &error))?;
        let operation = connection(cx)?
            .prepare(sql)
            .map_err(|error| sqlite_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method(
        "execute(string $sql, vec<null|bool|int|float|string|Whim\\Database\\Blob> $parameters): (Whim\\_Private\\SQLiteResult, Whim\\_Private\\SQLiteOperation)",
        must_use
    )]
    fn execute(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let sql = String::from_utf8(arguments.bytes(0).to_vec())
            .map_err(|_| DriverError::message("the SQL for SQLite must be valid UTF-8"))
            .map_err(|error| sqlite_error(cx, &error))?;
        let values = arguments.vec(1);
        let parameters = collect_parameters(cx, &values);
        let (result, operation) = connection(cx)?
            .execute(sql, parameters)
            .map_err(|error| sqlite_error(cx, &error))?;
        let result = build_result(cx, result)?;
        let operation = build_operation(cx, operation)?;
        Ok(cx.tuple([result, operation]))
    }

    #[whim_method(
        "begin(bool $readUncommitted, bool $readOnly): Whim\\_Private\\SQLiteOperation",
        must_use
    )]
    fn begin(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let operation = connection(cx)?
            .begin(arguments.bool(0), arguments.bool(1))
            .map_err(|error| sqlite_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method("commit(): Whim\\_Private\\SQLiteOperation", must_use)]
    fn commit(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let operation = connection(cx)?
            .commit()
            .map_err(|error| sqlite_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method("rollback(): Whim\\_Private\\SQLiteOperation", must_use)]
    fn rollback(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let operation = connection(cx)?
            .rollback()
            .map_err(|error| sqlite_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method("isReusable(): bool", no_track_caller, no_trace_boundary, must_use)]
    fn is_reusable(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        Ok(Value::bool(connection(cx)?.is_reusable()))
    }

    #[whim_method("isClosed(): bool", no_track_caller, no_trace_boundary, must_use)]
    fn is_closed(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        Ok(Value::bool(connection(cx)?.is_closed()))
    }

    #[whim_method("close(): void", no_track_caller, no_trace_boundary)]
    fn close(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        connection(cx)?.close();
        Ok(Value::null())
    }

    #[whim_method("__destruct(): void", no_track_caller, no_trace_boundary)]
    fn destruct(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        connection(cx)?.close();
        Ok(Value::null())
    }
}

#[whim_class("Whim\\_Private\\SQLiteResult", final)]
#[derive(Default)]
pub(crate) struct SQLiteResult {
    result: RefCell<Option<Rc<DriverResult>>>,
}

default_built_in_state!(SQLiteResult);

#[whim_methods]
impl SQLiteResult {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("columns(): vec<(string, null|string)>", must_use)]
    fn columns(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let metadata = unsafe {
            unwrap_option_invariant(
                result(cx)?.metadata(),
                "the SQLite result metadata is ready before it is exposed",
            )
        };
        let columns = metadata
            .columns
            .into_iter()
            .map(|column| {
                let name = Value::from_string_vec(cx.vm.heap(), column.name);
                let type_name = column.declared_type.map_or_else(Value::null, |name| {
                    Value::from_string_vec(cx.vm.heap(), name)
                });
                cx.tuple([name, type_name])
            })
            .collect::<Vec<_>>();
        Ok(cx.vec(columns))
    }

    #[whim_method("affectedRows(): null|(0..)", must_use)]
    fn affected_rows(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let metadata = unsafe {
            unwrap_option_invariant(
                result(cx)?.metadata(),
                "the SQLite result metadata is ready before it is exposed",
            )
        };
        Ok(metadata.affected_rows.map_or_else(Value::null, |rows| {
            Value::int(i64::try_from(rows).unwrap_or(i64::MAX))
        }))
    }

    #[whim_method("fetch(): null|vec<mixed>", must_use)]
    fn fetch(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let result = result(cx)?;
        loop {
            if let Some(next) = result.poll_row() {
                return match next {
                    Ok(Some(row)) => {
                        let row = row
                            .into_iter()
                            .map(|value| whim_value(cx, value))
                            .collect::<Vec<_>>();
                        Ok(cx.vec(row))
                    }
                    Ok(None) => Ok(Value::null()),
                    Err(error) => Err(sqlite_error(cx, &error)),
                };
            }

            cx.io_wait_until_readable(result.descriptor())?;
            result.drain_notification();
        }
    }

    #[whim_method("interrupt(): void", no_track_caller, no_trace_boundary)]
    fn interrupt(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        result(cx)?.interrupt();
        Ok(Value::null())
    }

    #[whim_method("isClosed(): bool", no_track_caller, no_trace_boundary, must_use)]
    fn is_closed(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        Ok(Value::bool(result(cx)?.is_closed()))
    }

    #[whim_method("close(): void")]
    fn close(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        close_result(cx)?;
        Ok(Value::null())
    }

    #[whim_method("__destruct(): void", no_track_caller, no_trace_boundary)]
    fn destruct(cx: &mut Context<'_, '_, '_>) -> Value {
        // Cleanup errors cannot escape a destructor.
        drop(close_result(cx));
        Value::null()
    }
}
