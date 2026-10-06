# Cron Expressions

`Whim\Cron\Expression` parses five-field cron expressions, checks whether a
time matches, and finds the next or previous occurrence in a chosen timezone.
It computes times; your application decides how to wait for them and run work.

## Parsing and matching

`Expression::parse()` returns an expression or `null` for invalid syntax.
`Expression::from()` throws `Whim\Unwind\InvalidArgumentException` on invalid
syntax. Expressions are readonly and implement `Whim\Convert\ToString`.
`toString()` returns the input with outer whitespace removed; it keeps macros,
letter case, and spacing between fields.

```whim
use Whim\Cron\Expression;
use Whim\DateTime\TimeZone;
use Whim\DateTime\ZonedDateTime;

$schedule = Expression::from('*/15 9-17 * * MON-FRI');
$zone = TimeZone::utc();
$time = ZonedDateTime::from('2026-08-21T10:15:30+00:00[UTC]')->systemTime;

assert!($schedule->matches($time, $zone));
assert!($schedule->toString() == '*/15 9-17 * * MON-FRI');
assert!(Expression::parse('60 * * * *') == null);

$next = $schedule->next($time, $zone);
assert!($zone->at($next)->dateTime->toString() == '2026-08-21T10:30:00');
$previous = $schedule->previous($time, $zone);
assert!($zone->at($previous)->dateTime->toString() == '2026-08-21T10:15:00');
```

| Method                   | Result                                                    |
| ------------------------ | --------------------------------------------------------- |
| `parse(expression)`      | An `Expression`, or `null` for invalid syntax.            |
| `from(expression)`       | An `Expression`, or an `InvalidArgumentException`.        |
| `matches(time, zone)`    | Whether the minute containing the `SystemTime` matches.   |
| `next(after, zone)`      | The first occurrence strictly after the supplied time.    |
| `previous(before, zone)` | The last occurrence strictly before the supplied time.    |
| `toString()`             | The expression as written, with outer whitespace removed. |

Occurrences fall on minute boundaries. `next()` skips an occurrence exactly at
the supplied time. `previous()` also skips an exact occurrence, but can return
the start of the same minute when the supplied time includes seconds or
nanoseconds.

## Fields and syntax

The fields are minute, hour, day of month, month, and day of week:

| Field        | Values                                                 |
| ------------ | ------------------------------------------------------ |
| Minute       | `0`–`59`                                               |
| Hour         | `0`–`23`                                               |
| Day of month | `1`–`31`                                               |
| Month        | `1`–`12`, or `JAN`–`DEC`                               |
| Day of week  | `0`–`7`, or `SUN`–`SAT`; both `0` and `7` mean Sunday. |

Names ignore case. Separate fields with spaces or tabs. Each field accepts:

- `*` for every value.
- Lists such as `1,15,30`.
- Inclusive ranges such as `9-17` or `MON-FRI`.
- Positive steps such as `*/15`, `1-30/7`, or `5/17`. A single value with a step
  runs from that value to the field's maximum; `5/17` selects minutes
  `5`, `22`, `39`, and `56`.

A step applies within its field. It does not measure elapsed time across
hours, days, or timezone changes. The parser rejects descending ranges, zero
or negative steps, unknown names, and values outside the field's range.

## Day matching

When both day fields are restricted, a date matches either field. For example,
`0 0 13 * FRI` runs at midnight on every Friday and every thirteenth day of
the month.

A day field is unrestricted only when it is exactly `*`. A stepped field such
as `*/2` counts as restricted. When one day field is unrestricted, the other
field alone selects the days. When both are unrestricted, every day matches.
The minute, hour, and month fields must always match.

## Macros

Macros ignore case and expand to these five-field forms:

| Macro                  | Expression  |
| ---------------------- | ----------- |
| `@yearly`, `@annually` | `0 0 1 1 *` |
| `@monthly`             | `0 0 1 * *` |
| `@weekly`              | `0 0 * * 0` |
| `@daily`, `@midnight`  | `0 0 * * *` |
| `@hourly`              | `0 * * * *` |

The parser accepts five fields and these macros. It does not accept a seconds
field, `@reboot`, `?`, `L`, `W`, or `#`.

## Timezones and clock changes

Every query takes a `Whim\DateTime\TimeZone`. The same expression can serve
more than one zone; it does not store a zone or use the system zone by default.

When clocks move backward, a local minute can occur twice. Both instants
participate in matching and in next or previous searches:

```whim
use Whim\Cron\Expression;
use Whim\DateTime\TimeZone;
use Whim\DateTime\ZonedDateTime;

$schedule = Expression::from('30 2 * * *');
$paris = TimeZone::from('Europe/Paris');
$first = ZonedDateTime::from('2026-10-25T00:30:00+00:00[UTC]')->systemTime;
$second = ZonedDateTime::from('2026-10-25T01:30:00+00:00[UTC]')->systemTime;

assert!($schedule->matches($first, $paris));
assert!($schedule->matches($second, $paris));
assert!($schedule->next($first, $paris)->equals($second));
assert!($schedule->previous($second, $paris)->equals($first));
```

When clocks move forward, a missing local time shifts forward by the size of
the gap. In Paris, a scheduled `02:30` becomes `03:30` on the spring change.
That shifted occurrence also matches `matches()`. This rule handles clock
changes of other sizes, including half an hour.

## Expressions that cannot occur

Valid syntax does not guarantee a calendar occurrence. For example,
`0 0 30 2 *` asks for February 30. `next()` searches through the calendar year
five years after the supplied time. `previous()` searches back through the year
five years before it. Both throw `Whim\Cron\UnmatchableExpressionException`
when they find no occurrence within that limit.

The search also follows the range supported by [Time and Calendars](time.md).
