# Structural Types

Whim can describe the parts of arrays, objects, and callables at runtime.

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

Keys may be string, integer, or boolean literals. As in dict values, `true`,
`1`, and `'1'` are distinct keys, as are `false`, `0`, and `'0'`.

```whim
type UserRow = dict['id' => int, 'name' => string];

$user = dict['id' => 1, 'name' => 'Ada'];
assert!($user is UserRow);
```

```whim
type Partition<T> = dict[true => vec<T>, false => vec<T>];

$partition = dict[true => vec[1, 3], false => vec[2, 4]];
assert!($partition is Partition<int>);
assert!(!(dict[1 => vec[1, 3], 0 => vec[2, 4]] is Partition<int>));
```

Without a rest entry, the dict must have only the listed keys. A rest entry
allows other keys and gives them a key and value type:

```whim
type Named = dict['name' => string, ...];
type ScoredUser = dict['id' => int, 'name' => string, ...<string, int|float>];

$user = dict['id' => 1, 'name' => 'Ada', 'score' => 9.5];
assert!($user is ScoredUser);
```

## Object shapes

`#{ name: string }` accepts an object with exactly one public instance property,
`name`, whose current value is a string. Add a final `...` to allow other public
instance properties:

```whim
type NamedObject = #{ name: string, ... };
type WithValue<T> = #{ value: T, ... };

class Person {
  public string $name = 'Ada';
  public int $age = 36;
}

$person = new Person();
assert!($person is NamedObject);
assert!($person is #{ age: int, name: string });
assert!(!($person is #{ name: string }));
```

Property names are case-sensitive identifiers, without `$`. Required properties
must exist, be public instance properties, be initialized, and have values that
match their corresponding types. Inherited public properties count. Private,
protected, and static properties do not participate, even when the check runs
inside their declaring class. An uninitialized required property rejects a
check; it does not throw an uninitialized-property error. An extra public
property still counts toward exactness when it is uninitialized.

`#{}` accepts objects with no public instance properties. `#{ ... }` accepts
any object, just like `object`. Neither form accepts an array or another
non-object value. Object shapes have a bare `...` rest marker; unlike dict
shapes, they do not have a typed rest entry. Property names cannot repeat.
Trailing commas are allowed.

Shapes check current values, independently of a property's declared type. A
`mixed` property holding an integer can satisfy an `int` entry. A successful
check does not change the property's declared type, visibility, or readonly
rules. Mutating the object can make a later check fail, including when the
object is held inside a vec, dict, or tuple.

Place a shape after a named type to combine the nominal and structural checks.
`Name #{ ... }` is shorthand for `Name & #{ ... }`, including generic names:

```whim
use Whim\Result\Ok;

$value = new Ok::<string>('hello');
assert!($value is Ok<string> & #{ value: string & !'' });
assert!($value is Ok<string> #{ value: string & !'' });
```

The name and shape form one type: `!Name #{ value: int }` negates the entire
combination, like `!(Name & #{ value: int })`. Closed and open property-set
rules apply equally to both spellings.

Shapes work in aliases, generic arguments and bounds, callable signatures,
parameter and return types, property types, `is`, `as`, and `?as`. Their
subtyping rules compare required names independently of source order and allow
narrower property value types. An open expected shape allows extra properties;
a closed expected shape requires the same public property set and a closed
actual shape. Every object shape is a subtype of `object`. A class name alone
does not promise that its properties are initialized with the required current
values; use a shape or an intersection to express that requirement.

The opening `#{` is one token. Whitespace or a comment cannot separate `#` from
`{`. Object shapes are types and patterns, not object construction expressions.
See [Match and Destructuring](patterns.md#object-patterns) for extracting their
properties in a match arm.

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
