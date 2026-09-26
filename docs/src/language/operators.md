# Operators and Arithmetic

Whim checks operator types at runtime. It does not turn strings or booleans
into numbers.

## Integer arithmetic

`+`, `-`, and `*` return int when both operands are int. Whim checks overflow
and underflow:

```whim
assert!(2 + 3 == 5);
assert!(2 - 3 == -1);
assert!(2 * 3 == 6);
```

An out-of-range result throws `OverflowError` or `UnderflowError`. Unary `-`,
`++`, and `--` use the same checks.

Two `uint` operands produce a checked `uint`. Mixed `int` and `uint` operands
throw `IncompatibleOperandsError` for `+`, `-`, `*`, `%`, and `**`, even when
both values are small. Cast explicitly to choose the integer kind:

```whim
assert!(2u + 3u == 5u);
assert!((2 as uint) + 3u == 5u);
assert!(3u - 2u == 1u);
```

Unsigned subtraction below zero throws `UnderflowError`; addition and
multiplication above `Whim\Math\UINT_MAX` throw `OverflowError`. Negating an
unsigned variable returns `0u` for zero and throws `UnderflowError` otherwise.
The compiler rejects a negated nonzero unsigned literal.

If either operand of `+`, `-`, or `*` is float, the result is float:

```whim
assert!(1 + 0.5 == 1.5);
assert!(2.0 * 3 == 6.0);
```

## Division and remainder

`/` accepts any pair of `int`, `uint`, and `float`, and always returns float:

```whim
assert!(10 / 2 == 5.0);
assert!(7 / 2 == 3.5);
```

`%` accepts two ints or two uints and keeps that kind. A signed result has the
sign of the left operand:

```whim
assert!(7 % 2 == 1);
assert!(-7 % 2 == -1);
```

Division or remainder by zero throws `DivisionByZeroError`, including float
division by zero. Operand kinds are checked first: `1u % 0` is an incompatible
pair, while `1u % 0u` divides by zero. For an exact unsigned quotient, use
`Whim\UInt\div(7u, 2u)`, which returns `3u`.

## Powers

`**` is right-associative. An int base and a nonnegative int exponent produce
an int when the result fits. A negative exponent or any float operand produces
a float:

```whim
assert!(2 ** 10 == 1024);
assert!(2 ** -1 == 0.5);
assert!(2.0 ** 2 == 4.0);
```

Integer overflow throws. `0 ** -1` throws `DivisionByZeroError`.

`uint ** uint` returns a checked unsigned result. `0u ** 0u` is `1u`;
`1u` raised to any unsigned power is `1u`. Any float operand selects floating
arithmetic, so `2u ** -1.0` is `0.5`.

## Bit operators

`&`, `|`, and `^` accept two ints or two uints. `~` accepts either integer
kind. They preserve that kind; `~0u` is `Whim\Math\UINT_MAX`.

The left operand of `<<` and `>>` may be int or uint. The count may independently
be int or uint, from 0 through 63; other counts throw `ArithmeticError`.
The result keeps the left operand's kind. Left shifts discard high bits.
Signed right shifts extend the sign; unsigned right shifts insert zeros.

## Increment and decrement

Prefix `++$value` changes the target and returns the new value. Postfix
`$value++` returns the old value. `--` follows the same rule. These operators
accept int, uint, and float targets. Unsigned steps preserve `uint`: `$value++`
adds an unsigned one, while `$value += 1` rejects an unsigned target.
If arithmetic or a declared type check fails, the target keeps its old value.

## Boolean operators

`!`, `&&`, and `||` accept bool. `&&` skips its right side when the left side is
false. `||` skips it when the left side is true.

## Concatenation

`.` joins strings, ints, uints, and floats as text. It rejects other values.
Unsigned values print all their decimal digits without a suffix. Interpolation
and output use the same rule.

## Comparison and type operators

[Equality and Order](../semantics/equality-and-comparison.md) covers `==`,
`!=`, `<`, `<=`, `>`, `>=`, and `<=>`.

`is`, `as`, and `?as` check a [runtime type](../semantics/type-system.md).
These operators and comparisons do not chain.

## Binding order

The full precedence table appears in the [operator appendix](../appendices/operators.md).
When the order is not plain, use parentheses. They cost nothing and state the
intended order.
