# Time and Calendars

`Whim\Time` measures exact spans and clock readings. `Whim\DateTime` handles
calendar dates, civil times, timezones, parsing, and formatting.

## Duration

`Time\Duration` stores normalized seconds and nanoseconds. It can represent a
positive, zero, or negative span.

Factory methods create spans from weeks, days, hours, minutes, seconds,
milliseconds, microseconds, or nanoseconds. `zero`, `second`, `millisecond`,
`microsecond`, `nanosecond`, and `max` return common values.

`plus`, `minus`, and `invert` return new durations. Total methods convert to
minutes through nanoseconds. Float totals may lose precision; total nanoseconds
returns an integer.

## Monotonic and wall clocks

`Time\Instant::now()` reads a monotonic clock. Use it for elapsed work because
wall-clock corrections do not move it backward. `elapsed`, `durationSince`,
`plus`, and `minus` work with durations.

`Time\SystemTime` is a wall-clock reading. It supports `now`, `unixEpoch`, Unix
timestamps, comparison, and duration arithmetic. Use it for dates, file times,
protocol times, and stored event times.

Do not turn an `Instant` into a calendar date. Do not measure a timeout with
`SystemTime`.

## Date, Time, and DateTime

`Date` stores a year, month, and day. `Time` stores hour, minute, second, and
nanosecond. `DateTime` combines them without a timezone.

Each type has a checked `fromParts`, a nullable `parse`, a throwing `from`,
formatting, comparison, and a standard `toString` form. `fromPartsUnsafe`
exists for trusted parsed data and does not replace input checks in application
code.

```whim
use Whim\DateTime\Date;

$date = Date::from('2026-08-21');
assert!($date->toString() == '2026-08-21');
assert!($date->isLeapYear() == false);
```

`Month`, `Weekday`, `Era`, and `Meridiem` name calendar values. Refined aliases
bound years, days, hours, minutes, seconds, nanoseconds, and UTC offsets.

## Periods and durations

`DateTime\Period` stores calendar years, months, weeks, and days. Adding one
month follows month length and an `Overflow` rule. Adding a `Time\Duration`
adds exact elapsed time.

These differ around short months and daylight-saving changes. Use a period for
"next month" or "tomorrow." Use a duration for "after 3,600 seconds."

## TimeZone

`TimeZone::from($id)` loads an IANA name, fixed offset, or other accepted zone.
`utc()` and `system()` return common zones. A zone can report its offset and
abbreviation at a `SystemTime`, plus its prior and next transition.

Resolving a local `DateTime` may find no instant or two instants during a clock
change. `Disambiguation` selects compatible, earlier, later, or reject behavior.

## ZonedDateTime

`ZonedDateTime` joins one exact instant with a timezone and its local calendar
fields. It can start from `SystemTime`, `DateTime`, current time, RFC 2822 text,
or the standard parser.

`withTimeZone` keeps the instant and changes its displayed local fields.
Duration arithmetic follows elapsed time. Period arithmetic follows the local
calendar and resolves the result in the zone.

## Formatting

`DateTime\Formatter::fromPattern()` builds a checked pattern. Ready formatters
cover ISO date, ISO time, ISO date-time, RFC 3339, RFC 2822, Temporal text, and
HTTP dates.

The formatter has separate methods for `Date`, `Time`, `DateTime`, and
`ZonedDateTime`. Parsing methods return `null` on invalid text; `from` methods
throw a domain error.
