//! A nonblocking libpq driver for the Whim runtime.

#![cfg(unix)]
#![deny(clippy::nursery, clippy::pedantic)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![forbid(unsafe_op_in_unsafe_fn)]

mod error;
mod value;

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::c_char;
use std::os::fd::BorrowedFd;
use std::os::fd::RawFd;
use std::ptr;
use std::ptr::NonNull;
use std::rc::Rc;
use std::slice;
use std::str;

use pq_sys::ConnStatusType;
use pq_sys::ExecStatusType;
use pq_sys::Oid;
use pq_sys::PG_DIAG_MESSAGE_DETAIL;
use pq_sys::PG_DIAG_MESSAGE_HINT;
use pq_sys::PG_DIAG_SQLSTATE;
use pq_sys::PGconn;
use pq_sys::PGresult;
use pq_sys::PQclear;
use pq_sys::PQcmdTuples;
use pq_sys::PQconnectPoll;
use pq_sys::PQconnectStart;
use pq_sys::PQconsumeInput;
use pq_sys::PQerrorMessage;
use pq_sys::PQfformat;
use pq_sys::PQfinish;
use pq_sys::PQflush;
use pq_sys::PQfname;
use pq_sys::PQftype;
use pq_sys::PQgetResult;
use pq_sys::PQgetisnull;
use pq_sys::PQgetlength;
use pq_sys::PQgetvalue;
use pq_sys::PQisBusy;
use pq_sys::PQnfields;
use pq_sys::PQntuples;
use pq_sys::PQresultErrorField;
use pq_sys::PQresultErrorMessage;
use pq_sys::PQresultStatus;
use pq_sys::PQsendPrepare;
use pq_sys::PQsendQuery;
use pq_sys::PQsendQueryParams;
use pq_sys::PQsendQueryPrepared;
use pq_sys::PQsetChunkedRowsMode;
use pq_sys::PQsetnonblocking;
use pq_sys::PQsocket;
use pq_sys::PQstatus;
use pq_sys::PostgresPollingStatusType;
use rustix::net;
use whim_loop::Interest;

pub use error::Error;
pub use value::Column;
pub use value::Metadata;
pub use value::Parameter;
pub use value::Row;
pub use value::Value;

const BOOL_OID: Oid = 16;
const BYTEA_OID: Oid = 17;
const CHAR_OID: Oid = 18;
const NAME_OID: Oid = 19;
const INT8_OID: Oid = 20;
const INT2_OID: Oid = 21;
const INT4_OID: Oid = 23;
const TEXT_OID: Oid = 25;
const JSON_OID: Oid = 114;
const FLOAT4_OID: Oid = 700;
const FLOAT8_OID: Oid = 701;
const BPCHAR_OID: Oid = 1_042;
const VARCHAR_OID: Oid = 1_043;
const JSONB_OID: Oid = 3_802;
const ROW_CHUNK_SIZE: i32 = 128;
const RESULT_CHUNK_POLL_BUDGET: usize = 8;

/// The state of a nonblocking operation.
pub enum Progress<T> {
    /// The operation completed.
    Ready(T),
    /// The operation made bounded progress and should be polled after yielding.
    Yield,
    /// The operation needs descriptor readiness.
    Pending {
        /// The libpq socket.
        descriptor: RawFd,
        /// The required readiness direction.
        interest: Interest,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Connecting,
    Idle,
    Busy,
    Closed,
}

struct ConnectionState {
    raw: Option<NonNull<PGconn>>,
    phase: Phase,
    next_statement: u64,
    pending_statements: Vec<CString>,
    cancelled: bool,
}

impl ConnectionState {
    fn raw(&self) -> Result<NonNull<PGconn>, Error> {
        self.raw
            .ok_or_else(|| Error::message("the PostgreSQL connection is closed"))
    }

    fn descriptor(&self) -> Result<RawFd, Error> {
        let raw = self.raw()?;
        // SAFETY: `raw` is a live libpq connection.
        let descriptor = unsafe { PQsocket(raw.as_ptr()) };
        if descriptor < 0 {
            return Err(connection_error(raw.as_ptr()));
        }
        Ok(descriptor)
    }

    fn close(&mut self) {
        self.phase = Phase::Closed;
        if let Some(raw) = self.raw.take() {
            // SAFETY: this state owns the live libpq connection.
            unsafe { PQfinish(raw.as_ptr()) };
        }
    }
}

impl Drop for ConnectionState {
    fn drop(&mut self) {
        self.close();
    }
}

/// A shared nonblocking database connection.
#[derive(Clone)]
pub struct Connection {
    state: Rc<RefCell<ConnectionState>>,
}

impl Connection {
    /// Starts opening a connection.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid connection string or a libpq allocation failure.
    pub fn open(connection_string: &[u8]) -> Result<(Self, Operation), Error> {
        let connection_string = c_string(connection_string, "the PostgreSQL connection string")?;
        // SAFETY: the connection string is null-terminated and lives through the call.
        let raw = unsafe { PQconnectStart(connection_string.as_ptr()) };
        let raw = NonNull::new(raw)
            .ok_or_else(|| Error::message("libpq could not allocate a PostgreSQL connection"))?;
        let connection = Self {
            state: Rc::new(RefCell::new(ConnectionState {
                raw: Some(raw),
                phase: Phase::Connecting,
                next_statement: 1,
                pending_statements: Vec::new(),
                cancelled: false,
            })),
        };
        let operation = Operation::connecting(connection.clone());
        Ok((connection, operation))
    }

    /// Starts a connection health check.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn ping(&self) -> Result<Operation, Error> {
        self.command(b"SELECT 1")
    }

    /// Starts preparing a statement.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid SQL or unless the connection is open and idle.
    pub fn prepare(&self, sql: &[u8]) -> Result<(Statement, Operation), Error> {
        self.prepare_with_format(sql, false)
    }

    /// Starts preparing a statement that requests binary results.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid SQL or unless the connection is open and idle.
    pub fn prepare_binary(&self, sql: &[u8]) -> Result<(Statement, Operation), Error> {
        self.prepare_with_format(sql, true)
    }

