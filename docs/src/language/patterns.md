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

Both sides may contain nested patterns. Whim performs every check before it
creates any binding. A failed arm cannot leave a partial binding or throw due
to a missing collection element.

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

A dict pattern uses literal keys. Without `...`, it requires the exact key set.
With `...`, it permits unlisted keys:

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

## Assignment destructuring

Destructuring assignment uses tuple and dict targets, but it does not test
match arms. A mismatch throws. See [Assignment and
Indexing](../semantics/assignment.md).
