# Equality and Order

Whim does not convert either side of an equality check.

## Equality

`==` tests equality. `!=` tests its opposite:

```whim
assert!(1 == 1);
assert!(1 != 1.0);
assert!('1' != 1);
assert!(0 != false);
assert!('' != null);
```

Two vecs or tuples are equal when their kinds, sizes, and ordered values match.
Two dicts are equal when they hold the same strict keys and equal values.
Dict insertion order does not affect equality:

```whim
assert!(vec[1, 2] != vec[2, 1]);
assert!((1, 2) != (2, 1));
assert!(dict['a' => 1, 'b' => 2] == dict['b' => 2, 'a' => 1]);
```

Objects and callables compare by identity:

```whim
final class Token {}

$token = new Token();
$same = $token;

assert!($token == $same);
assert!($token != new Token());

$callable = fn(): int => 1;
assert!($callable == $callable);
assert!($callable != fn(): int => 1);
```

Enum cases are single values. A case equals itself and not another case.

## Float details

Positive and negative floating zero are equal. NaN is not equal to any value,
including itself.

## Order operators

`<`, `<=`, `>`, and `>=` compare:

- int with int or float;
- float with int or float;
- string with string, in byte order.

Other pairs throw `IncompatibleOperandsError`. Arrays, objects, callables,
bool, and null have no built-in order.

```whim
assert!(1 < 2);
assert!(1 < 2.0);
assert!('apple' < 'banana');
assert!('10' < '9');
```

A comparison with NaN is false.

## Three-way comparison

`<=>` returns `-1`, `0`, or `1` under the same order rules:

```whim
assert!((1 <=> 2) == -1);
assert!((2 <=> 2) == 0);
assert!(('b' <=> 'a') == 1);
```

It throws when either number is NaN.

Comparison operators do not chain. Write `1 < $value && $value < 10`. The
parser rejects `1 < $value < 10`.
