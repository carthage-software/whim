# Collection and Callable Types

Whim can describe the parts of arrays and callables at runtime.

## Homogeneous vecs

`vec<T>` accepts a vec whose every value fits `T`.

```whim
function total(vec<int> $values): int {
  $sum = 0;
  foreach ($values as $value) {
    $sum += $value;
  }

  return $sum;
}
```

The vec's runtime element type changes as code adds or removes values. An empty
vec has element type `never`, so it fits `vec<T>` for any `T`.

Bare `vec` accepts a vec without checking its items at a declared boundary.
The matching forms `is vec`, `as vec`, and a bare `vec` match pattern are not
valid; write `vec<_>` when an explicit runtime check may accept any items.

## Vec shapes

`vec[T0, T1]` describes fixed positions and an exact length.

```whim
function pair(vec[int, string] $value): void {}

pair(vec[42, 'answer']);
```

A final `...T` allows zero or more extra values of `T`:

```whim
type Row = vec[string, ...int];

assert!(vec['row'] is Row);
assert!(vec['row', 1, 2] is Row);
```

## Homogeneous dicts

`dict<K, V>` checks every key against `K` and every value against `V`.

```whim
function scores(dict<string, int> $scores): void {}

scores(dict['Ada' => 10, 'Grace' => 12]);
```

Dict keys can be `int`, `string`, or `bool`. The type may use one of them, a
union, a range, or another type that fits those key kinds.

An empty dict has type `dict<never, never>` and fits every valid dict key and
value type.

Bare `dict` and `array` follow the same boundary rule. In an explicit runtime
check, use `dict<_, _>` or `array<_, _>`.

## Dict shapes

A dict shape lists required keys and their value types.

```whim
type UserRow = dict['id' => int, 'name' => string];

$user = dict['id' => 1, 'name' => 'Ada'];
assert!($user is UserRow);
```

Without a rest entry, the dict must have only the listed keys. A rest entry
allows other keys and gives them a key and value type:

```whim
type ScoredUser = dict['id' => int, 'name' => string, ...<string, int|float>];

$user = dict['id' => 1, 'name' => 'Ada', 'score' => 9.5];
assert!($user is ScoredUser);
```

## Tuple types

`(A, B)` describes an exact tuple length and each position.

```whim
type Coordinate = (float, float);

function move(Coordinate $point): Coordinate {
  return ($point[0] + 1.0, $point[1] + 1.0);
}
```

A one-item tuple type has a trailing comma: `(T,)`.

A final `...T` accepts zero or more trailing items of `T`:

```whim
type Delivery = (int, int, ...string);

assert!((41, 99) is Delivery);
assert!((41, 99, 'fragile', 'signed') is Delivery);
```

The rest item must be last. Omitting its type, as in `(int, ...)`, uses
`mixed`. A tuple has at least one and at most twelve fixed items.

## The common array type

`array<K, V>` accepts a tuple, vec, or dict whose keys fit `K` and values fit
`V`.

```whim
function count_values(array<_, _> $values): int {
  return length!($values);
}

assert!(count_values(vec[1, 2]) == 2);
assert!(count_values(dict['one' => 1]) == 1);
assert!(count_values((1, 'two')) == 2);
```

For a vec, keys are non-negative integers. For a tuple, keys form the range of
its positions. For a dict, keys keep their declared kinds.

`array<K, V>` is a read-only type view. It does not change the value into a new
array form.

## Callable types

`fn(A, B): R` describes a callable that accepts `A` and `B` and returns `R`.

```whim
function apply(fn(int): string $format, int $value): string {
  return $format($value);
}

$result = apply(fn(int $value): string => 'n=' . $value, 42);
```

A leading `=` marks an optional parameter in a callable type:

```text
fn(int, =string): bool
```

This type accepts a callable that lets callers omit its second argument.

Bare `fn` accepts any callable.

Callable parameters are contravariant. A callable that accepts `mixed` can
stand in for one that needs only `int`. Return types are covariant. A callable
that returns `int` can stand in for one that may return `int|string`.

Callable values also carry their generic binding and origin. The [symbols as
types chapter](symbol-types.md) covers specific function and method families.

## Class-name types

`classname<T>` accepts a string that names a class whose instances fit `T`.

```whim
interface Shape {}
class Circle implements Shape {}

function make(classname<Shape> $class): Shape {
  return new $class();
}

$shape = make('Circle');
```

The string may include reified type arguments, such as `Box<int>`. A child class
name also fits a parent or interface bound. A missing class or wrong type
argument fails the check.

The inner type must be able to contain a class-like type. `classname<int>` is
invalid.

## Wildcards

`_` means that a nested type exists but its fixed value does not matter.

```whim
assert!(vec[1, 'two'] is vec<_>);
assert!(dict['one' => 1] is dict<_, _>);
```

It can select some generic arguments while keeping others fixed:

```whim
final class Pair<A, B> {}

$pair = new Pair::<string, int>();
assert!($pair is Pair<_, int>);
```

It also works in tuple positions and callable parameters.

`_` cannot stand alone as a type. Whim does not allow `!_`. It also does not
mean `mixed`: `Box<_>` accepts a `Box<string>`, while `Box<mixed>` follows the
class's variance and may reject it.
