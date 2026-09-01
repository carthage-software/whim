//! Runs database work off the event-loop thread.

#![cfg(unix)]
#![deny(clippy::nursery, clippy::pedantic)]
#![forbid(unsafe_code)]

mod error;
mod value;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::mem::replace;
use std::mem::swap;
use std::os::fd::AsRawFd;
use std::os::fd::RawFd;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rusqlite::Connection as SQLiteConnection;
use rusqlite::Error as SQLiteError;
use rusqlite::OpenFlags;
use rusqlite::Statement as SQLiteStatement;
use rusqlite::types::Value as SQLiteValue;
use rusqlite::types::ValueRef;

pub use error::Error;
pub use value::Column;
pub use value::Metadata;
pub use value::Row;
pub use value::Value;

const STATUS_OPENING: u8 = 0;
const STATUS_READY: u8 = 1;
const STATUS_BROKEN: u8 = 2;
const STATUS_CLOSED: u8 = 3;
const FIRST_OPERATION: u64 = 1;
const RESULT_BUFFER_SIZE: usize = 64;
const CANCELLATION_PROGRESS_INTERVAL: i32 = 1_000;

/// A job accepted by the blocking executor.
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// Runs blocking jobs away from the event loop.
pub trait Executor: Send + Sync {
    /// Queues one job.
    ///
    /// # Errors
    ///
    /// Returns an error when the executor cannot accept the job.
    fn submit(&self, job: Job) -> io::Result<()>;
}

/// Settings used to open a connection.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the booleans represent independent database settings"
)]
pub struct Configuration {
    /// The database path or URI.
    pub path: PathBuf,
    /// Opens the database without write access.
    pub read_only: bool,
    /// Creates a missing database.
    pub create: bool,
    /// Treats `path` as a database URI.
    pub uri: bool,
    /// Waits this long for locked tables.
    pub busy_timeout: Duration,
    /// Enforces foreign keys.
    pub foreign_keys: bool,
    /// The prepared-statement cache capacity.
    pub statement_cache_capacity: usize,
}

struct Notifier {
    reader: UnixDatagram,
    writer: UnixDatagram,
}

impl Notifier {
    fn new() -> io::Result<Self> {
        let (reader, writer) = UnixDatagram::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        Ok(Self { reader, writer })
    }

    fn descriptor(&self) -> RawFd {
        self.reader.as_raw_fd()
    }

    fn signal(&self) {
        loop {
            match self.writer.send(&[1]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Ok(_) | Err(_) => return,
            }
        }
    }

    fn drain(&self) {
        let mut bytes = [0_u8; 64];
        loop {
            match self.reader.recv(&mut bytes) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(_) => return,
            }
        }
    }
}

enum OperationResponse {
    Pending,
    Ready(Result<(), Error>),
    Consumed,
}

struct OperationShared {
    response: Mutex<OperationResponse>,
    connection: Arc<ConnectionShared>,
    id: u64,
    cancelled: AtomicBool,
}

impl OperationShared {
    fn new(connection: Arc<ConnectionShared>, id: u64) -> Arc<Self> {
        Arc::new(Self {
            response: Mutex::new(OperationResponse::Pending),
            connection,
            id,
            cancelled: AtomicBool::new(false),
        })
    }

    fn complete(&self, response: Result<(), Error>) {
        let mut current = self.response.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(*current, OperationResponse::Pending) {
            return;
        }
        *current = OperationResponse::Ready(response);
        drop(current);
        self.connection.notifier.signal();
    }

    fn take(&self) -> Option<Result<(), Error>> {
        let mut response = self.response.lock().unwrap_or_else(PoisonError::into_inner);
        match replace(&mut *response, OperationResponse::Consumed) {
            OperationResponse::Ready(result) => {
                drop(response);
                Some(result)
            }
            current @ (OperationResponse::Pending | OperationResponse::Consumed) => {
                *response = current;
                None
            }
        }
    }

