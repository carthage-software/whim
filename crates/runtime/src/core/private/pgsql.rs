//! `PostgreSQL` driver values exposed to `Whim\Database`.

use std::cell::RefCell;
use std::rc::Rc;

use whim_macros::whim_class;
use whim_macros::whim_methods;
use whim_pgsql::Connection as DriverConnection;
use whim_pgsql::Error as DriverError;
use whim_pgsql::Isolation as DriverIsolation;
use whim_pgsql::Operation as DriverOperation;
use whim_pgsql::Parameter as DriverParameter;
use whim_pgsql::Progress;
use whim_pgsql::ResultSet as DriverResult;
use whim_pgsql::Statement as DriverStatement;
use whim_pgsql::Value as DriverValue;

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

const PGSQL_CONNECTION: &str = "Whim\\_Private\\PostgreSQLConnection";
const PGSQL_OPERATION: &str = "Whim\\_Private\\PostgreSQLOperation";
const PGSQL_RESULT: &str = "Whim\\_Private\\PostgreSQLResult";
const PGSQL_STATEMENT: &str = "Whim\\_Private\\PostgreSQLStatement";
const PGSQL_ERROR: &str = "Whim\\_Private\\PostgreSQLError";
const DATABASE_BLOB: &[u8] = b"Whim\\Database\\Blob";
const STATEMENT_RETIREMENT_BATCH: usize = 64;

fn pgsql_error(cx: &mut Context<'_, '_, '_>, error: &DriverError) -> Throw {
    let mut message = error.message_text().to_string();
    if let Some(detail) = error.detail() {
        message.push_str("; ");
        message.push_str(detail);
    }
    if let Some(hint) = error.hint() {
        message.push_str("; hint: ");
        message.push_str(hint);
    }
    let class = cx.vm.intern(PGSQL_ERROR.as_bytes());
    let sql_state = error
        .sqlstate()
        .map_or_else(Value::null, |sql_state| cx.string(sql_state.as_bytes()));
    let thrown = cx.vm.throw(class, &message, 0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let instance = unsafe {
        unwrap_option_invariant(
            thrown.0.as_object().cloned(),
            "a PostgreSQL driver failure is an error object",
        )
    };
    let class = instance.class();
    cx.vm
        .engine
        .write_error_slot(&instance, class, b"sqlState", sql_state);
    thrown
}

fn operation(cx: &mut Context<'_, '_, '_>) -> Result<Rc<DriverOperation>, Throw> {
    let operation = cx
        .state::<PostgreSQLOperation>()?
        .operation
        .borrow()
        .clone();
    operation.ok_or_else(|| cx.type_error("the PostgreSQL operation is not initialized"))
}

fn connection(cx: &mut Context<'_, '_, '_>) -> Result<DriverConnection, Throw> {
    let connection = cx
        .state::<PostgreSQLConnection>()?
        .connection
        .borrow()
        .clone();
    connection.ok_or_else(|| cx.type_error("the PostgreSQL connection is not initialized"))
}

fn result(cx: &mut Context<'_, '_, '_>) -> Result<Rc<DriverResult>, Throw> {
    let result = cx.state::<PostgreSQLResult>()?.result.borrow().clone();
    result.ok_or_else(|| cx.type_error("the PostgreSQL result is not initialized"))
}

fn statement(cx: &mut Context<'_, '_, '_>) -> Result<Rc<DriverStatement>, Throw> {
    let statement = cx
        .state::<PostgreSQLStatement>()?
        .statement
        .borrow()
        .clone();
    statement.ok_or_else(|| cx.type_error("the PostgreSQL statement is not initialized"))
}

fn wait(
    cx: &mut Context<'_, '_, '_>,
    poll: impl Fn() -> Result<Progress<()>, DriverError>,
) -> Result<Value, Throw> {
    loop {
        match poll().map_err(|error| pgsql_error(cx, &error))? {
            Progress::Ready(()) => return Ok(Value::null()),
            Progress::Yield => yield_to_scheduler(cx)?,
            Progress::Pending {
                descriptor,
                interest,
            } => cx.io_wait_until(descriptor, interest)?,
        }
    }
}

fn retire_statements(
    cx: &mut Context<'_, '_, '_>,
    connection: &DriverConnection,
    minimum: usize,
) -> Result<(), Throw> {
    let operation = connection
        .retire_statements(minimum)
        .map_err(|error| pgsql_error(cx, &error))?;
    if let Some(operation) = operation {
        _ = wait(cx, || operation.poll())?;
    }

    Ok(())
}

fn yield_to_scheduler(cx: &mut Context<'_, '_, '_>) -> Result<(), Throw> {
    if let Some(task) = cx.vm.loop_current_task() {
        cx.vm.loop_resume(task, Value::null());
        drop(cx.vm.loop_suspend()?);
    } else {
        _ = cx.vm.loop_run_once()?;
    }

    Ok(())
}

fn build_operation(
    cx: &mut Context<'_, '_, '_>,
    operation: DriverOperation,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(PGSQL_OPERATION)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<PostgreSQLOperation>(&object),
            "a PostgreSQL operation has built-in state",
        )
    };
    *state.operation.borrow_mut() = Some(Rc::new(operation));
    Ok(object)
}

