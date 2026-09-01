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

If either operand of `+`, `-`, or `*` is float, the result is float:

```whim
assert!(1 + 0.5 == 1.5);
assert!(2.0 * 3 == 6.0);
```

## Division and remainder

`/` always returns float:

```whim
assert!(10 / 2 == 5.0);
assert!(7 / 2 == 3.5);
```

`%` accepts ints only. Its result has the sign of the left operand:

```whim
assert!(7 % 2 == 1);
assert!(-7 % 2 == -1);
```

Division or remainder by zero throws `DivisionByZeroError`, including float
division by zero.

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

## Bit operators

`&`, `|`, `^`, `~`, `<<`, and `>>` accept ints only. A shift count must be from
0 through 63. Left shift uses the 64-bit bit pattern, so its result may wrap
from positive to negative.

## Increment and decrement

Prefix `++$value` changes the target and returns the new value. Postfix
`$value++` returns the old value. `--` follows the same rule. These operators
accept int and float targets.

## Boolean operators

`!`, `&&`, and `||` accept bool. `&&` skips its right side when the left side is
false. `||` skips it when the left side is true.

## Concatenation

`.` joins strings, ints, and floats as text. It rejects other values.

## Comparison and type operators

[Equality and Order](../semantics/equality-and-comparison.md) covers `==`,
`!=`, `<`, `<=`, `>`, `>=`, and `<=>`.

`is`, `as`, and `?as` check a [runtime type](../semantics/type-system.md).
These operators and comparisons do not chain.

## Binding order

The full precedence table appears in the [operator appendix](../appendices/operators.md).
When the order is not plain, use parentheses. They cost nothing and state the
intended order.