    fn pending(&self) -> bool {
        matches!(
            *self.response.lock().unwrap_or_else(PoisonError::into_inner),
            OperationResponse::Pending
        )
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.connection.interrupt(self.id);
    }
}

/// A pending connection or command operation.
pub struct Operation {
    shared: Arc<OperationShared>,
}

impl Operation {
    const fn new(shared: Arc<OperationShared>) -> Self {
        Self { shared }
    }

    /// Returns the descriptor used to signal completion.
    #[must_use]
    pub fn descriptor(&self) -> RawFd {
        self.shared.connection.notifier.descriptor()
    }

    /// Takes the operation result once it is ready.
    #[must_use]
    pub fn poll(&self) -> Option<Result<(), Error>> {
        self.shared.take()
    }

    /// Clears pending completion notifications.
    pub fn drain_notification(&self) {
        self.shared.connection.notifier.drain();
    }

    /// Interrupts this operation.
    pub fn cancel(&self) {
        self.shared.cancel();
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if self.shared.pending() {
            self.shared.cancel();
        }
    }
}

struct ResultQueue {
    metadata: Option<Metadata>,
    rows: VecDeque<Row>,
    terminal: Option<Result<(), Error>>,
    closed: bool,
}

struct ResultShared {
    queue: Mutex<ResultQueue>,
    space: Condvar,
    connection: Arc<ConnectionShared>,
    id: u64,
    retired: AtomicBool,
}

impl ResultShared {
    fn new(connection: Arc<ConnectionShared>, id: u64) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(ResultQueue {
                metadata: None,
                rows: VecDeque::new(),
                terminal: None,
                closed: false,
            }),
            space: Condvar::new(),
            connection,
            id,
            retired: AtomicBool::new(false),
        })
    }

    fn set_metadata(&self, metadata: Metadata) {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .metadata = Some(metadata);
    }

    fn push_batch(&self, rows: &mut Vec<Row>) -> bool {
        if rows.is_empty() {
            return true;
        }

        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        while queue.rows.len() + rows.len() > RESULT_BUFFER_SIZE && !queue.closed {
            queue = self
                .space
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if queue.closed {
            return false;
        }
        let signal = queue.rows.is_empty();
        queue.rows.extend(rows.drain(..));
        drop(queue);
        if signal {
            self.connection.notifier.signal();
        }
        true
    }

    fn finish(&self, result: Result<(), Error>) {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.terminal = Some(result);
        drop(queue);
        self.connection.notifier.signal();
        self.space.notify_all();
    }

    fn fail(&self, operation: &OperationShared, error: Error) {
        self.retire();
        operation.complete(Err(error.clone()));
        self.finish(Err(error));
    }

    fn close(&self) {
        let should_interrupt = {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            if queue.closed {
                return;
            }
            queue.closed = true;
            queue.rows.clear();
            queue.terminal.is_none()
        };
        self.space.notify_all();
        self.connection.notifier.signal();
        if should_interrupt {
            self.connection.interrupt(self.id);
        }
    }

    fn retire(&self) {
        let mut active = self
            .connection
            .active_result
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if active.as_ref().is_some_and(|active| active.id == self.id) {
            active.take();
        }
        drop(active);
        self.connection.release(self.id);
        self.retired.store(true, Ordering::Release);
        self.connection.notifier.signal();
    }
}

/// A streaming query result.
pub struct ResultSet {
    shared: Arc<ResultShared>,
    rows: RefCell<VecDeque<Row>>,
}

impl ResultSet {
    const fn new(shared: Arc<ResultShared>) -> Self {
        Self {
            shared,
            rows: RefCell::new(VecDeque::new()),
        }
    }

    /// Returns the descriptor used to signal rows and completion.
    #[must_use]
    pub fn descriptor(&self) -> RawFd {
        self.shared.connection.notifier.descriptor()
    }

