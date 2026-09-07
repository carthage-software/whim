# Calls and Evaluation Order

Whim evaluates expressions from left to right unless an operator states that
it short-circuits.

## Calls

Whim evaluates the callable first, then each argument from left to right. For a
method call, it evaluates the receiver before the arguments. A constructor uses
the same argument order.

```whim
final class Log {
  public static vec<string> $events = vec[];
}

function mark(string $name, int $value): int {
  Log::$events[] = $name;
  return $value;
}

function sum(int $a, int $b, int $c): int {
  return $a + $b + $c;
}

assert!(sum(mark('a', 1), mark('b', 2), mark('c', 3)) == 6);
assert!(Log::$events == vec['a', 'b', 'c']);
```

## Operators

Whim evaluates the left operand before the right. An indexed write evaluates
the index before the stored value. A dict entry evaluates its key before its
value.

Array literal elements, tuple entries, and object constructor arguments also
run from left to right.

## Short-circuit forms

These forms may skip work:

- `false && $right` skips `$right`;
- `true || $right` skips `$right`;
- a present, initialized, non-null left side of `??` skips the right side;
- a null receiver for `?->` skips the member access and call arguments;
- `match` evaluates only the chosen arm result;
- a destructuring default runs only for a missing position.

`0`, `false`, and `''` are not null, so `??` keeps them.

Property and index paths on the left of `??` stop at the first missing,
uninitialized, or null step and select the fallback. Later keys are skipped.
Receivers and evaluated keys run once. Invalid types, visibility violations,
undefined locals, and exceptions from calls still throw. See
[coalescing](../language/expressions.md#coalescing) for examples and the
boundary between probing an access path and evaluating a computation.

## Pipeline

`$value |> $callable` evaluates the value, then the callable, once each. It
then calls the callable with the value.

## Type checks

Whim checks call arguments after it has evaluated them. It checks a return
value when the callable returns. A failed check throws `TypeError` at that
boundary.

Default parameter values run when the call omits that argument. Named and
partial calls still preserve source evaluation order for the expressions that
the caller supplies.
