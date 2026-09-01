# Option and Result

Use `null|T` for simple absence when `null` cannot be a valid `T`.

```whim
function find_name(int $id): null|string {
  return match ($id) {
    1 => 'Ada',
    $_ => null,
  };
}
```

Use `Option<T>` when a present value may itself be `null`. Use `Result<T, E>`
when failure is data that the caller should inspect.

Keep nullable parameters nullable. Requiring callers to allocate an option to
pass no argument adds work and makes calls harder to read.

## Option values

`Whim\Option\Some<T>` contains a value. `Whim\Option\None` contains no value.

```whim
use Whim\Option\None;
use Whim\Option\Some;

$present = new Some::<null>(null);
$absent = new None();

assert!($present->isSome());
assert!($absent->isNone());
```

`Some<null>` differs from `None`, which is the reason to use an option here.

`Whim\Option\some($value)` and `Whim\Option\none()` are short constructors.

## Reading an option

`isSome()` and `isNone()` test the branch.

`unwrap()` returns the value from `Some`. It throws `LogicException` on `None`.
Use it only after `isSome()` or another rule proves the branch.

`unwrapOr($default)` returns the value or the given default.
`unwrapOrElse($fallback)` calls the fallback only for `None`.

```whim
use Whim\Option;

$value = Option\none()->unwrapOr::<int>(42);
assert!($value == 42);
```

## Changing an option

| Method | `Some<T>` | `None` |
| --- | --- | --- |
| `map($f)` | `Some($f($value))` | unchanged `None` |
| `mapOr($default, $f)` | `$f($value)` | `$default` |
| `andThen($f)` | `$f($value)` | unchanged `None` |
| `orElse($f)` | unchanged `Some` | `$f()` |
| `filter($test)` | same `Some` or `None` | unchanged `None` |
| `inspect($f)` | calls `$f`, then returns itself | returns itself |
| `okOr($error)` | `Ok($value)` | `Err($error)` |

Callbacks for a branch that does not run are not called.

## Result values

`Whim\Result\Ok<T>` contains a success value. `Whim\Result\Err<E>` contains an
error value.

```whim
use Whim\Result\Err;
use Whim\Result\Ok;
use Whim\Result\Result;

function parse_switch(string $value): Result<bool, string> {
  return match ($value) {
    'on' => new Ok::<bool>(true),
    'off' => new Ok::<bool>(false),
    $_ => new Err::<string>('expected on or off'),
  };
}
```

The error type need not implement `Throwable`. It may be a string, enum, object,
or any other type.

## Reading a result

`isOk()` and `isErr()` test the branch.

`unwrap()` returns the success value and throws `LogicException` on `Err`.
`unwrapErr()` returns the error and throws on `Ok`.

`unwrapOr($default)` returns the success value or the default.
`unwrapOrElse($fallback)` calls the fallback with the error only for `Err`.

`ok()` returns `Some($value)` or `None`. `err()` returns `Some($error)` or
`None`.

## Changing a result

| Method | `Ok<T>` | `Err<E>` |
| --- | --- | --- |
| `map($f)` | `Ok($f($value))` | keeps the error |
| `mapErr($f)` | keeps the value | `Err($f($error))` |
| `mapOr($default, $f)` | `$f($value)` | `$default` |
| `andThen($f)` | `$f($value)` | keeps the error |
| `orElse($f)` | keeps the value | `$f($error)` |
| `inspect($f)` | calls `$f`, then returns itself | returns itself |
| `inspectErr($f)` | returns itself | calls `$f`, then returns itself |

The generic return types keep all branches. For example, `andThen` returns
`Result<U, E|F>` when the old error is `E` and the callback may return `F`.

## Capturing a throwable

`Whim\Result\attempt::<T, E>($callback)` runs a callback and returns
`Ok<T>` on success. It catches only `E` and returns `Err<E>`. Any other
throwable continues outward.

```whim
use Whim\Result;
use Whim\Unwind\RuntimeException;

$result = Result\attempt::<int, RuntimeException>(
  fn(): int => throw new RuntimeException('failed'),
);

assert!($result->isErr());
```

`E` must implement `Throwable`.

## Must-use results

Option and result methods carry `#[MustUse]`. If code discards their result by
mistake, Whim raises `DiscardedResultError`. Use the returned value, pass it on,
or write `discard!(...)` when ignoring it is deliberate.
