# Circuit Breakers

`Whim\Fuse\Fuse` counts failures and blocks requests for a recovery period
once they reach a threshold. The caller checks `allow()`, runs the operation,
and reports its outcome with `recordSuccess()` or `recordFailure()`.

| State             | Behaviour                                                                                              |
| ----------------- | ------------------------------------------------------------------------------------------------------ |
| `State::Closed`   | Allows requests. A success clears the consecutive failure count; the failure threshold opens the fuse. |
| `State::Open`     | Blocks requests until the recovery period ends.                                                        |
| `State::HalfOpen` | Allows requests again. The success threshold closes the fuse; any failure reopens it.                  |

Defaults are five failures, thirty seconds of recovery, and one success to
close. Failure and success thresholds must be positive; recovery may be zero
but cannot be negative.

```whim
use Whim\Clock\FrozenClock;
use Whim\Fuse\Fuse;
use Whim\Fuse\State;
use Whim\Time\Duration;

$clock = FrozenClock::atUnixTimestamp(0);
$fuse = new Fuse(
  failureThreshold: 2u,
  recovery: Duration::fromSeconds(30),
  successThreshold: 2u,
  clock: $clock,
);

assert!($fuse->allow());
$fuse->recordFailure();
$fuse->recordFailure();
assert!(!$fuse->allow());
assert!(($fuse->retryAfter() as Duration)->equals(Duration::fromSeconds(30)));

$clock->advance(Duration::fromSeconds(30));
assert!($fuse->getState() == State::HalfOpen);
$fuse->recordSuccess();
$fuse->recordSuccess();
assert!($fuse->getState() == State::Closed);
```

`getState()` and `allow()` check whether recovery has ended. `retryAfter()`
returns the remaining delay while open, or `null` when requests may proceed.
`reset()` closes the fuse and clears all counts.

A fuse uses `Whim\Clock\SystemClock` by default. Pass any
`Whim\Clock\Clock` to control time. Moving the clock backward extends the
remaining recovery period.

Share one fuse among callers that should share a failure count. Its state
lives in memory. It does not run operations, impose timeouts, or limit the
number of requests in `HalfOpen`. The caller controls those choices and must
report each outcome.

See [Clocks and Sleepers](clock.md), [Retry Policies](retry.md), and
[Rate Limits](throttle.md).
