# Throwing and Catching

`throw` stops the current path with an object that implements
`Whim\Unwind\Throwable`.

```whim
use Whim\Unwind\InvalidArgumentException;

function divide(int $left, int $right): float {
  if ($right == 0) {
    throw new InvalidArgumentException('division by zero');
  }

  return $left / $right;
}
```

Throwing a scalar or an object that does not implement `Throwable` raises
`TypeError` instead.

## Throwable data

`Whim\Unwind\Error` and `Whim\Unwind\Exception` are the two built-in roots.
Both implement `Throwable` and expose:

- `getMessage(): string`
- `getCode(): int`
- `getFile(): string`
- `getLine(): int`
- `getTrace(): vec<TraceFrame>`
- `getPrevious(): null|Throwable`
- `toString(): string`

Their constructor accepts a message, an integer code, and an optional previous
throwable.

```whim
use Whim\Unwind\RuntimeException;

$cause = new RuntimeException('connection failed');
$error = new RuntimeException('request failed', 0, $cause);
```

Whim records the file, line, and trace when it creates the throwable. Throwing
the same object later does not replace that trace.

A trace frame contains the function name, file, line, and argument values. A
marker attribute may hide a sensitive parameter.

## Errors and exceptions

`Error` reports a language, compiler, or runtime contract failure. Examples
include `TypeError`, `OutOfBoundsError`, `ReadonlyError`, and
`UndefinedSymbolError`.

`Exception` reports a failure that application code may handle.
`LogicException` and `RuntimeException` are its two main groups.

Both use the same `throw` and `catch` rules. The split tells readers whether a
failure points to a broken program rule or an expected outside condition.

## `try` and `catch`

`try` must have at least one `catch`, `else`, or `finally` clause.

```whim
use Whim\Unwind\InvalidArgumentException;

try {
  throw new InvalidArgumentException('bad value');
} catch (InvalidArgumentException $error) {
  write_line!($error->getMessage());
}
```

Whim tests catch clauses in source order. A clause may omit its variable:

```whim
try {
  throw new Whim\Unwind\RuntimeException('failed');
} catch (Whim\Unwind\RuntimeException) {
  write_line!('failed');
}
```

A union catches more than one type:

```whim
use Whim\Unwind\RuntimeException;

final class TimeoutException extends RuntimeException {}
final class NetworkException extends RuntimeException {}

function work(): void {}

function recover(TimeoutException|NetworkException $error): void {}

try {
  work();
} catch (TimeoutException|NetworkException $error) {
  recover($error);
}
```

If no clause matches, Whim unwinds to an outer `try`. A throw from a catch body
also moves outward; sibling catches do not see it.

## Catch guards

Add `if` after a catch to test the matched value:

```whim
use Whim\Unwind\RuntimeException;

try {
  throw new RuntimeException('missing', 404);
} catch (RuntimeException $error) if ($error->getCode() == 404) {
  write_line!('not found');
} catch (RuntimeException $error) {
  write_line!('other failure');
}
```

Whim runs a guard only after its type matches. A false guard moves to the next
clause. If the guard throws, that new throwable replaces the current one and
moves outward.

## `else`

A `try` `else` block runs only when the try body reaches its end without a
throw, `return`, `break`, or `continue`.

```whim
$completed = false;

try {
  write_line!('work');
} else {
  $completed = true;
}

assert!($completed);
```

It does not run after a catch. A throw from `else` moves outward; sibling
catches do not see it.

Variables assigned in the try body are available in `else` under the normal
scope rules.

## `finally`

`finally` runs after the try body and any matching catch or `else`. It also runs
before `return`, `break`, or `continue` transfers control out of the protected
code.

```whim
try {
  write_line!('work');
} finally {
  write_line!('cleanup');
}
```

If `finally` throws, its throwable replaces a pending return or throwable.
Nested `finally` blocks each run once while control moves outward.

Whim checks a pending return value after it runs `finally`. Cleanup still runs
when the return later fails its declared type.

`exit!` ends normal execution. It does not run enclosing catch, `else`, or
`finally` clauses. Whim still runs shutdown destructors.

`panic!('message')` does the same, but first writes the message and current
stack trace to standard error. It always exits with status 255. It is for a
broken invariant, not an error a caller can handle.

## Uncaught throwables

An uncaught throwable ends the program with exit code 255. Whim writes its
class, message, code, source location, notes, and stack trace to standard error.

Set `WHIM_FULL_TRACE=true` when the normal trace hides frames marked as
boundaries.

## Catch narrowly

Catch the type that the current code can handle. Catching `Throwable` and then
continuing can hide type errors, broken invariants, and resource leaks. Let an
unknown failure keep its original trace.
