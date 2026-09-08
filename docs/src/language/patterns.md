# Match and Destructuring

`match` evaluates its subject once. It tests each arm in order, then evaluates
the first arm that matches.

## Literal patterns

Literal patterns use strict equality. `|` joins alternatives:

```whim
function label(mixed $value): string {
  return match ($value) {
    0 => 'zero',
    1 | 2 | 3 => 'small',
    $_ => 'other',
  };
}

assert!(label(2) == 'small');
assert!(label('2') == 'other');
```

If no arm matches, Whim throws `UnhandledMatchError`.

## Variable patterns

A variable pattern accepts any value and binds that value inside the selected
arm:

```whim
$description = match (42) {
  $value => 'value:' . $value,
};
```

`$_` is an ordinary variable. Use it when an arm needs a fallback but does not
need to read the value:

```whim
$value = null;
$label = match ($value) {
  null => 'none',
  $_ => 'some',
};

assert!($label == 'none');
```

The binding exists only in its arm. It may shadow an outer variable without
changing it.

## Type patterns

A type pattern checks the subject with `is`:

```whim
function kind(mixed $value): string {
  return match ($value) {
    int => 'integer',
    string => 'text',
    $_ => 'other',
  };
}

assert!(kind(42) == 'integer');
```

`_` is not a standalone match pattern. It remains valid as an ignored slot in
a larger type, such as `vec<_>`.

## Combining patterns with `@`

`left @ right` requires both patterns to match the same value. This lets one
side bind a value while the other checks it:

```whim
function describe(mixed $value): string {
  return match ($value) {
    $number @ int => 'int:' . $number,
    $text @ string => 'string:' . $text,
    $_ => 'other',
  };
}
```

Both sides may contain nested patterns. Bindings become available to the arm
expression after every check succeeds. A failed arm cannot leave a partial
binding or throw due to a missing collection element.

`@` takes the union on its right, so this checks either literal and binds the
result once:

```whim,ignore
$small @ 1 | 2
```

Use parentheses when each union branch has its own pattern tree:

```whim,ignore
($value @ 1) | ($value @ 2)
```

Every union branch must bind the same names in the same layout. One pattern
cannot bind the same name twice.

## Positional patterns

A parenthesized positional pattern matches a tuple or vec. Without `...`, its
length must match exactly:

```whim
function point_name(mixed $value): string {
  return match ($value) {
    ($x @ int, $y @ int) => $x . ',' . $y,
    $_ => 'not a point',
  };
}

assert!(point_name((3, 4)) == '3,4');
assert!(point_name(vec[3, 4]) == '3,4');
```

A trailing `...` permits more values. A pattern after it checks every value in
the remainder. A variable after it binds the remainder as a vec:

```whim
$total = match (vec[2, 3, 4]) {
  ($first, ...$rest) @ vec<int> => $first + length!($rest),
  $_ => 0,
};

assert!($total == 4);
```

Intersect the positional pattern with a tuple or vec type when the collection
kind matters. Since parenthesized patterns are positional, give a tuple type a
name when you need to distinguish it from a vec:

```whim
type Point = (int, int);

$point = match ((3, 4)) {
  ($x, $y) @ Point => ($x, $y),
  $_ => null,
};
```

## Vec patterns

`vec[...]` matches only a vec. Its length is exact unless it ends with `...`:

```whim
$first = match (vec[1, 2, 3]) {
  vec[$head @ int, ...int] => $head,
  $_ => 0,
};

assert!($first == 1);
```

## Dict patterns

A dict pattern uses string, integer, or boolean literal keys. `true`, `1`, and
`'1'` are distinct keys, as are `false`, `0`, and `'0'`. Without `...`, the
pattern requires the exact key set. With `...`, it permits unlisted keys:

```whim
$name = match (dict['id' => 7, 'name' => 'Ada']) {
  dict['name' => $value @ string, ...] => $value,
  $_ => 'unknown',
};

assert!($name == 'Ada');
```

A missing key rejects the arm. It does not raise `OutOfBoundsError`.