    fn prepare_with_format(
        &self,
        sql: &[u8],
        binary_results: bool,
    ) -> Result<(Statement, Operation), Error> {
        let sql = c_string(sql, "the SQL string for PostgreSQL")?;
        let name = {
            let mut state = self.state.borrow_mut();
            require_idle(&state)?;
            let name = format!("whim_{}", state.next_statement);
            state.next_statement = state.next_statement.wrapping_add(1);
            CString::new(name).map_err(|_| Error::message("invalid prepared statement name"))?
        };
        let raw = self.raw()?;
        // SAFETY: all pointers are valid for the duration of this libpq call.
        let sent =
            unsafe { PQsendPrepare(raw.as_ptr(), name.as_ptr(), sql.as_ptr(), 0, ptr::null()) };
        if sent == 0 {
            return Err(connection_error(raw.as_ptr()));
        }
        self.state.borrow_mut().phase = Phase::Busy;
        let binary_results = Rc::new(Cell::new(binary_results));
        let command = Rc::new(CommandState::new());
        Ok((
            Statement {
                connection: self.clone(),
                name,
                binary_results,
            },
            Operation::command(self.clone(), command),
        ))
    }

    /// Starts a parameterized query.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid inputs or unless the connection is open and idle.
    pub fn execute(
        &self,
        sql: &[u8],
        parameters: &[Parameter<'_>],
    ) -> Result<(ResultSet, Operation), Error> {
        self.execute_with_format(sql, parameters, false)
    }

    /// Starts a parameterized query that requests binary results.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid inputs or unless the connection is open and idle.
    pub fn execute_binary(
        &self,
        sql: &[u8],
        parameters: &[Parameter<'_>],
    ) -> Result<(ResultSet, Operation), Error> {
        self.execute_with_format(sql, parameters, true)
    }

    fn execute_with_format(
        &self,
        sql: &[u8],
        parameters: &[Parameter<'_>],
        binary_results: bool,
    ) -> Result<(ResultSet, Operation), Error> {
        let sql = c_string(sql, "the SQL string for PostgreSQL")?;
        let parameters = Parameters::new(parameters)?;
        let raw = self.raw()?;
        require_idle(&self.state.borrow())?;
        // SAFETY: `sql` and `parameters` retain every pointer through this call.
        let sent = unsafe {
            PQsendQueryParams(
                raw.as_ptr(),
                sql.as_ptr(),
                parameters.count,
                ptr::null(),
                parameters.pointers.as_ptr(),
                parameters.lengths.as_ptr(),
                parameters.formats.as_ptr(),
                i32::from(binary_results),
            )
        };
        if sent == 0 {
            return Err(connection_error(raw.as_ptr()));
        }
        start_chunked_row_mode(raw)?;
        self.state.borrow_mut().phase = Phase::Busy;
        Ok(self.query(None))
    }

    /// Starts a transaction.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn begin(&self, isolation: Option<Isolation>, read_only: bool) -> Result<Operation, Error> {
        let mut sql = String::from("BEGIN");
        if let Some(isolation) = isolation {
            sql.push_str(" ISOLATION LEVEL ");
            sql.push_str(match isolation {
                Isolation::ReadUncommitted => "READ UNCOMMITTED",
                Isolation::ReadCommitted => "READ COMMITTED",
                Isolation::RepeatableRead => "REPEATABLE READ",
                Isolation::Serializable => "SERIALIZABLE",
            });
        }
        if read_only {
            sql.push_str(" READ ONLY");
        }
        self.command(sql.as_bytes())
    }

    /// Starts committing the current transaction.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn commit(&self) -> Result<Operation, Error> {
        self.command(b"COMMIT")
    }

    /// Starts rolling back the current transaction.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn rollback(&self) -> Result<Operation, Error> {
        self.command(b"ROLLBACK")
    }

    fn command(&self, sql: &[u8]) -> Result<Operation, Error> {
        let sql = c_string(sql, "the SQL string for PostgreSQL")?;
        let raw = self.raw()?;
        require_idle(&self.state.borrow())?;
        // SAFETY: `sql` is null-terminated and all other pointer arguments are null.
        let sent = unsafe {
            PQsendQueryParams(
                raw.as_ptr(),
                sql.as_ptr(),
                0,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
            )
        };
        if sent == 0 {
            return Err(connection_error(raw.as_ptr()));
        }
        self.state.borrow_mut().phase = Phase::Busy;
        Ok(Operation::command(
            self.clone(),
            Rc::new(CommandState::new()),
        ))
    }

    /// Starts retiring every prepared statement whose last handle was dropped.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn retire_statements(&self, minimum: usize) -> Result<Option<Operation>, Error> {
        let sql = {
            let state = self.state.borrow();
            require_idle(&state)?;
            if state.pending_statements.len() < minimum.max(1) {
                return Ok(None);
            }

            retirement_query(&state.pending_statements)?
        };
        let raw = self.raw()?;
        // SAFETY: `sql` is a live, null-terminated query string.
        let sent = unsafe { PQsendQuery(raw.as_ptr(), sql.as_ptr()) };
        if sent == 0 {
            return Err(connection_error(raw.as_ptr()));
        }