fn build_connection(
    cx: &mut Context<'_, '_, '_>,
    connection: DriverConnection,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(PGSQL_CONNECTION)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<PostgreSQLConnection>(&object),
            "a PostgreSQL connection has built-in state",
        )
    };
    *state.connection.borrow_mut() = Some(connection);
    Ok(object)
}

fn build_result(cx: &mut Context<'_, '_, '_>, result: DriverResult) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(PGSQL_RESULT)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<PostgreSQLResult>(&object),
            "a PostgreSQL result has built-in state",
        )
    };
    *state.result.borrow_mut() = Some(Rc::new(result));
    Ok(object)
}

fn build_statement(
    cx: &mut Context<'_, '_, '_>,
    statement: DriverStatement,
) -> Result<Value, Throw> {
    let object = cx.new_built_in_instance(PGSQL_STATEMENT)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let state = unsafe {
        unwrap_option_invariant(
            state_ref::<PostgreSQLStatement>(&object),
            "a PostgreSQL statement has built-in state",
        )
    };
    *state.statement.borrow_mut() = Some(Rc::new(statement));
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

fn collect_parameters<'value>(
    cx: &Context<'_, '_, '_>,
    values: &'value ManagedRef<VecObject>,
) -> Vec<DriverParameter<'value>> {
    values
        .iter()
        .map(|value| match value.transparent() {
            ValueView::Null => DriverParameter::Null,
            ValueView::Bool(value) => DriverParameter::Boolean(*value),
            ValueView::Int(value) => DriverParameter::Integer(*value),
            ValueView::Float(value) => DriverParameter::Real(*value),
            ValueView::String(_) | ValueView::ShortString(_) if value_is_blob(cx, value) => {
                // SAFETY: the surrounding invariant proves this option contains a value.
                DriverParameter::Blob(unsafe {
                    unwrap_option_invariant(
                        value.as_string_bytes(),
                        "a database blob has string backing",
                    )
                })
            }
            // SAFETY: the surrounding invariant proves this option contains a value.
            ValueView::String(_) | ValueView::ShortString(_) => DriverParameter::Text(unsafe {
                unwrap_option_invariant(
                    value.as_string_bytes(),
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
        DriverValue::Boolean(value) => Value::bool(value),
        DriverValue::Integer(value) => Value::int(value),
        DriverValue::Real(value) => Value::float(value),
        DriverValue::Text(value) => Value::from_string_vec(cx.vm.heap(), value),
        DriverValue::Blob(value) => blob_value(cx, value),
    }
}

#[whim_class("Whim\\_Private\\PostgreSQLError", final)]
#[whim_extends("Whim\\Unwind\\Error")]
#[whim_property("private readonly null|string $sqlState")]
pub(crate) struct PostgreSQLError;

#[whim_methods]
impl PostgreSQLError {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("getSQLState(): null|string", no_track_caller, no_trace_boundary)]
    fn get_sql_state(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        cx.get_property(&this, "sqlState")
    }
}

#[whim_class("Whim\\_Private\\PostgreSQLOperation", final)]
#[derive(Default)]
pub(crate) struct PostgreSQLOperation {
    operation: RefCell<Option<Rc<DriverOperation>>>,
}

default_built_in_state!(PostgreSQLOperation);

#[whim_methods]
impl PostgreSQLOperation {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("wait(): void")]
    fn wait(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let operation = operation(cx)?;
        wait(cx, || operation.poll())
    }

    #[whim_method("cancel(): void", no_track_caller, no_trace_boundary)]
    fn cancel(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        operation(cx)?.cancel();
        Ok(Value::null())
    }
}

#[whim_class("Whim\\_Private\\PostgreSQLConnection", final)]
#[derive(Default)]
pub(crate) struct PostgreSQLConnection {
    connection: RefCell<Option<DriverConnection>>,
}

default_built_in_state!(PostgreSQLConnection);

#[whim_methods]
impl PostgreSQLConnection {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "open(#[SensitiveParameter] string $connectionString): (Whim\\_Private\\PostgreSQLConnection, Whim\\_Private\\PostgreSQLOperation)",
        static,
        must_use
    )]
    fn open(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let (connection, operation) =
            DriverConnection::open(arguments.bytes(0)).map_err(|error| pgsql_error(cx, &error))?;
        let connection = build_connection(cx, connection)?;
        let operation = build_operation(cx, operation)?;
        Ok(cx.tuple([connection, operation]))
    }

    #[whim_method("ping(): Whim\\_Private\\PostgreSQLOperation", must_use)]
    fn ping(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let connection = connection(cx)?;
        retire_statements(cx, &connection, 1)?;
        let operation = connection.ping().map_err(|error| pgsql_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method(
        "prepare(string $sql, bool $binaryResults): (Whim\\_Private\\PostgreSQLStatement, Whim\\_Private\\PostgreSQLOperation)",
        must_use
    )]
    fn prepare<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let connection = connection(cx)?;
        retire_statements(cx, &connection, STATEMENT_RETIREMENT_BATCH)?;
        let prepared = if arguments.bool(1) {
            connection.prepare_binary(arguments.bytes(0))
        } else {
            connection.prepare(arguments.bytes(0))
        };
        let (statement, operation) = prepared.map_err(|error| pgsql_error(cx, &error))?;
        let statement = build_statement(cx, statement)?;
        let operation = build_operation(cx, operation)?;
        Ok(cx.tuple([statement, operation]))
    }

    #[whim_method(
        "execute(string $sql, vec<null|bool|int|float|string|Whim\\Database\\Blob> $parameters, bool $binaryResults): (Whim\\_Private\\PostgreSQLResult, Whim\\_Private\\PostgreSQLOperation)",
        must_use
    )]
    fn execute<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let connection = connection(cx)?;
        retire_statements(cx, &connection, 1)?;
        let values = arguments.vec(1);
        let parameters = collect_parameters(cx, &values);
        let execution = if arguments.bool(2) {
            connection.execute_binary(arguments.bytes(0), &parameters)
        } else {
            connection.execute(arguments.bytes(0), &parameters)
        };
        let (result, operation) = execution.map_err(|error| pgsql_error(cx, &error))?;
        let result = build_result(cx, result)?;
        let operation = build_operation(cx, operation)?;
        Ok(cx.tuple([result, operation]))
    }

    #[whim_method(
        "begin(null|(0..=3) $isolation, bool $readOnly): Whim\\_Private\\PostgreSQLOperation",
        must_use
    )]
    fn begin<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let connection = connection(cx)?;
        retire_statements(cx, &connection, 1)?;
        let isolation = arguments.optional_int(0).map(|value| match value {
            0 => DriverIsolation::ReadUncommitted,
            1 => DriverIsolation::ReadCommitted,
            2 => DriverIsolation::RepeatableRead,
            3 => DriverIsolation::Serializable,
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe {
                unreachable_invariant("the PostgreSQL isolation is validated before dispatch")
            },
        });
        let operation = connection
            .begin(isolation, arguments.bool(1))
            .map_err(|error| pgsql_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method("commit(): Whim\\_Private\\PostgreSQLOperation", must_use)]
    fn commit(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let connection = connection(cx)?;
        retire_statements(cx, &connection, 1)?;
        let operation = connection
            .commit()
            .map_err(|error| pgsql_error(cx, &error))?;
        build_operation(cx, operation)
    }

    #[whim_method("rollback(): Whim\\_Private\\PostgreSQLOperation", must_use)]
    fn rollback(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let connection = connection(cx)?;
        retire_statements(cx, &connection, 1)?;
        let operation = connection
            .rollback()
            .map_err(|error| pgsql_error(cx, &error))?;
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

#[whim_class("Whim\\_Private\\PostgreSQLStatement", final)]
#[derive(Default)]
pub(crate) struct PostgreSQLStatement {
    statement: RefCell<Option<Rc<DriverStatement>>>,
}

default_built_in_state!(PostgreSQLStatement);

#[whim_methods]
impl PostgreSQLStatement {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "execute(vec<null|bool|int|float|string|Whim\\Database\\Blob> $parameters): (Whim\\_Private\\PostgreSQLResult, Whim\\_Private\\PostgreSQLOperation)",
        must_use
    )]
    fn execute<'call>(
        cx: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let statement = statement(cx)?;
        let connection = statement.connection();
        retire_statements(cx, &connection, 1)?;
        let values = arguments.vec(0);
        let parameters = collect_parameters(cx, &values);
        let (result, operation) = statement
            .execute(&parameters)
            .map_err(|error| pgsql_error(cx, &error))?;
        let result = build_result(cx, result)?;
        let operation = build_operation(cx, operation)?;
        Ok(cx.tuple([result, operation]))
    }

    #[whim_method("close(): void", no_track_caller, no_trace_boundary)]
    fn close(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let statement = cx.state::<Self>()?.statement.borrow_mut().take();
        if let Some(statement) = statement {
            drop(statement);
        }

        Ok(Value::null())
    }
}