    /// Clears pending result notifications.
    pub fn drain_notification(&self) {
        self.shared.connection.notifier.drain();
    }

    /// Returns the result metadata once it is available.
    #[must_use]
    pub fn metadata(&self) -> Option<Metadata> {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .metadata
            .clone()
    }

    /// Takes the next row or terminal result when available.
    #[must_use]
    pub fn poll_row(&self) -> Option<Result<Option<Row>, Error>> {
        let mut rows = self.rows.borrow_mut();
        if let Some(row) = rows.pop_front() {
            return Some(Ok(Some(row)));
        }

        let mut queue = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !queue.rows.is_empty() {
            swap(&mut *rows, &mut queue.rows);
            let row = rows.pop_front();
            drop(queue);
            self.shared.space.notify_one();
            self.shared.connection.notifier.drain();
            return row.map(|row| Ok(Some(row)));
        }
        if queue.closed {
            drop(queue);
            return Some(Ok(None));
        }
        if let Some(terminal) = queue.terminal.take() {
            queue.closed = true;
            drop(queue);
            self.shared.connection.notifier.drain();
            return Some(terminal.map(|()| None));
        }
        drop(queue);
        None
    }

    /// Interrupts the query producing this result.
    pub fn interrupt(&self) {
        self.shared.connection.interrupt(self.shared.id);
    }

    /// Reports whether the result has closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed
    }

    /// Closes the result and discards unread rows.
    pub fn close(&self) {
        self.rows.borrow_mut().clear();
        self.shared.close();
    }

    /// Reports whether the worker released the connection.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.shared.retired.load(Ordering::Acquire)
    }
}

impl Drop for ResultSet {
    fn drop(&mut self) {
        self.shared.close();
    }
}

struct ConnectionState {
    connection: Option<SQLiteConnection>,
    transaction: bool,
}

struct ExecutionState {
    active: AtomicU64,
    cancelled: AtomicU64,
}

impl ExecutionState {
    const fn new() -> Self {
        Self {
            active: AtomicU64::new(FIRST_OPERATION),
            cancelled: AtomicU64::new(0),
        }
    }

    fn reserve(&self, id: u64) -> Result<(), Error> {
        self.active
            .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error::concurrent_operation())
    }

    fn release(&self, id: u64) {
        let _ = self
            .active
            .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
    }

    fn cancel(&self, id: u64) -> bool {
        if self.active.load(Ordering::Acquire) != id {
            return false;
        }
        self.cancelled.store(id, Ordering::Release);
        true
    }

    fn is_cancelled(&self, id: u64) -> bool {
        self.cancelled.load(Ordering::Acquire) == id
    }

    fn active_is_cancelled(&self) -> bool {
        let active = self.active.load(Ordering::Acquire);
        active != 0 && self.is_cancelled(active)
    }
}

impl ConnectionState {
    fn connection(&self) -> Result<&SQLiteConnection, Error> {
        self.connection
            .as_ref()
            .ok_or_else(|| Error::message("the SQLite connection is not open"))
    }
}

struct ConnectionShared {
    executor: Weak<dyn Executor>,
    state: Mutex<ConnectionState>,
    interrupt: Mutex<Option<rusqlite::InterruptHandle>>,
    notifier: Notifier,
    status: AtomicU8,
    execution: Arc<ExecutionState>,
    next_id: AtomicU64,
    active_result: Mutex<Option<ActiveResult>>,
}

struct ActiveResult {
    id: u64,
    result: Weak<ResultShared>,
}

impl ConnectionShared {
    fn opening(executor: Weak<dyn Executor>, notifier: Notifier) -> Arc<Self> {
        Arc::new(Self {
            executor,
            state: Mutex::new(ConnectionState {
                connection: None,
                transaction: false,
            }),
            interrupt: Mutex::new(None),
            notifier,
            status: AtomicU8::new(STATUS_OPENING),
            execution: Arc::new(ExecutionState::new()),
            next_id: AtomicU64::new(FIRST_OPERATION + 1),
            active_result: Mutex::new(None),
        })
    }

