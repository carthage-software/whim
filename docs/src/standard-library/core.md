# Core Types and Functions

This page covers small contracts used throughout the standard library.

## Refine

`Whim\Refine` names common types:

- `ArrayKey` is `string|int|bool`.
- `Numeric` is `int|float`.
- `Scalar` is `int|float|string|bool`.
- `Nullable<T>` is `T|null`.
- `NonNull`, `NonEmptyString`, `NonEmptyVec<T>`, and `NonEmptyDict<K, V>`
  exclude empty values.
- `PositiveInt`, `NonNegativeInt`, `NegativeInt`, and `NonPositiveInt` name
  integer ranges.
- `Uint8`, `Int8`, `Uint16`, `Int16`, `Uint32`, `Int32`, and `Uint64` name
  fixed bounds.
- `Percent` is `0..=100`; `Digit` is `0..=9`.
- `Iterable<K, V>` accepts an iterator, a value that creates an iterator, or an
  array.
- `AnyTuple` and `AnyTupleOf<T>` accept tuples of any supported length.
- `Exclude<T, N>` removes `N` from `T`; `Extract<T, U>` keeps their overlap.

It also names common callables:

- `Predicate<T>`: `fn(T): bool`
- `Transform<T, U>`: `fn(T): U`
- `Reducer<T, U>`: `fn(U, T): U`
- `Consumer<T>`: `fn(T): void`
- `Supplier<T>`: `fn(): T`
- `Comparator<T>`: `fn(T, T): Comparison\Ordering`

`Falsy` and `Truthy` describe PHP's old truth rules for data conversion. Whim
conditions still require `bool`; Whim does not apply those aliases to `if` or
loops.

## Option and Result

`Option<T>` is the sealed family `Some<T>|None`. `Some` stores its public
readonly `value`. `None` stores nothing.

The main operations are `isSome`, `isNone`, `unwrap`, `unwrapOr`,
`unwrapOrElse`, `map`, `mapOr`, `andThen`, `orElse`, `filter`, `okOr`, and
`inspect`. `unwrap()` on `None` throws `LogicException`.

`Result<T, E>` is the sealed family `Ok<T>|Err<E>`. Their public readonly fields
are `value` and `error`. Result adds `unwrapErr`, `mapErr`, `ok`, `err`, and
`inspectErr` to the same map and chain style. Unwrapping the wrong side throws
`LogicException`.

Use `Option\some($value)` and `Option\none()` as short constructors.
`Result\attempt::<T, E>($callback)` returns `Ok` or catches `E` into `Err`.

See [Option and Result](../core-library/errors.md) for examples and the null
rule.

## Comparison

`Equal<T>` defines `equals(T): bool`. `Order<T>` extends it with
`compare(T): Ordering` and default methods for less, greater, min, max, and
clamp.

`Ordering` has `Less`, `Equal`, and `Greater`. It can test each relation,
reverse an order, or chain a second order through `then` and `thenWith`.

Collection sort functions take `Comparator<T>`, which returns `Ordering`, not
an integer.

## Conversion and defaults

`Convert\ToString` defines the explicit `toString(): string` contract. Whim has
no magic `__toString` method.

`Default\Default` defines `public static function default(): static`. Types use
it when one value is the clear default configuration or empty state.

## Runtime type IDs

`Whim\Type` is part of the runtime core. Programs can use it without a
standard-library artifact.

`Type\of($value)` returns the engine-local `TypeId` of a value.
`Type\id::<T>()` returns the ID of a reified type. Equal types have equal IDs in
one engine run.

`Type\to_debug_string($id)` returns a type spelling for logs and diagnostics.
The spelling is not an identity or a source format. Two different IDs may have
the same debug string, such as when one name denotes a type parameter in one
place and a declared type in another. Never use the string as a map key or for
type comparison; use `TypeId` itself. The spelling may change between Whim
releases.

Type IDs are for maps and dispatch inside one process. They may change between
runs. Do not store them in files or send them over a network.

## Symbols and enums

`Symbol\exists($name, $autoload)` checks any named symbol. `get_kind($name)`
returns its `SymbolKind` or throws when absent.

Every enum implements `Enum\UnitEnum`; a backed enum also implements
`BackedEnum<int|string>`. Unit enums expose `name`; backed cases also expose
`value`.

## Garbage collection

Reference counts release ordinary values. `GC\collect_cycles()` asks the cycle
collector to find unreachable strong cycles and returns the number it
collected. Normal programs seldom need to call it.
