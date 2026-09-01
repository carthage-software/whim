# Built-in Values

Every Whim value has a runtime type. A declaration can state which values it
accepts. Whim checks that rule when the program reaches the boundary.

## Scalar values

Whim has five scalar value kinds.

| Type | Values |
| --- | --- |
| `null` | `null` |
| `bool` | `true` and `false` |
| `int` | signed 64-bit integers |
| `float` | double-precision floating-point numbers |
| `string` | byte strings |

Integers and floats stay distinct:

```whim
assert!(1 is int);
assert!(1.0 is float);
assert!(1 != 1.0);
```

Strings hold bytes, not a promise of valid UTF-8. The source file is UTF-8, but
a string may contain any byte through escapes, file reads, sockets, or binary
decoding.

## Arrays

Whim has three array forms:

| Type | Meaning |
| --- | --- |
| `(A, B)` | an immutable, fixed-size tuple |
| `vec<T>` | a mutable list with integer keys from zero |
| `dict<K, V>` | a mutable, ordered key-value map |

`array<K, V>` accepts a tuple, vec, or dict whose keys and values fit `K` and
`V`.

```whim
$pair = ('Ada', 36);
$names = vec['Ada', 'Grace'];
$scores = dict['Ada' => 10, 'Grace' => 12];

assert!($pair is (string, int));
assert!($names is vec<string>);
assert!($scores is dict<string, int>);
```

The [collections chapter](collections.md) covers their syntax and update rules.

## Objects and callables

`object` accepts any class instance. A class or interface name accepts objects
of that class or its subtypes.

`fn(A, B): R` accepts a callable with two parameters and result `R`:

```whim
function apply(fn(int): int $operation, int $value): int {
  return $operation($value);
}

$double = fn(int $value): int => $value * 2;
assert!(apply($double, 21) == 42);
```

`classname<T>` accepts a class name whose instances satisfy `T`.

## Wide and empty types

`mixed` accepts every value. `!never` also means every value.

`never` accepts no value. A function with return type `never` cannot return.
It must throw, exit, or keep running.

`void` is valid only as a function or method return. Such a callable returns no
value. You cannot use `void` for a parameter or property.

## Literal types

A scalar value may also act as a type:

```whim
function choose('yes'|'no' $answer): bool {
  return $answer == 'yes';
}

assert!(choose('yes'));
```

`true`, `42`, and `'yes'` each describe one value. Literal types make unions,
ranges, enum cases, and constants precise.

## Conditions

Conditions must be `bool`. Whim has no truthy or falsy conversion:

```text
$name = 'Ada';
if ($name) {
  write_line!($name);
}
```

Write the test you mean:

```whim
$name = 'Ada';
if ($name != '') {
  write_line!($name);
}
```

This rule also applies to `while`, `do ... while`, `&&`, `||`, `&&=`, and
`||=`.