    fn reserve(&self) -> Result<u64, Error> {
        if self.status.load(Ordering::Acquire) != STATUS_READY {
            return Err(Error::message("the SQLite connection is not open"));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.execution.reserve(id)?;
        Ok(id)
    }

    fn submit(&self, job: Job) -> Result<(), Error> {
        let executor = self
            .executor
            .upgrade()
            .ok_or_else(|| Error::message("the SQLite executor is closed"))?;
        executor.submit(job).map_err(Error::from)
    }

    fn with_connection<T>(&self, run: impl FnOnce(&SQLiteConnection) -> T) -> Result<T, Error> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(run(state.connection()?))
    }

    fn release(&self, id: u64) {
        self.execution.release(id);
    }

    fn interrupt(&self, id: u64) {
        if !self.execution.cancel(id) {
            return;
        }
        if let Some(interrupt) = self
            .interrupt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            interrupt.interrupt();
        }
    }

    fn close(self: &Arc<Self>) {
        if self.status.swap(STATUS_CLOSED, Ordering::AcqRel) == STATUS_CLOSED {
            return;
        }
        let active = self.execution.active.load(Ordering::Acquire);
        if active != 0 {
            self.interrupt(active);
        }
        let active = self
            .active_result
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(result) = active.and_then(|active| active.result.upgrade()) {
            result.close();
        }

        let shared = Arc::clone(self);
        let _ = self.submit(Box::new(move || shared.close_connection()));
    }

    fn close_connection(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(connection) = state.connection.take() else {
            return;
        };
        if state.transaction {
            let _ = connection.execute_batch("ROLLBACK");
            state.transaction = false;
        }
        self.interrupt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(state);
        drop(connection);
    }
}

/// A shared asynchronous database connection.
pub struct Connection {
    shared: Arc<ConnectionShared>,
}

impl Connection {
    /// Starts opening a connection.
    ///
    /// # Errors
    ///
    /// Returns an error when the completion notifier cannot be created.
    pub fn open(
        configuration: Configuration,
        executor: &Arc<dyn Executor>,
    ) -> Result<(Self, Operation), Error> {
        let shared = ConnectionShared::opening(Arc::downgrade(executor), Notifier::new()?);
        let opening = OperationShared::new(Arc::clone(&shared), FIRST_OPERATION);
        let connection = Self {
            shared: Arc::clone(&shared),
        };
        let operation = Operation::new(Arc::clone(&opening));
        let worker_shared = Arc::clone(&shared);
        let worker_opening = Arc::clone(&opening);
        let job = Box::new(move || open(&configuration, &worker_shared, &worker_opening));
        if let Err(error) = executor.submit(job) {
            connection
                .shared
                .status
                .store(STATUS_BROKEN, Ordering::Release);
            connection.shared.release(FIRST_OPERATION);
            opening.complete(Err(Error::from(error)));
        }
        Ok((connection, operation))
    }

    fn submit(
        &self,
        run: impl FnOnce(&mut ConnectionState) -> Result<(), Error> + Send + 'static,
    ) -> Result<Operation, Error> {
        let id = self.shared.reserve()?;
        let operation = OperationShared::new(Arc::clone(&self.shared), id);
        let shared = Arc::clone(&self.shared);
        let worker_operation = Arc::clone(&operation);
        let job = Box::new(move || {
            let result = run_operation(&worker_operation, || {
                let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
                run(&mut state)
            });
            finish_operation(&shared, id, &worker_operation, result);
        });
        if let Err(error) = self.shared.submit(job) {
            self.shared.release(id);
            operation.complete(Err(error));
        }
        Ok(Operation::new(operation))
    }

