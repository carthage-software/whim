# Value Semantics

Whim treats scalars, strings, and arrays as values. Objects and callables have
identity.

## Array assignment

Assigning an array creates an independent value:

```whim
$first = vec[1, 2];
$second = $first;
$second[] = 3;

assert!($first == vec[1, 2]);
assert!($second == vec[1, 2, 3]);
```

The runtime may share array storage until one value changes. Code cannot
observe that sharing.

Array parameters follow the same rule:

```whim
function append_zero(vec<int> $values): vec<int> {
  $values[] = 0;
  return $values;
}

$source = vec[1];
$result = append_zero($source);

assert!($source == vec[1]);
assert!($result == vec[1, 0]);
```

## Nested arrays

A write through mutable containers updates the outer value:

```whim
$grid = vec[vec[1, 2]];
$grid[0][1] = 20;
assert!($grid == vec[vec[1, 20]]);
```

A tuple blocks every write through its positions, even when one position holds
a vec.

## Object identity

Object assignment keeps the same object:

```whim
final class Cell {
  public int $value = 0;
}

$first = new Cell();
$second = $first;
$second->value = 42;

assert!($first->value == 42);
```

Copying an array does not clone the objects inside it:

```whim
final class Cell {
  public int $value = 0;
}

$cell = new Cell();
$left = vec[$cell];
$right = $left;
$right[0]->value = 7;

assert!($left[0]->value == 7);
```

The arrays are distinct values. Both hold the same object.

## Closure captures

A short closure captures each outer variable it uses. It captures the current
value. An object value still points to the same object:

```whim
final class Cell {
  public int $value = 0;
}

$number = 1;
$cell = new Cell();
$read = fn(): (int, int) {
  return ($number, $cell->value);
};

$number = 2;
$cell->value = 3;

assert!($read() == (1, 3));
```

## Lifetime

A value stays alive while a strong reference can reach it. The runtime also
finds unreachable cycles.

`using`, `drop!`, destructors, weak references, and cycle collection add rules
for resources. [Resources and Cleanup](../core-library/overview.md) and
[References and Cycles](../core-library/references.md) cover those rules.