#[whim_class("Whim\\_Private\\PostgreSQLResult", final)]
#[derive(Default)]
pub(crate) struct PostgreSQLResult {
    result: RefCell<Option<Rc<DriverResult>>>,
}

default_built_in_state!(PostgreSQLResult);

#[whim_methods]
impl PostgreSQLResult {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("columns(): vec<(string, null|string)>", must_use)]
    fn columns(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let metadata = unsafe {
            unwrap_option_invariant(
                result(cx)?.metadata(),
                "the PostgreSQL result metadata is ready before it is exposed",
            )
        };
        let columns = metadata
            .columns
            .into_iter()
            .map(|column| {
                let name = Value::from_string_vec(cx.vm.heap(), column.name);
                let type_name = column.type_name.map_or_else(Value::null, |name| {
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
                "the PostgreSQL result metadata is ready before it is exposed",
            )
        };
        Ok(metadata.affected_rows.map_or_else(Value::null, |rows| {
            Value::int(i64::try_from(rows).unwrap_or(i64::MAX))
        }))
    }

    #[whim_method("poll(): bool|null|vec<mixed>", must_use)]
    fn poll(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let result = result(cx)?;
        match result.poll_row().map_err(|error| pgsql_error(cx, &error))? {
            Progress::Ready(Some(row)) => {
                let row = row
                    .into_iter()
                    .map(|value| whim_value(cx, value))
                    .collect::<Vec<_>>();
                Ok(cx.vec(row))
            }
            Progress::Ready(None) => Ok(Value::null()),
            Progress::Yield | Progress::Pending { .. } => Ok(Value::bool(false)),
        }
    }

    #[whim_method("fetch(): null|vec<mixed>", must_use)]
    fn fetch(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let result = result(cx)?;
        loop {
            match result.poll_row().map_err(|error| pgsql_error(cx, &error))? {
                Progress::Ready(Some(row)) => {
                    let row = row
                        .into_iter()
                        .map(|value| whim_value(cx, value))
                        .collect::<Vec<_>>();
                    return Ok(cx.vec(row));
                }
                Progress::Ready(None) => return Ok(Value::null()),
                Progress::Yield => yield_to_scheduler(cx)?,
                Progress::Pending {
                    descriptor,
                    interest,
                } => cx.io_wait_until(descriptor, interest)?,
            }
        }
    }

    #[whim_method("fetchAll(): vec<vec<mixed>>", must_use)]
    fn fetch_all(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let result = result(cx)?;
        let mut rows = Vec::new();
        loop {
            match result.poll_row().map_err(|error| pgsql_error(cx, &error))? {
                Progress::Ready(Some(row)) => {
                    let row = row
                        .into_iter()
                        .map(|value| whim_value(cx, value))
                        .collect::<Vec<_>>();
                    rows.push(cx.vec(row));
                }
                Progress::Ready(None) => return Ok(cx.vec(rows)),
                Progress::Yield => yield_to_scheduler(cx)?,
                Progress::Pending {
                    descriptor,
                    interest,
                } => cx.io_wait_until(descriptor, interest)?,
            }
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
        let result = result(cx)?;
        wait(cx, || result.poll_close())
    }

    #[whim_method("__destruct(): void")]
    fn destruct(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let result = result(cx)?;
        // Cleanup errors cannot escape a destructor.
        drop(wait(cx, || result.poll_close()));
        Ok(Value::null())
    }
}
