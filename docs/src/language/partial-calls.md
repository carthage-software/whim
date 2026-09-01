# First-Class and Partial Calls

Whim can turn a known function or method into a callable without wrapping it in
a closure.

## First-class functions

Write `(...)` in place of the argument list:

```whim
function square(int $value): int {
  return $value * $value;
}

$square = square(...);
assert!($square(9) == 81);
```

For a generic function, put the type arguments first:

```whim
function identity<T>(T $value): T {
  return $value;
}

$read = identity::<string>(...);
assert!($read('value') == 'value');
```

## Bound methods

A first-class instance method keeps its receiver:

```whim
final class Greeter {
  public function __construct(private string $name) {}

  public function greet(string $prefix): string {
    return $prefix . $this->name;
  }
}

$greeter = new Greeter('Ada');
$greet = $greeter->greet(...);
assert!($greet('Hello, ') == 'Hello, Ada');
```

Static methods use `ClassName::method(...)`. Whim checks visibility when it
creates the callable.

## Partial calls

`?` leaves one argument open:

```whim
function join(string $left, string $middle, string $right): string {
  return $left . $middle . $right;
}

$wrap = join('(', ?, ')');
assert!($wrap('value') == '(value)');
```

Whim evaluates bound argument expressions when it creates the partial. Later
calls reuse those values.

Several holes become parameters in the order in which the partial expression
lists them:

```whim
function format(int $id, string $label, bool $loud): string {
  return $label . ':' . $id;
}

$render = format(label: ?, loud: true, id: ?);
assert!($render('item', 7) == 'item:7');
```

The first new parameter fills `label`; the second fills `id`.

## Leaving later parameters open

A trailing `...` leaves other unbound parameters open:

```whim
function shape(string $kind, int $size = 1, string $mode = 'flat'): string {
  return $kind . ':' . $size . ':' . $mode;
}

$deep = shape(?, mode: 'deep', ...);
assert!($deep('cube', 4) == 'cube:4:deep');
```

Without the trailing `...`, parameters without holes keep their defaults or no
longer belong to the partial callable.

A partial callable may itself be partially called. Whim preserves the bound
values and the order of the remaining holes.
