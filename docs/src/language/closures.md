# Closures

A closure is an anonymous function you can save in a variable or pass as arguments to other functions. You can create the closure in one place and then call the closure elsewhere to evaluate it in a different context. Unlike functions, closures can capture values from the scope in which they’re defined.

## Bodies

A closure uses `fn`:

```whim
$double = fn(int $value): int {
  return $value * 2;
};

assert!($double(21) == 42);
```

Like a named function, a closure may have type parameters, typed
parameters, defaults, a return type, attributes, and a block body.

A closure may also have an expression body:

```whim
$factor = 3;
$multiply = fn(int $value): int => $value * $factor;

assert!($multiply(4) == 12);
```

A block body may contain any statements. It does not return its last
expression. Use `return` to return a value. A block body may declare `void`;
an expression body may not.

## Captures

A closure captures each outer variable that its body uses. Capture is by
value at creation time. Parameters and locals assigned before use are not
captures.

```whim
$offset = 10;
$add = fn(int $value): int {
  return $value + $offset;
};

assert!($add(5) == 15);
```

Capture copies the current value. A later assignment to the outer variable does
not change that copy. Mutating a captured local also does not change the outer
local.

Objects keep identity when copied, so a captured object still sees later
property changes.

## `$this`

A closure made in an instance method may use `$this`:

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
