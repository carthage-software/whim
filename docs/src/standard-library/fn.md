# Function Helpers

`Whim\Fn` provides five helpers that return callables. They use the
`Transform<I, O>` and `Consumer<T>` aliases from `Whim\Refine` and need no
package install.

| Helper | Returned callable |
| --- | --- |
| `identity<T>()` | Returns its input unchanged. |
| `tap<T>(callback)` | Calls the callback, then returns its input. |
| `after<I, O, R>(first, next)` | Passes the result of `first` to `next`. |
| `when<I, O>(condition, then, otherwise)` | Tests the input and calls the chosen branch. |
| `rethrow()` | Throws the throwable passed to it. |

The helpers do not call their callbacks until you call the returned function.
Callbacks may suspend or throw. `after` waits for the first callback to return
before it calls the next; `when` calls only the chosen branch. Errors pass
through unchanged.

## Identity and observation

`identity` returns the same value. `tap` adds a callback without changing the
value passed to the next step:

```whim
use Whim\Fn;
use Whim\Reference\Strong;

$identity = Fn\identity::<int>();
assert!($identity(7) == 7);

$seen = new Strong::<int>(0);
$observe = Fn\tap::<int>(fn(int $value): void {
  $seen->value = $value;
});

assert!($observe(3) == 3);
assert!($seen->value == 3);
```

`tap` returns the input after the callback runs. A callback can still change an
object it receives, as with any other function call.

## Composition and branches

`after` runs two transforms in order. The first output type must match the
second input type:

```whim
use Whim\Fn;

$doubleThenText = Fn\after::<int, int, string>(
  fn(int $value): int => $value * 2,
  fn(int $value): string => 'n=' . $value,
);

assert!($doubleThenText(4) == 'n=8');
```

`when` passes the same input to the predicate and the chosen transform. Both
branches must return the declared output type:

```whim
use Whim\Fn;

$choose = Fn\when::<int, string>(
  fn(int $value): bool => $value > 0,
  fn(int $_): string => 'positive',
  fn(int $_): string => 'other',
);

assert!($choose(1) == 'positive');
assert!($choose(0) == 'other');
```

## Rethrowing errors

`rethrow` returns a `fn(Throwable): never`. Use it when an error callback should
throw the error it receives:

```whim
use Whim\Fn;
use Whim\Unwind\RuntimeException;

$rethrow = Fn\rethrow();
$expected = new RuntimeException('failed');
$caught = null;
try {
  $rethrow($expected);
} catch (RuntimeException $error) {
  $caught = $error;
}

assert!($caught == $expected);
```

Each helper has `#[MustUse]`: store, pass, call, or explicitly discard the
returned callable.
