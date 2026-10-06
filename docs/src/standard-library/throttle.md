# Rate Limits

`Whim\Throttle` provides two limiters. Each key has its own state and starts
with a full allowance. Keys must be nonempty strings.

| Limiter       | Behaviour                                                                      |
| ------------- | ------------------------------------------------------------------------------ |
| `TokenBucket` | Adds one permit per rate interval, up to a burst limit.                        |
| `FixedWindow` | Grants a fixed permit count per window, starting at a key's first hit or peek. |

Both implement `Limiter`:

| Method                           | Effect                                                  |
| -------------------------------- | ------------------------------------------------------- |
| `hit(key)`                       | Takes one permit if available and returns a `Decision`. |
| `peek(key)`                      | Returns the current decision without taking a permit.   |
| `allow(key)`                     | Takes a permit if available and returns a boolean.      |
| `wait(key, cancellation = null)` | Sleeps until a permit is available, then takes it.      |
| `forget(key)`                    | Removes the stored state for one key.                   |

`Decision` has two cases: `Allowed` holds the `remaining` permit count;
`Denied` holds a `retryAfter` duration. `isAllowed()` reports which case it is.

## Token buckets

`Rate::of(amount, per)` sets a permit count per duration. `perSecond`,
`perMinute`, and `perHour` are shorter forms. The count and duration must be
positive. `interval()` divides the duration by the count, rounding down with
a minimum of one nanosecond.

The default burst limit equals the rate's permit count. Pass a separate burst
limit to change how many permits a key can store:

```whim
use Whim\Clock\FrozenClock;
use Whim\Throttle\Allowed;
use Whim\Throttle\Denied;
use Whim\Throttle\Rate;
use Whim\Throttle\TokenBucket;
use Whim\Time\Duration;

$clock = FrozenClock::atUnixTimestamp(0);
$limiter = new TokenBucket(Rate::perSecond(1u), burst: 2u, clock: $clock);
assert!(($limiter->peek('peer') as Allowed)->remaining == 2u);
assert!($limiter->allow('peer'));
assert!($limiter->allow('peer'));

$decision = $limiter->hit('peer');
$wait = match ($decision) {
  Allowed => Duration::zero(),
  $denied @ Denied => $denied->retryAfter,
};
assert!($wait->equals(Duration::second()));
assert!($limiter->allow('other'));

$clock->advance(Duration::second());
assert!($limiter->allow('peer'));
assert!(!$limiter->allow('peer'));
```

## Fixed windows and waiting

`FixedWindow` accepts a positive limit and window duration. A new window
restores the full allowance. Requests just before and after a window boundary
can use up to twice the limit.

Both limiters use `Whim\Clock\SystemClock` and `SystemSleeper` by default.
Pass a frozen clock and virtual sleeper to test waits without real delays:

```whim
use Whim\Clock\FrozenClock;
use Whim\Clock\VirtualSleeper;
use Whim\Throttle\FixedWindow;
use Whim\Time\Duration;

$clock = FrozenClock::atUnixTimestamp(0);
$limiter = new FixedWindow(
  1u,
  Duration::fromSeconds(5),
  $clock,
  new VirtualSleeper($clock),
);

assert!($limiter->allow('worker'));
$limiter->wait('worker');
assert!($clock->now()->getSeconds() == 5);
assert!(!$limiter->allow('worker'));
```

The cancellation token cancels a wait's sleep. When a permit is already
available, `wait` takes it without sleeping.

Limiter state lives in memory. Share an instance among callers that should
share limits, and call `forget` when a key no longer needs tracking. Moving
the clock backward resets a fixed window; a token bucket keeps its permit
count and starts counting elapsed time from the new time.

See [Clocks and Sleepers](clock.md), [Retry Policies](retry.md), and
[Circuit Breakers](fuse.md).
