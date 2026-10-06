# Clocks and Sleepers

`Whim\Clock` lets code read time and sleep through interfaces. Use the system
implementations for real time, or a controlled clock and sleeper in tests.
The types are part of the standard library; they need no package or autoloader.

## Reading time

`Clock::now()` returns a `Whim\Time\SystemTime`.
`Clock::nowIn($timezone)` reads the clock once and returns a
`Whim\DateTime\ZonedDateTime` in that timezone.

- `SystemClock` reads `SystemTime::now()` on each call.
- `FrozenClock` holds a time until `advance($duration)` or `travelTo($time)`
  changes it. A negative duration moves it backward.
- `OffsetClock` adds a fixed duration to another clock's current reading.
  It follows changes to the inner clock.

Pass a `Clock` to code that needs the current time:

```whim
use Whim\Clock\Clock;
use Whim\Clock\FrozenClock;
use Whim\Time\Duration;
use Whim\Time\SystemTime;

final readonly class Expiry {
  public function __construct(private Clock $clock) {}

  public function after(Duration $duration): SystemTime {
    return $this->clock->now()->plus($duration);
  }
}

$clock = FrozenClock::atUnixTimestamp(1_000);
$expiry = new Expiry($clock);
assert!($expiry->after(Duration::fromMinutes(30))->getSeconds() == 2_800);
$clock->advance(Duration::fromMinutes(1));
assert!($expiry->after(Duration::fromMinutes(30))->getSeconds() == 2_860);
```

`new FrozenClock($time)` accepts an existing `SystemTime`.
`FrozenClock::atUnixTimestamp($seconds, $nanoseconds = 0)` accepts a Unix
timestamp; nanoseconds must be in `0..999_999_999`.
`FrozenClock::startingNow()` freezes the time when the call runs.
Copies of the clock share its state. A saved reading stays unchanged when the
clock moves.

```whim
use Whim\Clock\FrozenClock;
use Whim\Clock\OffsetClock;
use Whim\DateTime\TimeZone;
use Whim\Time\Duration;

$base = FrozenClock::atUnixTimestamp(0);
$ahead = new OffsetClock($base, Duration::fromHours(1));
assert!($ahead->now()->getSeconds() == 3_600);
assert!(1 == $ahead->nowIn(TimeZone::utc())->dateTime->time->hour);
$base->advance(Duration::second());
assert!($ahead->now()->getSeconds() == 3_601);
```

`SystemClock` also implements `Whim\Default\Default`.
`SystemClock::default()` returns a new system clock.

## Sleeping

`Sleeper::sleep($duration, $cancellation = null)` pauses the current task.
Both implementations accept a `Whim\Async\CancellationToken` and throw
`Whim\Async\CancelledException` when cancellation is requested.

- `SystemSleeper` calls `Whim\Async\sleep()`. Other tasks can run while it
  waits. It also implements `Whim\Default\Default`;
  `SystemSleeper::default()` returns a new system sleeper.
- `VirtualSleeper` advances a linked `FrozenClock` for a positive duration,
  then yields once with `Whim\Async\later()`. It adds no real delay.

Zero and negative durations add no delay and do not move the frozen clock.
Every virtual sleep still yields once, including those durations.

```whim
use Whim\Clock\FrozenClock;
use Whim\Clock\VirtualSleeper;
use Whim\Time\Duration;

$clock = FrozenClock::atUnixTimestamp(0);
$sleeper = new VirtualSleeper($clock);
$sleeper->sleep(Duration::fromDays(1));
assert!($clock->now()->getSeconds() == 86_400);
$sleeper->sleep(Duration::fromSeconds(-1));
assert!($clock->now()->getSeconds() == 86_400);
```

`VirtualSleeper` checks cancellation before advancing and after yielding.
Cancellation before the call changes no time. Cancellation during the yield
leaves any time already added in place.

Concurrent virtual sleeps each add their duration to the same clock. Two sleeps
of two and three seconds advance it by five seconds. This helper does not
simulate timer deadlines or advance the scheduler's clock.

See [Time and Calendars](time.md) for time values and timezone operations,
and [Tasks and Futures](async.md) for scheduling and cancellation.