    /// Starts a connection health check.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn ping(&self) -> Result<Operation, Error> {
        self.submit(|state| ping(state.connection()?))
    }

    /// Starts preparing a statement in the connection cache.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn prepare(&self, sql: String) -> Result<Operation, Error> {
        self.submit(move |state| {
            state
                .connection()?
                .prepare_cached(&sql)
                .map(|_| ())
                .map_err(Error::from)
        })
    }

    /// Starts a query and returns its result stream.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn execute(
        &self,
        sql: String,
        parameters: Vec<Value>,
    ) -> Result<(ResultSet, Operation), Error> {
        let id = self.shared.reserve()?;
        let operation = OperationShared::new(Arc::clone(&self.shared), id);
        let result = ResultShared::new(Arc::clone(&self.shared), id);
        *self
            .shared
            .active_result
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(ActiveResult {
            id,
            result: Arc::downgrade(&result),
        });
        let shared = Arc::clone(&self.shared);
        let worker_result = Arc::clone(&result);
        let worker_operation = Arc::clone(&operation);
        let job = Box::new(move || {
            let opened = shared.with_connection(|connection| {
                execute(
                    connection,
                    &sql,
                    parameters,
                    &worker_result,
                    &worker_operation,
                );
            });
            if let Err(error) = opened {
                worker_result.fail(&worker_operation, error);
            }
        });
        if let Err(error) = self.shared.submit(job) {
            result.fail(&operation, error);
        }
        Ok((ResultSet::new(result), Operation::new(operation)))
    }

    /// Starts a transaction.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn begin(&self, read_uncommitted: bool, read_only: bool) -> Result<Operation, Error> {
        self.submit(move |state| {
            begin(state.connection()?, read_uncommitted, read_only).inspect(|()| {
                state.transaction = true;
            })
        })
    }

    /// Starts committing the active transaction.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn commit(&self) -> Result<Operation, Error> {
        self.submit(|state| {
            end_transaction(state.connection()?, "COMMIT").inspect(|()| {
                state.transaction = false;
            })
        })
    }

    /// Starts rolling back the active transaction.
    ///
    /// # Errors
    ///
    /// Returns an error unless the connection is open and idle.
    pub fn rollback(&self) -> Result<Operation, Error> {
        self.submit(|state| {
            end_transaction(state.connection()?, "ROLLBACK").inspect(|()| {
                state.transaction = false;
            })
        })
    }

    /// Reports whether the connection can accept another operation.
    #[must_use]
    pub fn is_reusable(&self) -> bool {
        self.shared.status.load(Ordering::Acquire) == STATUS_READY
            && self.shared.execution.active.load(Ordering::Acquire) == 0
    }

    /// Reports whether the connection is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shared.status.load(Ordering::Acquire) == STATUS_CLOSED
    }

    /// Closes the connection.
    pub fn close(&self) {
        self.shared.close();
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shared.close();
    }
}

fn open(
    configuration: &Configuration,
    shared: &Arc<ConnectionShared>,
    opening: &Arc<OperationShared>,
) {
    if opening.cancelled.load(Ordering::Acquire) {
        shared.status.store(STATUS_CLOSED, Ordering::Release);
        shared.release(FIRST_OPERATION);
        opening.complete(Err(Error::message("the SQLite operation was cancelled")));
        return;
    }

    let connection = match open_connection(configuration, Arc::clone(&shared.execution)) {
        Ok(connection) => connection,
        Err(error) => {
            shared.status.store(STATUS_BROKEN, Ordering::Release);
            shared.release(FIRST_OPERATION);
            opening.complete(Err(error));
            return;
        }
    };
    if opening.cancelled.load(Ordering::Acquire) {
        shared.status.store(STATUS_CLOSED, Ordering::Release);
        shared.release(FIRST_OPERATION);
        opening.complete(Err(Error::message("the SQLite operation was cancelled")));
        return;
    }

    *shared
        .interrupt
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(connection.get_interrupt_handle());
    shared
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .connection = Some(connection);
    if shared
        .status
        .compare_exchange(
            STATUS_OPENING,
            STATUS_READY,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        shared.close_connection();
        shared.release(FIRST_OPERATION);
        opening.complete(Err(Error::message("the SQLite connection was closed")));
        return;
    }
    shared.release(FIRST_OPERATION);
    opening.complete(Ok(()));
}

