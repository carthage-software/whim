use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::ActiveResult;
use crate::Configuration;
use crate::Connection;
use crate::ConnectionShared;
use crate::Error;
use crate::ExecutionState;
use crate::Executor;
use crate::FIRST_OPERATION;
use crate::Job;
use crate::Metadata;
use crate::Notifier;
use crate::Operation;
use crate::PoisonError;
use crate::ResultSet;
use crate::ResultShared;
use crate::Row;
use crate::Value;
use crate::numbered_parameter;
use crate::open_connection;

struct ExecutorStub;

impl Executor for ExecutorStub {
    fn submit(&self, job: Job) -> io::Result<()> {
        drop(job);
        Ok(())
    }
}

struct ThreadExecutor;

impl Executor for ThreadExecutor {
    fn submit(&self, job: Job) -> io::Result<()> {
        thread::Builder::new().spawn(job).map(drop)
    }
}

#[derive(Default)]
struct ManualExecutor {
    jobs: Mutex<VecDeque<Job>>,
}

impl ManualExecutor {
    fn run_next(&self) {
        let job = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(job) = job else {
            panic!("the manual SQLite executor has no pending job");
        };
        job();
    }
}

impl Executor for ManualExecutor {
    fn submit(&self, job: Job) -> io::Result<()> {
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(job);
        Ok(())
    }
}

fn wait(operation: &Operation) -> Result<(), Error> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(result) = operation.poll() {
            return result;
        }
        assert!(Instant::now() < deadline, "database operation timed out");
        thread::yield_now();
    }
}

fn collect(result: &ResultSet) -> Result<Vec<Row>, Error> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut rows = Vec::new();
    loop {
        if let Some(row) = result.poll_row() {
            match row? {
                Some(row) => rows.push(row),
                None => return Ok(rows),
            }
        }
        assert!(Instant::now() < deadline, "database result timed out");
        thread::yield_now();
    }
}

fn query(
    connection: &Connection,
    sql: &str,
    parameters: Vec<Value>,
) -> Result<(Metadata, Vec<Row>), Error> {
    let (result, operation) = connection.execute(sql.to_string(), parameters)?;
    wait(&operation)?;
    let Some(metadata) = result.metadata() else {
        return Err(Error::message("query metadata is missing"));
    };
    let rows = collect(&result)?;
    Ok((metadata, rows))
}

fn memory_configuration() -> Configuration {
    Configuration {
        path: PathBuf::from(":memory:"),
        read_only: false,
        create: true,
        uri: false,
        busy_timeout: Duration::from_secs(1),
        foreign_keys: true,
        statement_cache_capacity: 16,
    }
}

fn memory_connection() -> (Arc<dyn Executor>, Connection) {
    let executor: Arc<dyn Executor> = Arc::new(ThreadExecutor);
    let configuration = memory_configuration();
    let Ok((connection, operation)) = Connection::open(configuration, &executor) else {
        panic!("could not start opening the in-memory database");
    };
    if let Err(error) = wait(&operation) {
        panic!("could not open the in-memory database: {error}");
    }
    (executor, connection)
}

#[test]
fn interrupt_before_query_start_is_not_lost() {
    let executor = Arc::new(ManualExecutor::default());
    let erased: Arc<dyn Executor> = Arc::clone(&executor) as Arc<dyn Executor>;
    let configuration = memory_configuration();
    let Ok((connection, opening)) = Connection::open(configuration, &erased) else {
        panic!("could not start opening the in-memory database");
    };
    executor.run_next();
    if let Err(error) = wait(&opening) {
        panic!("could not open the in-memory database: {error}");
    }

    let Ok((result, operation)) = connection.execute("SELECT 1".to_string(), Vec::new()) else {
        panic!("could not start the test query");
    };
    result.interrupt();
    executor.run_next();

    assert!(wait(&operation).is_err());
    assert!(collect(&result).is_err());
    assert!(connection.is_reusable());
}

