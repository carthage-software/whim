# Databases

`Whim\Database` defines one async contract for SQLite, PostgreSQL, custom
drivers, and connection pools. It accepts raw parameterized SQL and does not
include a query builder.

## Values and rows

`Database\Value` is `null|bool|int|float|string|Blob`. `Blob` is a newtype over
string, so text and binary parameters stay distinct. A `Row` is a vec of values.

`Column` stores result-column name and database type data.

## Connectors and connections

A `Connector` opens a `Connection` with an optional cancellation token. A
connection can:

- execute parameterized SQL
- prepare a statement
- begin a transaction
- check the server with `ping`
- report whether it is safe to reuse
- close

SQL parameters use `$1`, `$2`, and so on. The number controls binding; first
appearance does not. Pass values in numeric placeholder order.

```whim,norun
use Whim\Database;
use Whim\Database\PostgreSQL\Connector;

using ($connection = new Connector('dbname=app')->connect()) {
  using ($result = $connection->execute(
    'SELECT id, name FROM users WHERE id = $1',
    vec[42],
  )) {
    $row = $result->fetch();
  }
}
```

Prepared `Statement::execute()` accepts only its values and cancellation.
Statements and results are closeable.

## Results

A result exposes column metadata, an affected-row count when the operation has
one, and streaming `fetch()`. Fetch returns one row or `null` at the end.

Helpers cover common shapes:

- `fetch_one` returns one row or `null` and rejects extra rows.
- `fetch_all` returns every row.
- `fetch_value` returns one selected value.
- `transactional` begins, calls a function, commits on success, and rolls back
  on failure.

Close a result after an early stop so its connection can run another operation.

## Transactions

`begin($isolation, $readOnly, $cancellation)` starts a transaction. A null
isolation uses the database default. `TransactionIsolation` has read
uncommitted, read committed, repeatable read, and serializable levels, though a
driver may reject a level its database cannot provide.

A transaction implements `Executor` and adds `isActive`, `commit`, `rollback`,
and close. Closing an active transaction rolls it back.

## Pool

`ConnectionPool` wraps a connector. `connect()` checks out a lease that still
implements `Connection`; closing it returns a reusable connection to the pool.

`PoolConfiguration` controls:

- maximum open connections
- maximum idle connections
- idle timeout
- total connection lifetime
- validation interval
- acquisition timeout

The default acquisition timeout is 30 seconds. A null timeout waits without a
pool deadline, though the caller's cancellation token can still end the wait.

The pool reports open, idle, and waiting counts. Closing it closes idle
connections and fails pending acquisitions. Checked-out leases close or return
when their owners finish.

## SQLite

`Database\SQLite\Connector` opens a path or URI. `inMemory()` creates a private
in-memory database.

Configuration sets read-only and create behavior, URI parsing, busy timeout,
statement-cache size, and foreign-key enforcement. SQLite work runs through the
shared bounded blocking pool, so file work does not stop the event loop.

Each connection permits one active operation. The driver waits for its prior
worker operation to retire before reuse; application code does not retry a
private busy state.

## PostgreSQL

`Database\PostgreSQL\Connector` accepts a libpq connection string, which is
marked sensitive in traces. It uses non-blocking libpq socket progress with the
Whim event loop.

PostgreSQL errors preserve SQLSTATE, detail, and hint on `Database\Exception`.
Call `getSQLState()` instead of matching error text.

## Errors

The hierarchy separates connection, query, and transaction failures.
`AcquisitionTimeoutException` means a pool wait expired.
`ConcurrentOperationException` means code attempted overlapping work on one
connection. Database server "busy" or lock errors remain query errors with
their database code.