fn open_connection(
    configuration: &Configuration,
    execution: Arc<ExecutionState>,
) -> Result<SQLiteConnection, Error> {
    let mut flags = if configuration.read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    if configuration.create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    if configuration.uri {
        flags |= OpenFlags::SQLITE_OPEN_URI;
    }
    flags |= OpenFlags::SQLITE_OPEN_NO_MUTEX;

    let connection = SQLiteConnection::open_with_flags(&configuration.path, flags)?;
    connection.busy_timeout(configuration.busy_timeout)?;
    connection.set_prepared_statement_cache_capacity(configuration.statement_cache_capacity);
    connection.pragma_update(None, "foreign_keys", configuration.foreign_keys)?;
    connection.progress_handler(
        CANCELLATION_PROGRESS_INTERVAL,
        Some(move || execution.active_is_cancelled()),
    )?;
    Ok(connection)
}

fn finish_operation(
    connection: &ConnectionShared,
    id: u64,
    operation: &OperationShared,
    result: Result<(), Error>,
) {
    connection.release(id);
    operation.complete(result);
}

fn run_operation(
    operation: &OperationShared,
    run: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    if operation.cancelled.load(Ordering::Acquire) {
        return Err(Error::message("the SQLite operation was cancelled"));
    }
    run()
}

fn ping(connection: &SQLiteConnection) -> Result<(), Error> {
    connection
        .query_row("SELECT 1", [], |_| Ok(()))
        .map_err(Error::from)
}

fn begin(
    connection: &SQLiteConnection,
    read_uncommitted: bool,
    read_only: bool,
) -> Result<(), Error> {
    connection.pragma_update(None, "read_uncommitted", read_uncommitted)?;
    connection.pragma_update(None, "query_only", read_only)?;
    if let Err(error) = connection.execute_batch("BEGIN") {
        let _ = connection.pragma_update(None, "query_only", false);
        let _ = connection.pragma_update(None, "read_uncommitted", false);
        return Err(Error::from(error));
    }
    Ok(())
}

fn end_transaction(connection: &SQLiteConnection, sql: &str) -> Result<(), Error> {
    let result = connection.execute_batch(sql).map_err(Error::from);
    let query_only = connection
        .pragma_update(None, "query_only", false)
        .map_err(Error::from);
    let read_uncommitted = connection
        .pragma_update(None, "read_uncommitted", false)
        .map_err(Error::from);
    result.and(query_only).and(read_uncommitted)
}

