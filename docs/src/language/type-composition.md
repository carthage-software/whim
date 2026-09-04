# Unions, Intersections, and Ranges

Whim builds larger types from smaller ones. These operators describe sets of
values. They do not convert values.

## Union types

`A|B` accepts a value that fits `A` or `B`.

```whim
type Number = int|float;

function square(Number $value): Number {
  return $value * $value;
}
```

A union may contain any valid member except `void` or `never`. `never` adds no
value, so Whim rejects it as a redundant union member. `mixed` already contains
every value, so Whim rejects other members beside it.

Direct duplicate or covered members are also errors:

```text
int|int
bool|true
```

Aliases can hide that two written members are equal. Whim still gives the union
the right runtime meaning.

## Intersection types

`A&B` accepts a value that fits both `A` and `B`.

```whim
interface Named {}
interface Stored {}

function save(Named&Stored $value): void {}
```

An intersection can combine class and interface contracts or refine any type:

```whim
type NonEmptyString = string&!'';
type SmallPositiveInt = int&1..=100;
```

`mixed` adds no rule to an intersection, so Whim rejects it there. A direct
duplicate member is also an error.

An impossible intersection is a valid empty type. For example,
`int&SomeInterface` has no value unless the two parts can overlap.

## Negated types

`!T` accepts every value outside `T`.

```whim
function require_value(!null $value): !null {
  return $value;
}
```

Negation works inside collections, callable types, bounds, catch clauses, and
other composed types.

```whim
$values = vec[1, 'text'];
assert!($values is vec<!bool>);
```

Useful identities include:

- `!never` accepts every value;
- `!mixed` accepts no value;
- `!!T` has the same values as `T`.

Whim does not allow negation of `void` or the wildcard `_`.

## Precedence

Prefix `!` and `=` bind first. `&` binds before `|`.

```text
A&B|C       means (A&B)|C
A|B&C       means A|(B&C)
!(A|B)      excludes the whole union
```

Use parentheses when they make the type easier to read.

## Literal types

An integer, float, string, boolean, `null`, enum case, or constant can describe
one value.

```whim
type Answer = 'yes'|'no';
type Ordering = -1|0|1;

function enabled(true $value): void {}
```

Integer and float literal types remain distinct. `1` does not contain `1.0`.

## Integer range types

Range types accept integer intervals.

| Type | Accepted values |
| --- | --- |
| `1..10` | 1 through 9 |
| `1..=10` | 1 through 10 |
| `0..` | zero and all larger integers |
| `..0` | all integers below zero |
| `..=0` | zero and all smaller integers |

The lower bound is inclusive. `..` excludes the upper bound, while `..=`
includes it.

```whim
type Port = 1..=65535;
type Offset = 0..;

function connect(Port $port): void {}
```

Both bounds may be negative. A reversed or equal exclusive range is empty.
`1..=1` contains only `1`.

Ranges work as generic bounds and type arguments:

```whim
function clamp_input<T: 0..=100>(T $value): T {
  return $value;
}

assert!(clamp_input::<25>(25) == 25);
```

## String length types

Square brackets after `string` limit its byte length:

| Type | Accepted lengths |
| --- | --- |
| `string[5]` | exactly five bytes |
| `string[1..]` | one byte or more |
| `string[..64]` | fewer than 64 bytes |
| `string[1..=64]` | one through 64 bytes |

These types follow integer range rules. Empty ranges accept no value.

```whim
type Name = string[1..=64];

function greet(Name $name): string {
  return 'Hello, ' . $name;
}
```

Whim treats `string[0]` as `''`, `string[1..]` as `string&!''`, and
`string[0..]` as `string`.

## `never`

`never` contains no values. A function that returns `never` must throw, exit,
or keep running.

```whim,norun
function fail(string $message): never {
  throw new Whim\Unwind\RuntimeException($message);
}
```

`never` may appear in parameters and generic types. Since no caller can supply
a value of that type, it helps express branches that cannot run. A method that
returns `never` can satisfy a contract with any return type.

## `void`

`void` is only a return type. A void function returns no value.

`void` cannot appear in a parameter, property, union, negation, type argument,
or type alias.