        let mut state = self.state.borrow_mut();
        state.pending_statements.clear();
        state.phase = Phase::Busy;
        Ok(Some(Operation::command(
            self.clone(),
            Rc::new(CommandState::new()),
        )))
    }

    fn query(&self, binary_results: Option<Rc<Cell<bool>>>) -> (ResultSet, Operation) {
        let query = Rc::new(QueryState::new(binary_results));
        (
            ResultSet::new(self.clone(), Rc::clone(&query)),
            Operation::query(self.clone(), query),
        )
    }

    fn raw(&self) -> Result<NonNull<PGconn>, Error> {
        self.state.borrow().raw()
    }

    fn pending<T>(&self, interest: Interest) -> Result<Progress<T>, Error> {
        Ok(Progress::Pending {
            descriptor: self.state.borrow().descriptor()?,
            interest,
        })
    }

    fn finish(&self) {
        let mut state = self.state.borrow_mut();
        if state.phase != Phase::Closed {
            state.phase = Phase::Idle;
        }
    }

    fn cancel(&self) {
        let mut state = self.state.borrow_mut();
        state.cancelled = true;
        let Ok(descriptor) = state.descriptor() else {
            return;
        };
        // SAFETY: `descriptor` belongs to the live connection.
        let descriptor = unsafe { BorrowedFd::borrow_raw(descriptor) };
        _ = net::shutdown(descriptor, net::Shutdown::Both);
    }

    fn fail_if_cancelled(&self) -> Result<(), Error> {
        if !self.state.borrow().cancelled {
            return Ok(());
        }
        self.close();
        Err(Error::message("the PostgreSQL operation was cancelled"))
    }

    fn is_cancelled(&self) -> bool {
        self.state.borrow().cancelled
    }

    /// Reports whether the connection can accept another operation.
    #[must_use]
    pub fn is_reusable(&self) -> bool {
        let state = self.state.borrow();
        state.phase == Phase::Idle
            && state.raw.is_some_and(|raw| {
                // SAFETY: `raw` is retained by `state` for the duration of the call.
                unsafe { PQstatus(raw.as_ptr()) == ConnStatusType::CONNECTION_OK }
            })
    }

    /// Reports whether the connection is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state.borrow().phase == Phase::Closed
    }

    /// Closes the connection.
    pub fn close(&self) {
        self.state.borrow_mut().close();
    }
}

fn retirement_query(names: &[CString]) -> Result<CString, Error> {
    let mut sql = Vec::new();
    for name in names {
        sql.extend_from_slice(b"DEALLOCATE \"");
        sql.extend_from_slice(name.as_bytes());
        sql.extend_from_slice(b"\";");
    }
    CString::new(sql).map_err(|_| Error::message("invalid prepared statement retirement query"))
}

/// A transaction isolation level.
#[derive(Clone, Copy)]
pub enum Isolation {
    /// Read uncommitted.
    ReadUncommitted,
    /// Read committed.
    ReadCommitted,
    /// Repeatable read.
    RepeatableRead,
    /// Serializable.
    Serializable,
}

/// A prepared statement.
pub struct Statement {
    connection: Connection,
    name: CString,
    binary_results: Rc<Cell<bool>>,
}

impl Statement {
    /// Returns the connection that owns this prepared statement.
    #[must_use]
    pub fn connection(&self) -> Connection {
        self.connection.clone()
    }

    /// Starts executing this statement.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid parameters or unless the connection is open and idle.
    pub fn execute(&self, parameters: &[Parameter<'_>]) -> Result<(ResultSet, Operation), Error> {
        let parameters = Parameters::new(parameters)?;
        let raw = self.connection.raw()?;
        require_idle(&self.connection.state.borrow())?;
        let result_format = i32::from(self.binary_results.get());
        // SAFETY: `self` and `parameters` retain every pointer through this call.
        let sent = unsafe {
            PQsendQueryPrepared(
                raw.as_ptr(),
                self.name.as_ptr(),
                parameters.count,
                parameters.pointers.as_ptr(),
                parameters.lengths.as_ptr(),
                parameters.formats.as_ptr(),
                result_format,
            )
        };
        if sent == 0 {
            return Err(connection_error(raw.as_ptr()));
        }
        start_chunked_row_mode(raw)?;
        self.connection.state.borrow_mut().phase = Phase::Busy;
        Ok(self.connection.query(Some(Rc::clone(&self.binary_results))))
    }
}

impl Drop for Statement {
    fn drop(&mut self) {
        let mut state = self.connection.state.borrow_mut();
        if state.phase != Phase::Closed {
            state.pending_statements.push(self.name.clone());
        }
    }
}

enum ParameterStorage<'value> {
    Null,
    Owned(Vec<u8>),
    Borrowed(&'value [u8]),
}

impl ParameterStorage<'_> {
    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Null => None,
            Self::Owned(value) => Some(value),
            Self::Borrowed(value) => Some(value),
        }
    }
}

struct Parameters<'value> {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "owns the buffers referenced by `pointers`")
    )]
    storage: Vec<ParameterStorage<'value>>,
    pointers: Vec<*const c_char>,
    lengths: Vec<i32>,
    formats: Vec<i32>,
    count: i32,
}