Patterns may nest on either side of `@`:

```whim
function extract(mixed $value): null|(int, string) {
  return match ($value) {
    dict['foo' => $foo @ 1 | 2, 'bar' => $bar @ !'', ...] @ dict['foo' => int, 'bar' => string, 'baz' => float, ...] => (
      $foo,
      $bar,
    ),
    $_ => null,
  };
}

$value = dict['foo' => 2, 'bar' => 'yes', 'baz' => 1.5];
assert!(extract($value) == (2, 'yes'));
```

## Rest patterns and captures

In tuple, vec, and dict patterns, `...` permits a remainder. A pattern after
`...` checks every remaining value and must not introduce bindings inside its
elements, entries, properties, alternatives, or nested rests.

Two forms can bind the whole remainder: `...$rest` and
`...$rest @ pattern`. In the second form, `pattern` checks each remaining value
and must contain no bindings. The capture goes on the left of `@`.
Parentheses do not change these rules.

```whim
$result = match (vec[1, 'two', 'three']) {
  vec[$first @ int, ...$strings @ string] => ($first, $strings),
  $_ => null,
};
assert!($result == (1, vec['two', 'three']));
```

Tuple and vec rests bind a vec. Dict rests bind a dict that preserves the
remaining keys and their order. An empty remainder passes its element checks
and binds an empty collection. If an element fails, the entire arm fails.

`...#{ value: int }` and `...$rest @ #{ value: int }` are valid.
`...#{ $value }`, `...($x, $y)`, `...vec[...$inner]`, and
`...$rest @ #{ $value }` are compile errors. Rest captures do not collect
individual fields into new collections.

## Pattern intersections

`left & right` requires both patterns to match the same value. It binds more
tightly than `@` and `|`, and can combine type checks with destructuring:

```whim
$pair = match ((3, 4)) {
  ($x, $y) & (int, int) => ($x, $y),
  $_ => null,
};
assert!($pair == (3, 4));
```

## Object patterns

`#{ name: pattern }` matches public instance properties. It uses the same exact
and open property-set rules as [object shape types](structural-types.md#object-shapes).
A missing, inaccessible, or uninitialized required property rejects the arm.
Private, protected, and static properties are ignored for exactness.

An entry may contain any pattern, including a nested shape. `#{ $value }` is
shorthand for `#{ value: $value }`; this shorthand is available only in
patterns. Write `#{ value: $other }` to bind a different name. Add `...` to allow
additional public properties.

Inside a collection rest, object patterns follow the
[rest binding rules](#rest-patterns-and-captures):
`...$rest @ #{ value: int }` captures the whole remainder, while
`...#{ $value }` is a compile error.

A named type can precede an object pattern directly. `Foo #{ $value }` has the
same matching and binding behavior as `Foo & #{ $value }`. This also works
with generic names and nested patterns:

```whim
use Whim\Result\Ok;
use Whim\Result\Err;

function describe_result(mixed $result): string {
  return match ($result) {
    Ok<string> #{ value: '' } => 'empty',
    Ok<string> #{ $value } => $value,
    Err<int> #{ error: $code @ 400..=499 } => 'client error: ' . $code,
    Err<int> #{ $error } => 'error: ' . $error,
    $_ => 'other',
  };
}

assert!(describe_result(new Ok::<string>('hello')) == 'hello');
assert!(describe_result(new Err::<int>(404)) == 'client error: 404');
```

Intersections check the left pattern before the right. Once an object pattern's
layout matches, its listed property values are captured in source order before
its nested patterns are checked. The selected arm receives those captured
values, even if resolving a nested type invokes an autoloader that changes the
original object. Bindings remain local to the selected arm; unsuccessful arms
and union alternatives do not expose partial bindings. Property names cannot
repeat within a shape, and the usual duplicate-binding and union-layout rules
apply.

## Assignment destructuring

Destructuring assignment uses tuple and dict targets, but it does not test
match arms. A mismatch throws. See [Assignment and
Indexing](../semantics/assignment.md).
