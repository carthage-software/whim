# Retry Policies

`Whim\Retry` runs a callable again after a failure. A `Policy` sets the maximum
attempt count, delays, jitter, error condition, and optional deadline.

```whim
use Whim\Reference\Strong;
use Whim\Retry;
use Whim\Retry\Policy;
use Whim\Unwind\RuntimeException;

$calls = new Strong::<int>(0);
$result = Retry\retry::<string>(Policy::attempts(3u), fn(): string {
  $calls->value++;
  if ($calls->value < 3) {
    throw new RuntimeException('try again');
  }

  return 'done';
});

assert!($result == 'done');
assert!($calls->value == 3);
```

The count includes the first attempt. `Policy::attempts(1u)` runs once.
By default, a policy has no delay and retries only `Whim\Unwind\Exception`
and its subclasses. It does not retry an `Error`. When retries stop, the
retrier throws the last throwable unchanged.

## Delays and conditions

Policy methods return copies:

| Method                                                           | Effect                                                                                      |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `withDelay(Delay)`                                               | Uses `NoDelay`, `FixedDelay`, or `ExponentialDelay`.                                        |
| `withFixedDelay(Duration)`                                       | Uses the same delay after each failed attempt.                                              |
| `withExponentialDelay(initial, multiplier = 2u, maximum = null)` | Multiplies the delay after each failed attempt, with an optional cap.                       |
| `withJitter(Jitter)`                                             | Uses the exact delay (`None`) or a random delay from zero to that delay (`Full`).           |
| `withCondition(fn(Throwable): bool)`                             | Replaces the rule that decides which errors to retry.                                       |
| `withDeadline(Duration)`                                         | Stops after a failure if elapsed time exceeds the deadline or the next delay would pass it. |

Attempt counts must be positive. Delays and deadlines may be zero but cannot
be negative. An exponential multiplier must be at least two. An uncapped
delay that grows beyond `Duration`'s range throws `OverflowError`.

The deadline does not interrupt an operation already running. Use an operation's
own timeout or cancellation support to bound that call. A cancellation token
passed to `retry` or `Retrier::run` cancels retry pauses; the operation must
handle its own cancellation.

## Controlled time

`Retrier` accepts a `Whim\Clock\Sleeper`, a `Whim\Clock\Clock`, and a
`Whim\RandomSequence\Sequence`. It uses `SystemSleeper`, `SystemClock`, and
a securely seeded sequence by default.

A frozen clock and virtual sleeper let tests check delays without waiting:

```whim
use Whim\Clock\FrozenClock;
use Whim\Clock\VirtualSleeper;
use Whim\RandomSequence\MersenneTwisterSequence;
use Whim\Reference\Strong;
use Whim\Retry\Jitter;
use Whim\Retry\Policy;
use Whim\Retry\Retrier;
use Whim\Time\Duration;
use Whim\Unwind\RuntimeException;

$clock = FrozenClock::atUnixTimestamp(0);
$retrier = new Retrier(
  new VirtualSleeper($clock),
  $clock,
  new MersenneTwisterSequence(7u),
);
$policy = Policy::attempts(4u)
  ->withExponentialDelay(Duration::second())
  ->withJitter(Jitter::None)
  ->withDeadline(Duration::fromSeconds(10));

$calls = new Strong::<int>(0);
$result = $retrier->run::<int>($policy, fn(): int {
  $calls->value++;
  if ($calls->value < 4) {
    throw new RuntimeException('try again');
  }

  return 42;
});

assert!($result == 42);
assert!($clock->now()->getSeconds() == 7);
```

See [Clocks and Sleepers](clock.md), [Circuit Breakers](fuse.md), and
[Rate Limits](throttle.md).