impl<'value> Parameters<'value> {
    fn new(parameters: &[Parameter<'value>]) -> Result<Self, Error> {
        let count = i32::try_from(parameters.len())
            .map_err(|_| Error::message("too many PostgreSQL parameters"))?;
        let mut storage = Vec::with_capacity(parameters.len());
        let mut formats = Vec::with_capacity(parameters.len());
        for parameter in parameters {
            let (value, format) = match *parameter {
                Parameter::Null => (ParameterStorage::Null, 0),
                Parameter::Boolean(value) => (
                    ParameterStorage::Owned(text_parameter(if value { b"t" } else { b"f" })?),
                    0,
                ),
                Parameter::Integer(value) => {
                    let mut buffer = itoa::Buffer::new();
                    (
                        ParameterStorage::Owned(text_parameter(buffer.format(value).as_bytes())?),
                        0,
                    )
                }
                Parameter::Real(value) => {
                    let mut buffer = ryu::Buffer::new();
                    (
                        ParameterStorage::Owned(text_parameter(buffer.format(value).as_bytes())?),
                        0,
                    )
                }
                Parameter::Text(value) => (ParameterStorage::Owned(text_parameter(value)?), 0),
                Parameter::Blob(value) => (ParameterStorage::Borrowed(value), 1),
            };
            storage.push(value);
            formats.push(format);
        }
        let pointers = storage
            .iter()
            .map(|value| {
                value
                    .bytes()
                    .map_or(ptr::null(), |value| value.as_ptr().cast())
            })
            .collect();
        let lengths = storage
            .iter()
            .zip(&formats)
            .map(|(value, format)| {
                value.bytes().map_or(Ok(0), |value| {
                    let length = if *format == 0 {
                        value.len().checked_sub(1).ok_or_else(|| {
                            Error::message("a PostgreSQL text parameter is not terminated")
                        })?
                    } else {
                        value.len()
                    };
                    i32::try_from(length)
                        .map_err(|_| Error::message("a PostgreSQL parameter is too large"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            storage,
            pointers,
            lengths,
            formats,
            count,
        })
    }
}

fn text_parameter(value: &[u8]) -> Result<Vec<u8>, Error> {
    if value.contains(&0) {
        return Err(Error::message(
            "a PostgreSQL text parameter contains a null byte",
        ));
    }
    let capacity = value
        .len()
        .checked_add(1)
        .ok_or_else(|| Error::message("a PostgreSQL parameter is too large"))?;
    let mut terminated = Vec::with_capacity(capacity);
    terminated.extend_from_slice(value);
    terminated.push(0);
    Ok(terminated)
}

struct CommandState {
    error: RefCell<Option<Error>>,
}

impl CommandState {
    const fn new() -> Self {
        Self {
            error: RefCell::new(None),
        }
    }
}

struct QueryState {
    metadata: RefCell<Option<Metadata>>,
    field_types: RefCell<Vec<Oid>>,
    field_formats: RefCell<Vec<i32>>,
    binary_results: Option<Rc<Cell<bool>>>,
    rows: RefCell<VecDeque<Row>>,
    error: RefCell<Option<Error>>,
    finished: Cell<bool>,
}

impl QueryState {
    const fn new(binary_results: Option<Rc<Cell<bool>>>) -> Self {
        Self {
            metadata: RefCell::new(None),
            field_types: RefCell::new(Vec::new()),
            field_formats: RefCell::new(Vec::new()),
            binary_results,
            rows: RefCell::new(VecDeque::new()),
            error: RefCell::new(None),
            finished: Cell::new(false),
        }
    }
}

enum OperationKind {
    Connect,
    Command(Rc<CommandState>),
    Query(Rc<QueryState>),
}

/// A pending connection, command, or query operation.
pub struct Operation {
    connection: Connection,
    kind: OperationKind,
    completed: Cell<bool>,
}

impl Operation {
    const fn connecting(connection: Connection) -> Self {
        Self {
            connection,
            kind: OperationKind::Connect,
            completed: Cell::new(false),
        }
    }

    const fn command(connection: Connection, command: Rc<CommandState>) -> Self {
        Self {
            connection,
            kind: OperationKind::Command(command),
            completed: Cell::new(false),
        }
    }

    const fn query(connection: Connection, query: Rc<QueryState>) -> Self {
        Self {
            connection,
            kind: OperationKind::Query(query),
            completed: Cell::new(false),
        }
    }

    /// Polls the operation once.
    ///
    /// # Errors
    ///
    /// Returns a connection or query error.
    pub fn poll(&self) -> Result<Progress<()>, Error> {
        if self.completed.get() {
            return Ok(Progress::Ready(()));
        }
        if let Err(error) = self.connection.fail_if_cancelled() {
            self.completed.set(true);
            return Err(error);
        }
        let progress = match &self.kind {
            OperationKind::Connect => poll_connect(&self.connection),
            OperationKind::Command(command) => poll_command(&self.connection, command),
            OperationKind::Query(query) => poll_query_start(&self.connection, query),
        };
        if matches!(progress, Ok(Progress::Ready(())) | Err(_)) {
            self.completed.set(true);
        }
        progress
    }

    /// Cancels this operation.
    pub fn cancel(&self) {
        if !self.completed.get() {
            if let OperationKind::Query(query) = &self.kind {
                *query.error.borrow_mut() =
                    Some(Error::message("the PostgreSQL operation was cancelled"));
                query.finished.set(true);
            }
            self.connection.cancel();
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.completed.get() {
            self.connection.close();
        }
    }
}

/// A streaming result set.
pub struct ResultSet {
    connection: Connection,
    query: Rc<QueryState>,
    closed: Cell<bool>,
}

impl ResultSet {
    const fn new(connection: Connection, query: Rc<QueryState>) -> Self {
        Self {
            connection,
            query,
            closed: Cell::new(false),
        }
    }

    /// Returns the result metadata once it is available.
    #[must_use]
    pub fn metadata(&self) -> Option<Metadata> {
        self.query.metadata.borrow().clone()
    }

    /// Polls the next row once.
    ///
    /// # Errors
    ///
    /// Returns a connection or query error.
    pub fn poll_row(&self) -> Result<Progress<Option<Row>>, Error> {
        for _ in 0..RESULT_CHUNK_POLL_BUDGET {
            if let Some(row) = self.query.rows.borrow_mut().pop_front() {
                return Ok(Progress::Ready(Some(row)));
            }
            if self.query.finished.get() {
                self.closed.set(true);
                if self.connection.is_cancelled() {
                    self.connection.close();
                }

                if let Some(error) = self.query.error.borrow_mut().take() {
                    return Err(error);
                }

                return Ok(Progress::Ready(None));
            }

            if let Some(pending) =
                drive_query(&self.connection, &self.query, RowDisposition::Buffer)?
            {
                return Ok(pending.into());
            }
        }

        Ok(Progress::Yield)
    }

    /// Discards unread rows and polls until the result closes.
    ///
    /// # Errors
    ///
    /// Returns a connection or query error.
    pub fn poll_close(&self) -> Result<Progress<()>, Error> {
        for _ in 0..RESULT_CHUNK_POLL_BUDGET {
            self.query.rows.borrow_mut().clear();
            if self.query.finished.get() {
                self.closed.set(true);
                return Ok(Progress::Ready(()));
            }

            if let Some(pending) =
                drive_query(&self.connection, &self.query, RowDisposition::Discard)?
            {
                return Ok(pending.into());
            }
        }

        Ok(Progress::Yield)
    }

    /// Interrupts this result and closes its connection.
    pub fn interrupt(&self) {
        *self.query.error.borrow_mut() =
            Some(Error::message("the PostgreSQL operation was cancelled"));
        self.query.finished.set(true);
        self.connection.cancel();
    }

    /// Reports whether this result has finished or closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.get() || self.query.finished.get()
    }
}

impl Drop for ResultSet {
    fn drop(&mut self) {
        if !self.query.finished.get() {
            self.connection.close();
        }
    }
}

fn poll_connect(connection: &Connection) -> Result<Progress<()>, Error> {
    let raw = connection.raw()?;
    // SAFETY: `raw` is a live connection in the connecting phase.
    match unsafe { PQconnectPoll(raw.as_ptr()) } {
        PostgresPollingStatusType::PGRES_POLLING_READING => connection.pending(Interest::Readable),
        PostgresPollingStatusType::PGRES_POLLING_WRITING => connection.pending(Interest::Writable),
        PostgresPollingStatusType::PGRES_POLLING_OK => {
            // SAFETY: `raw` is a live connection.
            if unsafe { PQsetnonblocking(raw.as_ptr(), 1) } != 0 {
                let error = connection_error(raw.as_ptr());
                connection.close();
                return Err(error);
            }
            connection.finish();
            Ok(Progress::Ready(()))
        }
        PostgresPollingStatusType::PGRES_POLLING_FAILED => {
            let error = connection_error(raw.as_ptr());
            connection.close();
            Err(error)
        }
        PostgresPollingStatusType::PGRES_POLLING_ACTIVE => {
            connection.pending(Interest::ReadableOrWritable)
        }
    }
}

fn poll_command(connection: &Connection, command: &CommandState) -> Result<Progress<()>, Error> {
    let raw = connection.raw()?;
    let output_pending = consume_and_flush(raw)?;
    // SAFETY: `raw` is a live connection with an active command.
    while unsafe { PQisBusy(raw.as_ptr()) } == 0 {
        // SAFETY: libpq permits fetching results while the connection is not busy.
        let result = unsafe { PQgetResult(raw.as_ptr()) };
        let Some(result) = NonNull::new(result) else {
            connection.finish();
            if let Some(error) = command.error.borrow_mut().take() {
                return Err(error);
            }
            return Ok(Progress::Ready(()));
        };
        let result = RawResult(result);
        if let Some(error) = result_error(&result) {
            *command.error.borrow_mut() = Some(error);
        }
    }
    connection.pending(if output_pending {
        Interest::ReadableOrWritable
    } else {
        Interest::Readable
    })
}

fn poll_query_start(connection: &Connection, query: &QueryState) -> Result<Progress<()>, Error> {
    for _ in 0..RESULT_CHUNK_POLL_BUDGET {
        if query.metadata.borrow().is_some() || query.finished.get() {
            if let Some(error) = query.error.borrow().as_ref() {
                return Err(error.clone());
            }

            return Ok(Progress::Ready(()));
        }

        if let Some(pending) = drive_query(connection, query, RowDisposition::Buffer)? {
            return Ok(pending.into());
        }
    }

    Ok(Progress::Yield)
}

#[derive(Clone, Copy)]
enum RowDisposition {
    Buffer,
    Discard,
}

struct PendingIo {
    descriptor: RawFd,
    interest: Interest,
}

impl<T> From<PendingIo> for Progress<T> {
    fn from(pending: PendingIo) -> Self {
        Self::Pending {
            descriptor: pending.descriptor,
            interest: pending.interest,
        }
    }
}

fn drive_query(
    connection: &Connection,
    query: &QueryState,
    rows: RowDisposition,
) -> Result<Option<PendingIo>, Error> {
    connection.fail_if_cancelled()?;
    let raw = connection.raw()?;
    let output_pending = consume_and_flush(raw)?;
    // SAFETY: `raw` is a live connection with an active query.
    if unsafe { PQisBusy(raw.as_ptr()) } != 0 {
        let interest = if output_pending {
            Interest::ReadableOrWritable
        } else {
            Interest::Readable
        };
        return Ok(Some(PendingIo {
            descriptor: connection.state.borrow().descriptor()?,
            interest,
        }));
    }

    // SAFETY: libpq permits fetching a result after `PQisBusy` returns zero.
    let result = unsafe { PQgetResult(raw.as_ptr()) };
    let Some(result) = NonNull::new(result) else {
        query.finished.set(true);
        connection.finish();
        return Ok(None);
    };
    let result = RawResult(result);
    // SAFETY: `result` owns a live `PGresult`.
    let status = unsafe { PQresultStatus(result.0.as_ptr()) };
    match status {
        ExecStatusType::PGRES_SINGLE_TUPLE
        | ExecStatusType::PGRES_TUPLES_CHUNK
        | ExecStatusType::PGRES_TUPLES_OK => {
            ensure_metadata(query, &result)?;
            if matches!(rows, RowDisposition::Buffer) {
                append_result_rows(query, &result)?;
            }
        }
        ExecStatusType::PGRES_COMMAND_OK | ExecStatusType::PGRES_EMPTY_QUERY => {
            if query.metadata.borrow().is_none() {
                *query.metadata.borrow_mut() = Some(Metadata {
                    columns: Vec::new(),
                    affected_rows: affected_rows(&result),
                });
            }
        }
        ExecStatusType::PGRES_BAD_RESPONSE
        | ExecStatusType::PGRES_NONFATAL_ERROR
        | ExecStatusType::PGRES_FATAL_ERROR
        | ExecStatusType::PGRES_PIPELINE_ABORTED => {
            *query.error.borrow_mut() = Some(result_error(&result).unwrap_or_else(|| {
                Error::message("the PostgreSQL server returned an unknown query error")
            }));
        }
        ExecStatusType::PGRES_COPY_OUT
        | ExecStatusType::PGRES_COPY_IN
        | ExecStatusType::PGRES_COPY_BOTH
        | ExecStatusType::PGRES_PIPELINE_SYNC => {
            *query.error.borrow_mut() = Some(Error::message(
                "this PostgreSQL result mode is not supported",
            ));
            connection.close();
            query.finished.set(true);
        }
    }
    Ok(None)
}

fn consume_and_flush(raw: NonNull<PGconn>) -> Result<bool, Error> {
    // SAFETY: `raw` is a live connection.
    if unsafe { PQconsumeInput(raw.as_ptr()) } == 0 {
        return Err(connection_error(raw.as_ptr()));
    }
    // SAFETY: `raw` is a live connection.
    match unsafe { PQflush(raw.as_ptr()) } {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(connection_error(raw.as_ptr())),
    }
}

fn ensure_metadata(query: &QueryState, result: &RawResult) -> Result<(), Error> {
    if query.metadata.borrow().is_some() {
        return Ok(());
    }
    // SAFETY: `result` owns a live `PGresult`.
    let count = unsafe { PQnfields(result.0.as_ptr()) };
    let count = usize::try_from(count)
        .map_err(|_| Error::message("the PostgreSQL server returned invalid column metadata"))?;
    let mut columns = Vec::with_capacity(count);
    let mut field_types = Vec::with_capacity(count);
    let mut field_formats = Vec::with_capacity(count);
    for index in 0..count {
        let index = i32::try_from(index)
            .map_err(|_| Error::message("the PostgreSQL server returned too many columns"))?;
        // SAFETY: `index` is within the field count reported by libpq.
        let name = unsafe { PQfname(result.0.as_ptr(), index) };
        // SAFETY: `name` is null or a C string owned by `result`.
        let name = unsafe { c_bytes(name) }.unwrap_or_default();
        // SAFETY: `index` is within the field count reported by libpq.
        let field_type = unsafe { PQftype(result.0.as_ptr(), index) };
        // SAFETY: `index` is within the field count reported by libpq.
        let field_format = unsafe { PQfformat(result.0.as_ptr(), index) };
        let type_name = type_name(field_type);
        columns.push(Column { name, type_name });
        field_types.push(field_type);
        field_formats.push(field_format);
    }
    if !field_types.is_empty()
        && let Some(binary_results) = &query.binary_results
        && field_types.iter().copied().all(supports_binary_result)
    {
        binary_results.set(true);
    }
    *query.field_types.borrow_mut() = field_types;
    *query.field_formats.borrow_mut() = field_formats;
    *query.metadata.borrow_mut() = Some(Metadata {
        columns,
        affected_rows: None,
    });
    Ok(())
}

fn append_result_rows(query: &QueryState, result: &RawResult) -> Result<(), Error> {
    // SAFETY: `result` owns a live `PGresult`.
    let fields = unsafe { PQnfields(result.0.as_ptr()) };
    let fields = usize::try_from(fields)
        .map_err(|_| Error::message("the PostgreSQL server returned an invalid row"))?;
    // SAFETY: `result` owns a live `PGresult`.
    let tuples = unsafe { PQntuples(result.0.as_ptr()) };
    let tuples = usize::try_from(tuples)
        .map_err(|_| Error::message("the PostgreSQL server returned an invalid row count"))?;
    let field_types = query.field_types.borrow();
    let field_formats = query.field_formats.borrow();
    let mut rows = query.rows.borrow_mut();
    rows.reserve(tuples);
    for row in 0..tuples {
        let row = i32::try_from(row)
            .map_err(|_| Error::message("the PostgreSQL server returned too many rows"))?;
        let mut values = Vec::with_capacity(fields);
        for field in 0..fields {
            let field_index = i32::try_from(field)
                .map_err(|_| Error::message("the PostgreSQL server returned too many columns"))?;
            // SAFETY: the row and field indexes are within libpq's reported bounds.
            if unsafe { PQgetisnull(result.0.as_ptr(), row, field_index) } != 0 {
                values.push(Value::Null);
                continue;
            }
            // SAFETY: the row and field indexes are within libpq's reported bounds.
            let length = unsafe { PQgetlength(result.0.as_ptr(), row, field_index) };
            let length = usize::try_from(length).map_err(|_| {
                Error::message("the PostgreSQL server returned an invalid value length")
            })?;
            // SAFETY: the row and field indexes are within libpq's reported bounds.
            let value = unsafe { PQgetvalue(result.0.as_ptr(), row, field_index) };
            let bytes = if length == 0 {
                &[]
            } else {
                let value = NonNull::new(value.cast::<u8>()).ok_or_else(|| {
                    Error::message("the PostgreSQL server returned a null value pointer")
                })?;
                // SAFETY: libpq returned a non-null value valid for `length` bytes.
                unsafe { slice::from_raw_parts(value.as_ptr(), length) }
            };
            let Some(&oid) = field_types.get(field) else {
                return Err(Error::message(
                    "the PostgreSQL server returned inconsistent column metadata",
                ));
            };
            let Some(&format) = field_formats.get(field) else {
                return Err(Error::message(
                    "the PostgreSQL server returned inconsistent column metadata",
                ));
            };
            values.push(database_value(oid, format, bytes)?);
        }
        rows.push_back(values);
    }
    Ok(())
}

fn database_value(oid: Oid, format: i32, bytes: &[u8]) -> Result<Value, Error> {
    match format {
        0 => database_text_value(oid, bytes),
        1 => database_binary_value(oid, bytes),
        _ => Err(Error::message(
            "the PostgreSQL server returned an unknown value format",
        )),
    }
}

fn database_text_value(oid: Oid, bytes: &[u8]) -> Result<Value, Error> {
    match oid {
        BOOL_OID => Ok(Value::Boolean(bytes == b"t")),
        INT2_OID | INT4_OID | INT8_OID => str::from_utf8(bytes)
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .map(Value::Integer)
            .ok_or_else(|| Error::message("the PostgreSQL server returned an invalid integer")),
        FLOAT4_OID | FLOAT8_OID => str::from_utf8(bytes)
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .map(Value::Real)
            .ok_or_else(|| Error::message("the PostgreSQL server returned an invalid float")),
        BYTEA_OID => decode_text_bytea(bytes).map(Value::Blob),
        _ => Ok(Value::Text(bytes.to_vec())),
    }
}

fn database_binary_value(oid: Oid, bytes: &[u8]) -> Result<Value, Error> {
    match oid {
        BOOL_OID => match bytes {
            [0] => Ok(Value::Boolean(false)),
            [1] => Ok(Value::Boolean(true)),
            _ => Err(invalid_binary_value("boolean value")),
        },
        INT2_OID => binary_array(bytes, "small integer")
            .map(i16::from_be_bytes)
            .map(i64::from)
            .map(Value::Integer),
        INT4_OID => binary_array(bytes, "integer")
            .map(i32::from_be_bytes)
            .map(i64::from)
            .map(Value::Integer),
        INT8_OID => binary_array(bytes, "big integer")
            .map(i64::from_be_bytes)
            .map(Value::Integer),
        FLOAT4_OID => binary_array(bytes, "single-precision float")
            .map(u32::from_be_bytes)
            .map(f32::from_bits)
            .map(f64::from)
            .map(Value::Real),
        FLOAT8_OID => binary_array(bytes, "double-precision float")
            .map(u64::from_be_bytes)
            .map(f64::from_bits)
            .map(Value::Real),
        BYTEA_OID => Ok(Value::Blob(bytes.to_vec())),
        CHAR_OID | NAME_OID | TEXT_OID | JSON_OID | BPCHAR_OID | VARCHAR_OID => {
            Ok(Value::Text(bytes.to_vec()))
        }
        JSONB_OID => match bytes {
            [1, value @ ..] => Ok(Value::Text(value.to_vec())),
            _ => Err(invalid_binary_value("JSONB")),
        },
        _ => Err(Error::message(
            "the PostgreSQL server returned an unsupported binary value",
        )),
    }
}

const fn supports_binary_result(oid: Oid) -> bool {
    matches!(
        oid,
        BOOL_OID
            | BYTEA_OID
            | CHAR_OID
            | NAME_OID
            | INT8_OID
            | INT2_OID
            | INT4_OID
            | TEXT_OID
            | JSON_OID
            | FLOAT4_OID
            | FLOAT8_OID
            | BPCHAR_OID
            | VARCHAR_OID
            | JSONB_OID
    )
}

fn binary_array<const LENGTH: usize>(bytes: &[u8], name: &str) -> Result<[u8; LENGTH], Error> {
    bytes.try_into().map_err(|_| invalid_binary_value(name))
}

fn invalid_binary_value(name: &str) -> Error {
    Error::message(format!(
        "the PostgreSQL server returned an invalid binary {name}"
    ))
}

fn decode_text_bytea(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    if let Some(hexadecimal) = bytes.strip_prefix(b"\\x") {
        return decode_hexadecimal_bytea(hexadecimal);
    }

    let mut decoded = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        if bytes[position] != b'\\' {
            decoded.push(bytes[position]);
            position += 1;
            continue;
        }
        if bytes.get(position + 1) == Some(&b'\\') {
            decoded.push(b'\\');
            position += 2;
            continue;
        }
        let Some(octal) = bytes.get(position + 1..position + 4) else {
            return Err(invalid_text_bytea());
        };
        if !matches!(octal[0], b'0'..=b'3')
            || !octal[1..].iter().all(|byte| matches!(byte, b'0'..=b'7'))
        {
            return Err(invalid_text_bytea());
        }
        decoded.push((octal[0] - b'0') * 64 + (octal[1] - b'0') * 8 + octal[2] - b'0');
        position += 4;
    }
    Ok(decoded)
}

fn decode_hexadecimal_bytea(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid_text_bytea());
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    let mut position = 0;
    while position < bytes.len() {
        let high = hexadecimal_digit(bytes[position]).ok_or_else(invalid_text_bytea)?;
        let low = hexadecimal_digit(bytes[position + 1]).ok_or_else(invalid_text_bytea)?;
        decoded.push(high << 4 | low);
        position += 2;
    }
    Ok(decoded)
}

const fn hexadecimal_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn invalid_text_bytea() -> Error {
    Error::message("the PostgreSQL server returned an invalid bytea value")
}

fn affected_rows(result: &RawResult) -> Option<u64> {
    // SAFETY: `result` owns a live `PGresult`; libpq returns a borrowed C string.
    let value = unsafe { PQcmdTuples(result.0.as_ptr()) };
    // SAFETY: `value` is null or a valid C string owned by `result`.
    let bytes = unsafe { c_bytes(value) }?;
    if bytes.is_empty() {
        return None;
    }
    str::from_utf8(&bytes).ok()?.parse().ok()
}

fn result_error(result: &RawResult) -> Option<Error> {
    // SAFETY: `result` owns a live `PGresult`.
    let status = unsafe { PQresultStatus(result.0.as_ptr()) };
    if !matches!(
        status,
        ExecStatusType::PGRES_BAD_RESPONSE
            | ExecStatusType::PGRES_NONFATAL_ERROR
            | ExecStatusType::PGRES_FATAL_ERROR
            | ExecStatusType::PGRES_PIPELINE_ABORTED
    ) {
        return None;
    }
    // SAFETY: `result` owns a live `PGresult`; libpq returns a borrowed C string.
    let message = unsafe { c_text(PQresultErrorMessage(result.0.as_ptr())) }
        .unwrap_or_else(|| "the PostgreSQL query failed".to_string());
    Some(Error {
        message,
        sqlstate: error_field(result, PG_DIAG_SQLSTATE),
        detail: error_field(result, PG_DIAG_MESSAGE_DETAIL),
        hint: error_field(result, PG_DIAG_MESSAGE_HINT),
    })
}

fn error_field(result: &RawResult, field: u8) -> Option<String> {
    // SAFETY: `result` owns a live `PGresult`; libpq returns a borrowed C string.
    unsafe { c_text(PQresultErrorField(result.0.as_ptr(), i32::from(field))) }
}

fn connection_error(raw: *mut PGconn) -> Error {
    // SAFETY: callers pass a live connection; libpq returns a borrowed C string.
    let message = unsafe { c_text(PQerrorMessage(raw)) }
        .unwrap_or_else(|| "the PostgreSQL connection failed".to_string());
    Error::message(message)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Copies a nullable C string.
///
/// # Safety
///
/// `value` must be null or point to a live null-terminated string.
unsafe fn c_bytes(value: *const c_char) -> Option<Vec<u8>> {
    if value.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees that `value` points to a live C string.
    Some(unsafe { CStr::from_ptr(value) }.to_bytes().to_vec())
}

/// Copies and normalizes a nullable C string.
///
/// # Safety
///
/// `value` must be null or point to a live null-terminated string.
unsafe fn c_text(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees that `value` points to a live C string.
    Some(text(unsafe { CStr::from_ptr(value) }.to_bytes()))
}

fn c_string(bytes: &[u8], subject: &str) -> Result<CString, Error> {
    CString::new(bytes).map_err(|_| Error::message(format!("{subject} contains a null byte")))
}

fn require_idle(state: &ConnectionState) -> Result<(), Error> {
    match state.phase {
        Phase::Idle => Ok(()),
        Phase::Connecting => Err(Error::message("the PostgreSQL connection is still opening")),
        Phase::Busy => Err(Error::message("the PostgreSQL connection is busy")),
        Phase::Closed => Err(Error::message("the PostgreSQL connection is closed")),
    }
}

fn start_chunked_row_mode(raw: NonNull<PGconn>) -> Result<(), Error> {
    // SAFETY: `raw` is a live connection with a query in progress.
    if unsafe { PQsetChunkedRowsMode(raw.as_ptr(), ROW_CHUNK_SIZE) } == 0 {
        return Err(connection_error(raw.as_ptr()));
    }
    Ok(())
}

fn type_name(oid: Oid) -> Option<Vec<u8>> {
    match oid {
        BOOL_OID => Some(b"bool".to_vec()),
        BYTEA_OID => Some(b"bytea".to_vec()),
        INT2_OID => Some(b"int2".to_vec()),
        INT4_OID => Some(b"int4".to_vec()),
        INT8_OID => Some(b"int8".to_vec()),
        FLOAT4_OID => Some(b"float4".to_vec()),
        FLOAT8_OID => Some(b"float8".to_vec()),
        _ => None,
    }
}

struct RawResult(NonNull<PGresult>);

impl Drop for RawResult {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the live result and drops it exactly once.
        unsafe { PQclear(self.0.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::ffi::CString;
    use std::rc::Rc;

    use crate::BOOL_OID;
    use crate::BYTEA_OID;
    use crate::Connection;
    use crate::ConnectionState;
    use crate::FLOAT4_OID;
    use crate::INT2_OID;
    use crate::JSONB_OID;
    use crate::Parameter;
    use crate::ParameterStorage;
    use crate::Parameters;
    use crate::Phase;
    use crate::Statement;
    use crate::TEXT_OID;
    use crate::Value;
    use crate::database_value;
    use crate::decode_text_bytea;
    use crate::retirement_query;
    use crate::supports_binary_result;

    #[test]
    fn dropped_statements_queue_one_batched_retirement_query() {
        let connection = Connection {
            state: Rc::new(RefCell::new(ConnectionState {
                raw: None,
                phase: Phase::Idle,
                next_statement: 3,
                pending_statements: Vec::new(),
                cancelled: false,
            })),
        };
        for name in ["whim_1", "whim_2"] {
            drop(Statement {
                connection: connection.clone(),
                name: CString::new(name).unwrap(),
                binary_results: Rc::new(Cell::new(false)),
            });
        }

        let state = connection.state.borrow();
        assert_eq!(state.pending_statements.len(), 2);
        assert_eq!(
            retirement_query(&state.pending_statements)
                .unwrap()
                .as_bytes(),
            b"DEALLOCATE \"whim_1\";DEALLOCATE \"whim_2\";"
        );
    }

    #[test]
    fn text_parameters_are_terminated_without_counting_the_terminator() {
        let blob = b"\0bytes";
        let Ok(parameters) = Parameters::new(&[
            Parameter::Integer(42),
            Parameter::Text(b"value"),
            Parameter::Blob(blob),
        ]) else {
            panic!("valid PostgreSQL parameters were rejected");
        };

        assert_eq!(parameters.lengths, [2, 5, 6]);
        assert!(matches!(
            &parameters.storage[0],
            ParameterStorage::Owned(value) if value == b"42\0"
        ));
        assert!(matches!(
            &parameters.storage[1],
            ParameterStorage::Owned(value) if value == b"value\0"
        ));
        assert!(matches!(
            &parameters.storage[2],
            ParameterStorage::Borrowed(value)
                if *value == blob && value.as_ptr() == blob.as_ptr()
        ));
    }

    #[test]
    fn text_parameters_reject_null_bytes() {
        assert!(Parameters::new(&[Parameter::Text(b"bad\0value")]).is_err());
    }

    #[test]
    fn text_bytea_decodes_hexadecimal_and_escape_formats() {
        assert_eq!(decode_text_bytea(b"\\x0041ff").unwrap(), b"\0A\xff");
        assert_eq!(decode_text_bytea(b"A\\000\\\\B").unwrap(), b"A\0\\B");
        assert!(decode_text_bytea(b"\\x0").is_err());
        assert!(decode_text_bytea(b"\\12x").is_err());
        assert!(decode_text_bytea(b"\\777").is_err());
    }

    #[test]
    fn binary_results_decode_supported_primitive_values() {
        assert!(matches!(
            database_value(BOOL_OID, 1, &[1]),
            Ok(Value::Boolean(true))
        ));
        assert!(matches!(
            database_value(INT2_OID, 1, &42_i16.to_be_bytes()),
            Ok(Value::Integer(42))
        ));
        assert!(matches!(
            database_value(FLOAT4_OID, 1, &1.5_f32.to_bits().to_be_bytes()),
            Ok(Value::Real(value)) if value.to_bits() == 1.5_f64.to_bits()
        ));
        assert!(matches!(
            database_value(BYTEA_OID, 1, b"\0blob"),
            Ok(Value::Blob(value)) if value == b"\0blob"
        ));
        assert!(matches!(
            database_value(TEXT_OID, 1, b"text"),
            Ok(Value::Text(value)) if value == b"text"
        ));
        assert!(matches!(
            database_value(JSONB_OID, 1, b"\x01{\"ok\":true}"),
            Ok(Value::Text(value)) if value == br#"{"ok":true}"#
        ));
    }

    #[test]
    fn binary_results_are_enabled_only_for_supported_types() {
        assert!(supports_binary_result(BYTEA_OID));
        assert!(supports_binary_result(TEXT_OID));
        assert!(!supports_binary_result(1_084));
    }
}