#[test]
fn progress_handler_observes_an_early_interrupt() {
    let execution = Arc::new(ExecutionState::new());
    let Ok(connection) = open_connection(&memory_configuration(), Arc::clone(&execution)) else {
        panic!("could not open the in-memory database");
    };
    assert!(execution.cancel(FIRST_OPERATION));

    let query = "WITH RECURSIVE counter(value) AS (\
        SELECT 1 UNION ALL \
        SELECT value + 1 FROM counter WHERE value < 1000000\
    ) SELECT sum(value) FROM counter";
    let result = connection.query_row(query, [], |row| row.get::<_, i64>(0));
    let Err(error) = result else {
        panic!("the early interrupt did not cancel the SQLite query");
    };
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::OperationInterrupted)
    );
}

#[test]
fn retiring_an_old_result_preserves_a_newer_registration() {
    let executor: Arc<dyn Executor> = Arc::new(ExecutorStub);
    let Ok(notifier) = Notifier::new() else {
        panic!("could not create the result notifier");
    };
    let connection = ConnectionShared::opening(Arc::downgrade(&executor), notifier);
    let old = ResultShared::new(Arc::clone(&connection), FIRST_OPERATION);
    let newer = ResultShared::new(Arc::clone(&connection), FIRST_OPERATION + 1);
    *connection
        .active_result
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(ActiveResult {
        id: FIRST_OPERATION + 1,
        result: Arc::downgrade(&newer),
    });

    old.retire();

    let (id, actual) = {
        let active_guard = connection
            .active_result
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(active) = active_guard.as_ref() else {
            panic!("retiring an old result cleared the newer registration");
        };
        let Some(actual) = active.result.upgrade() else {
            panic!("the newer result registration was dropped");
        };
        let id = active.id;
        drop(active_guard);
        (id, actual)
    };
    assert_eq!(id, FIRST_OPERATION + 1);
    assert!(Arc::ptr_eq(&actual, &newer));
}

#[test]
fn numbered_parameters_use_their_declared_indexes() {
    assert_eq!(numbered_parameter("$1"), Some(1));
    assert_eq!(numbered_parameter("$42"), Some(42));
    assert_eq!(numbered_parameter("$0"), None);
    assert_eq!(numbered_parameter("$name"), None);
    assert_eq!(numbered_parameter(":1"), None);
}

#[test]
fn in_memory_queries_stream_typed_rows() {
    let (_executor, connection) = memory_connection();
    let Ok(_) = query(
        &connection,
        "CREATE TABLE item (id INTEGER, name TEXT, payload BLOB)",
        Vec::new(),
    ) else {
        panic!("could not create the test table");
    };
    let Ok((insert, rows)) = query(
        &connection,
        "INSERT INTO item (id, name, payload) VALUES ($2, $1, $3)",
        vec![
            Value::Text(b"alpha".to_vec()),
            Value::Integer(7),
            Value::Blob(vec![0, 1, 2]),
        ],
    ) else {
        panic!("could not insert the test row");
    };
    assert_eq!(insert.affected_rows, Some(1));
    assert!(rows.is_empty());

    let Ok((metadata, rows)) = query(
        &connection,
        "SELECT id, name, payload FROM item",
        Vec::new(),
    ) else {
        panic!("could not query the test row");
    };
    assert_eq!(metadata.columns.len(), 3);
    assert_eq!(metadata.columns[0].name, b"id");
    assert_eq!(rows.len(), 1);
    assert!(matches!(rows[0][0], Value::Integer(7)));
    assert!(matches!(&rows[0][1], Value::Text(value) if value == b"alpha"));
    assert!(matches!(&rows[0][2], Value::Blob(value) if value == &[0, 1, 2]));
}

#[test]
fn queries_cross_result_batches_without_losing_rows() {
    let (_executor, connection) = memory_connection();
    let Ok((_, rows)) = query(
        &connection,
        "WITH RECURSIVE counter(value) AS (\
            SELECT 1 UNION ALL \
            SELECT value + 1 FROM counter WHERE value < 130\
        ) SELECT value FROM counter",
        Vec::new(),
    ) else {
        panic!("could not query rows across batch boundaries");
    };

    assert_eq!(rows.len(), 130);
    assert!(matches!(rows[0][0], Value::Integer(1)));
    assert!(matches!(rows[64][0], Value::Integer(65)));
    assert!(matches!(rows[129][0], Value::Integer(130)));
}
