# Closures

A callable is a closure, short closure, first-class function, bound method, or
partial call.

## Short closures

A short closure uses `fn`. This is the preferred closure syntax:

```whim
$double = fn(int $value): int {
  return $value * 2;
};

assert!($double(21) == 42);
```

Like a named function, a short closure may have type parameters, typed
parameters, defaults, a return type, attributes, and a block body.

A short closure may also have an expression body:

```whim
$factor = 3;
$multiply = fn(int $value): int => $value * $factor;

assert!($multiply(4) == 12);
```

A block body may contain any statements. It does not return its last
expression. Use `return` to return a value. A block body may declare `void`;
an expression body may not.

A short closure captures each outer variable that its body uses. Capture is by
value at creation time. Parameters are not captures.

## Explicit captures

A long closure uses `function` and lists outer variables in `use`:

```whim
$offset = 10;
$add = function(int $value) use ($offset): int {
  return $value + $offset;
};

assert!($add(5) == 15);
```

Capture copies the current value. A later assignment to the outer variable does
not change that copy. Mutating a captured local also does not change the outer
local.

Objects keep identity when copied, so a captured object still sees later
property changes.

Use a long closure when the explicit capture list helps the reader. Prefer
`fn` otherwise.

## `$this`

A closure or short closure made in an instance method may use `$this` without
listing it in `use`:

```whim
final class Counter {
  public function __construct(private int $value) {}

  public function reader(): fn(): int {
    return fn(): int => $this->value;
  }
}

$counter = new Counter(7);
assert!($counter->reader()() == 7);
```

The callable keeps the receiver alive.

## Callable types

`fn(int, string): bool` describes a callable by its input and output types.
Whim checks callable compatibility when a value crosses a typed boundary, then
checks each call as it runs.

Parameter types are contravariant and the return type is covariant. A callable
that accepts `mixed` may replace one that accepts `int`. A callable that
returns `int` may replace one that returns `int|string`.

Callable values compare by identity. Two closures with the same source are
still different values.
