# Runtime Type Checks

Whim keeps type data while a program runs. It checks types at each boundary
that has a declared type.

```whim
function double(int $value): int {
  return $value * 2;
}

assert!(double(4) == 8);
```

Calling `double('4')` throws `Whim\Unwind\TypeError`. Whim does not turn the
string into an integer.

## Where Whim checks types

Whim checks:

- function and method arguments;
- return values;
- property defaults and writes;
- static properties and constants;
- enum backing values;
- generic type arguments and their bounds;
- collection writes when a typed place holds the collection;
- `is`, `as`, `?as`, match patterns, and typed catch clauses;
- class, interface, and override contracts.

The compiler may prove that a check will pass and remove it. This changes no
result. A check remains when the compiler cannot prove the type.

## Type errors do not change the old value

Whim prepares a write, runs the operation, checks the result, then stores it.
If any step throws, the old place stays unchanged.

```whim,norun
class Counter {
  public int $value = 1;
}

$counter = new Counter();
$counter->value = 'wrong'; // throws; value remains 1
```

This rule also covers nested array writes and compound assignments.

## `is`

`$value is T` returns `true` when the value fits `T`.

```whim
function describe(int|string $value): string {
  if ($value is int) {
    return 'integer ' . $value;
  }

  return 'string ' . $value;
}
```

The value expression runs once. The type may contain aliases, ranges,
collections, type parameters, and symbol types.

An unknown name may run the autoloader. If the name remains unknown, the check
returns `false` instead of throwing. A later declaration can make a later check
match.

Whim permits some bare generic names in a class test:

```whim
final class Box<T> {}

$box = new Box::<string>();
assert!($box is Box);
assert!($box is Box<_>);
```

Built-in collection names need their full number of type arguments in a runtime
check. Write `vec<_>`, `dict<_, _>`, or `array<_, _>`, not bare `vec`, `dict`,
or `array`.

## `as`

`$value as T` checks `T` and returns the same value. It throws `TypeError` on a
failed check.

```whim
$value = 42 as int;
```

The cast does not convert scalar values. `1 as float` fails.

A cast to a newtype adds that newtype's tag after checking the backing type.

## `?as`

`$value ?as T` returns the checked value or `null`.

```whim
$raw = 'not an integer';
$timeout = ($raw ?as int) ?? 30;
```

This form does not throw for a type mismatch. It may still throw while
evaluating `$value`.

## Deep checks

Collection and generic checks inspect their parts when the type calls for it.

```whim
$values = vec[1, 2];
assert!($values is vec<int>);

$values[] = 'changed';
assert!(!($values is vec<int>));
assert!($values is vec<int|string>);
```

The same rule applies to dict keys and values, tuple positions, shape types,
object type arguments, and newtype backing values.

An empty vec has type `vec<never>`. An empty dict has type
`dict<never, never>`. Since `never` fits every type, empty collections fit any
compatible element type.

## Reified generic types

An object keeps its type arguments:

```whim
final class Box<T> {
  public function __construct(public T $value) {}
}

$box = new Box::<int>(1);
assert!($box is Box<int>);
assert!(!($box is Box<string>));
```

Function and method calls also keep their bound type arguments for the length
of the call. Code may test a value against a type parameter with `is T`.

## Relative class types

Inside a class:

- `self` is the declaring class with its bound type arguments;
- `parent` is its direct parent;
- `static` is the runtime class on which the call began.

`static` may appear as a return type but not a parameter type. Use `self` for a
parameter that accepts the declaring class and its children.

## Type identifiers

`Whim\Type` gives code an opaque integer key for a type.

```whim
use Whim\Type;

function same_type<T>(T $value): bool {
  return Type\of($value) == Type\id::<T>();
}

assert!(same_type::<int>(42));
```

`Type\of($value)` describes the value's runtime type. `Type\id::<T>()`
describes `T`. Equal types have equal identifiers in one engine.

Use a type identifier only for equality or as an array key. Its integer value
has no public meaning. Do not store it for another process or run.

## Loading and checks

Whim can compile a type name before that symbol loads. Once an autoloader or a
file declares the symbol, later checks use its real type. This is why Whim does
not require all type names to exist when it first parses a file.

Links between declared classes and interfaces are stricter. Whim checks their
inheritance and member contracts when it links them.
