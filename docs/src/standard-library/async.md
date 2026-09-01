# Tasks and Futures

Whim runs tasks on one event loop. A task may pause while it waits for a timer,
a socket, a channel, or another task. Other ready tasks run during that pause.
This is concurrency, not parallel work on several CPU cores.

## Starting a task

`Async\spawn()` schedules a callable and returns a `Future<T>`.

```whim,norun
use Whim\Async;

$future = Async\spawn::<int>(fn(): int {
  return 21 * 2;
});

$answer = $future->await();
assert!($answer == 42);
```

The task starts on a later event-loop turn. `await()` pauses the current task
until the future succeeds or throws. If the task throws, `await()` throws the
same error.

Code must observe every future. Await it, attach a result handler, return it,
or call `ignore()` when the result does not matter. Dropping an unobserved
failed future may raise `UnhandledAwaitableException`.

## Future operations

`Future<T>` extends `Promise<T>` and provides:

- `await($cancellation): T` waits for its result.
- `map($success): Future<U>` changes a success value.
- `then($success, $failure): Future<U>` handles either result.
- `catch($failure): Future<T|U>` recovers from a failure.
- `always($callback): Future<T>` runs after either result.
- `ignore(): static` marks the result as intentionally unobserved.

These methods return a new future, except `await()` and `ignore()`. They do not
change the source future.

```whim,norun
use Whim\Async;

$length = Async\spawn::<string>(fn(): string => 'whim')
  ->map::<int>(fn(string $value): int => length!($value));

assert!($length->await() == 4);
```

## Deferred results

`Deferred<T>` owns the write side of a future. Its consumer receives only the
`Future<T>` interface.

```whim,norun
use Whim\Async\Deferred;

$deferred = new Deferred::<int>();
$future = $deferred->getFuture();
$deferred->complete(42);
assert!($future->await() == 42);
```

Call `complete($value)` for success or `error($error)` for failure. A deferred
may complete only once.

## Waiting for many tasks

`Async\all()` awaits every keyed future and returns a dict with the same keys.
It waits for all futures even when one fails. It throws the sole error or a
`CompositeException` that contains all errors.

`Async\concurrently()` starts each supplied callable, then applies `all()`.
`Async\series()` calls each callable in order without spawning tasks.

`Async\first()` returns or throws the first completed result. `Async\any()`
returns the first successful result and throws `CompositeException` if every
future fails. Both require at least one future. Futures that lose remain
observable; neither function consumes or ignores their values.

## Yielding and timers

- `Async\later()` pauses until the next event-loop turn.
- `Async\sleep($duration, $cancellation)` pauses for a duration.
- `Async\drain()` runs scheduled work until no work remains.

Use `later()` to let ready work run without adding a time delay. Use `sleep()`
for a real delay. Both may suspend the current task.

## Task-local values

`TaskLocal<T>` stores one value per task. A new task starts with no value, even
when the task that starts it has set one. Each `TaskLocal` object has its own
storage.

```whim,norun
use Whim\Async;
use Whim\Async\TaskLocal;

$requestId = new TaskLocal::<string>();
$requestId->set('main');

$future = Async\spawn::<null|string>(fn(): null|string => $requestId->get());
assert!($future->await() == null);
assert!($requestId->get() == 'main');

$requestId->clear();
assert!($requestId->get() == null);
```

Top-level code has its own task-local storage. `get()` returns `null` until the
current task calls `set()`, and again after `clear()`.

## TaskGroup

`TaskGroup` tracks tasks that return `void`.

```whim,norun
use Whim\Async\TaskGroup;

$group = new TaskGroup();
$group->defer(fn(): void {
  write_line!('first');
});
$group->defer(fn(): void {
  write_line!('second');
});
$group->awaitAll();
```

`awaitAll()` waits for every tracked task. It then throws the sole error or a
`CompositeException` with all failures. A cancellation token cancels the wait,
not the tasks already in the group.

## WaitGroup

`WaitGroup` tracks work completed elsewhere. Call `add()` before starting one
unit of work, call `done()` once it finishes, and call `wait()` to pause until
the count reaches zero. `done()` throws if the count is already zero.

`getCount()` returns the current count. A cancellation token cancels only the
wait.

## Semaphore and Sequence

`Semaphore<Tin, Tout>` limits how many calls to one operation may run at once.
Its constructor takes a positive limit and the operation. `waitFor($input)`
waits for a slot, runs the operation, and releases the slot in a `finally`
block.

`Sequence<Tin, Tout>` is a semaphore with a limit of one. It runs calls in
order. Both types can report pending work, wait for a free slot, and cancel
pending calls with a supplied error.
