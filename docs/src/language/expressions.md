# Expressions

Most expressions produce a value. Literals, variables, arrays, operators,
calls, member access, object construction, closures, casts, assignments, and
`match` are expressions. `return`, `throw`, `break`, and `continue` are
expressions that do not complete normally.

## Variables and constants

Reading an unassigned variable throws `UndefinedVariableError`. An assignment
creates the variable:

```whim
$value = 42;
assert!($value == 42);
```

A constant uses its bare name. A class constant uses `::`:

```whim
const LIMIT = 10;

final class Defaults {
  public const int RETRIES = 3;
}

assert!(LIMIT == 10);
assert!(Defaults::RETRIES == 3);
```

## Object creation

`new` calls a class constructor:

```whim
final class Point {
  public function __construct(public int $x, public int $y) {}
}

$point = new Point(3, 4);
assert!($point->x == 3);
```

You may omit parentheses when the constructor takes no argument: `new Point` and
`new Point()` are equal calls.

Use `::<...>` between a generic class name and its arguments:

```whim
final class Box<T> {
  public function __construct(public T $value) {}
}

$box = new Box::<string>('value');
assert!($box->value == 'value');
```

A class-name string may drive construction:

```whim
final class Token {}

$class = 'Token';
$token = new $class();
assert!($token is Token);
```

The value must name a declared concrete class. Use `classname<T>` at a typed
boundary when the named class must fit a base type.

## Member access

`->` reads an instance property or calls an instance method. `::` reads a
static property, constant, enum case, or static method.

`?->` stops when the receiver is null. It returns null and does not evaluate
the call arguments:

```whim
final class User {
  public function __construct(public string $name) {}
}

$user = null;
assert!($user?->name == null);
```

The receiver must otherwise be an object of the right type. Whim has no dynamic
property creation.

## Type tests and casts

`is` returns a bool:

```whim
$value = 42;
assert!($value is int);
assert!(!($value is string));
```

`as` returns the same value or throws `TypeError`:

```whim
$value = 42 as int;
assert!($value == 42);
```

`?as` returns null instead of throwing:

```whim
$value = 'forty-two' ?as int;
assert!($value == null);
```

Whim keeps types at runtime, so these operators work with unions, ranges,
generic types, aliases, newtypes, collection shapes, and symbol types.

## Coalescing

`$left ?? $right` returns the left value unless it is null. It evaluates the
right side only for null:

```whim
assert!((0 ?? 10) == 0);
assert!((false ?? true) == false);
assert!((null ?? 10) == 10);
```

The left expression still runs. A missing dict key or bad property access
throws before `??` can inspect a value.

## Pipeline

`|>` calls the callable on its right with the left value:

```whim
function double(int $value): int {
  return $value * 2;
}

$answer = 21 |> double(...);
assert!($answer == 42);
```

Both sides run once. A partial callable may choose the input position:

```whim
function add(int $left, int $right): int {
  return $left + $right;
}

assert!((3 |> add(?, 4)) == 7);
```

Whim has no ternary operator. Use `if` for statements or `match` for a value.

## `return`

`return` ends the current function, method, or closure. It may carry the value
given back to the caller:

```whim
function classify(bool $ready): string {
  return match ($ready) {
    true => 'ready',
    false => return 'waiting',
  };
}

assert!(classify(false) == 'waiting');
```

A bare `return` gives back null. A `void` callable cannot return a value, and a
`never` callable cannot return. `return` is not valid at file scope or inside a
`finally` block.

Returning runs each active `finally` block and releases each active `using`
value before the caller resumes.

## `break` and `continue`

`break` leaves the nearest loop. `continue` starts its next pass. Neither
produces a value because control leaves the expression.

```whim
$values = vec[];
for ($index = 0; $index < 5; $index++) {
  $value = match ($index) {
    1 | 3 => continue,
    4 => break,
    $_ => $index,
  };

  $values[] = $value;
}

assert!($values == vec[0, 2]);
```

Both run any active `finally` blocks and release any active `using` values on
the way out.

An integer literal chooses an outer loop:

```whim
$count = 0;
for ($x = 0; $x < 3; $x++) {
  for ($y = 0; $y < 3; $y++) {
    $count++;
    break 2;
  }
}

assert!($count == 1);
```

The level must be greater than zero and cannot exceed the number of enclosing
loops. Omitting it means `1`.

## `throw`

`throw` evaluates an object that implements `Whim\Unwind\Throwable`, then
starts unwinding. It does not produce a value because the current path ends.
This makes it useful in expressions such as a coalescing fallback:

```whim
use Whim\Unwind\RuntimeException;

function cached_or_fail(null|string $cached): string {
  return $cached ?? throw new RuntimeException('value is not cached');
}

assert!(cached_or_fail('ready') == 'ready');
```

See [Throwing and Catching](../semantics/error-handling.md) for exception types,
catching, and cleanup.