fn execute(
    connection: &SQLiteConnection,
    sql: &str,
    parameters: Vec<Value>,
    result: &ResultShared,
    operation: &OperationShared,
) {
    if operation.cancelled.load(Ordering::Acquire)
        || result.connection.execution.is_cancelled(result.id)
    {
        result.fail(
            operation,
            Error::message("the SQLite operation was cancelled"),
        );
        return;
    }

    let parameters = match sqlite_parameters(parameters) {
        Ok(parameters) => parameters,
        Err(error) => {
            result.fail(operation, error);
            return;
        }
    };
    let mut statement = match connection.prepare_cached(sql) {
        Ok(statement) => statement,
        Err(error) => {
            result.fail(operation, Error::from(error));
            return;
        }
    };

    if let Err(error) = bind_parameters(&mut statement, &parameters) {
        result.fail(operation, error);
        return;
    }

    let column_count = statement.column_count();
    if column_count == 0 {
        match statement.raw_execute() {
            Ok(affected_rows) => {
                result.set_metadata(Metadata {
                    columns: Vec::new(),
                    affected_rows: Some(affected_rows as u64),
                });
                result.retire();
                operation.complete(Ok(()));
                result.finish(Ok(()));
            }
            Err(error) => {
                result.fail(operation, Error::from(error));
            }
        }
        return;
    }

    let columns = statement
        .columns()
        .into_iter()
        .map(|column| Column {
            name: column.name().as_bytes().to_vec(),
            declared_type: column.decl_type().map(|name| name.as_bytes().to_vec()),
        })
        .collect();
    result.set_metadata(Metadata {
        columns,
        affected_rows: None,
    });

    let mut rows = statement.raw_query();
    let mut batch = Vec::with_capacity(RESULT_BUFFER_SIZE);
    operation.complete(Ok(()));

    loop {
        if result.connection.execution.is_cancelled(result.id) {
            finish_rows(
                result,
                &mut batch,
                Err(Error::message("the SQLite operation was cancelled")),
            );

            return;
        }

        match rows.next() {
            Ok(Some(row)) => {
                let mut values = Vec::with_capacity(column_count);
                for index in 0..column_count {
                    let value = match row.get_ref(index) {
                        Ok(value) => database_value(value),
                        Err(error) => {
                            finish_rows(result, &mut batch, Err(Error::from(error)));
                            return;
                        }
                    };

                    values.push(value);
                }
                batch.push(values);
                if batch.len() == RESULT_BUFFER_SIZE && !result.push_batch(&mut batch) {
                    result.retire();
                    return;
                }
            }
            Ok(None) => {
                finish_rows(result, &mut batch, Ok(()));
                return;
            }
            Err(error) => {
                finish_rows(result, &mut batch, Err(Error::from(error)));
                return;
            }
        }
    }
}

fn finish_rows(result: &ResultShared, batch: &mut Vec<Row>, terminal: Result<(), Error>) {
    if !result.push_batch(batch) {
        result.retire();
        return;
    }
    result.retire();
    result.finish(terminal);
}

fn bind_parameters(
    statement: &mut SQLiteStatement<'_>,
    parameters: &[SQLiteValue],
) -> Result<(), Error> {
    let count = statement.parameter_count();
    let mut required = count;
    for slot in 1..=count {
        if let Some(index) = statement.parameter_name(slot).and_then(numbered_parameter) {
            required = required.max(index);
        }
    }

    if parameters.len() != required {
        return Err(Error::from(SQLiteError::InvalidParameterCount(
            parameters.len(),
            required,
        )));
    }

    statement.clear_bindings();
    for slot in 1..=count {
        let index = statement
            .parameter_name(slot)
            .and_then(numbered_parameter)
            .unwrap_or(slot);
        statement
            .raw_bind_parameter(slot, &parameters[index - 1])
            .map_err(Error::from)?;
    }

    Ok(())
}

fn numbered_parameter(name: &str) -> Option<usize> {
    let digits = name.strip_prefix('$')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|index| *index != 0)
}

fn sqlite_parameters(values: Vec<Value>) -> Result<Vec<SQLiteValue>, Error> {
    values
        .into_iter()
        .map(|value| match value {
            Value::Null => Ok(SQLiteValue::Null),
            Value::Integer(value) => Ok(SQLiteValue::Integer(value)),
            Value::Real(value) => Ok(SQLiteValue::Real(value)),
            Value::Text(value) => String::from_utf8(value)
                .map(SQLiteValue::Text)
                .map_err(|_| Error::message("text parameters for SQLite must be valid UTF-8")),
            Value::Blob(value) => Ok(SQLiteValue::Blob(value)),
        })
        .collect()
}

fn database_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::Integer(value),
        ValueRef::Real(value) => Value::Real(value),
        ValueRef::Text(value) => Value::Text(value.to_vec()),
        ValueRef::Blob(value) => Value::Blob(value.to_vec()),
    }
}
