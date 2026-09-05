//! Date, time, and timezone primitives.

use std::cell::OnceCell;
use std::str::from_utf8;

use jiff::SignedDuration;
use jiff::Span;
use jiff::Timestamp;
use jiff::Zoned;
use jiff::civil::Date;
use jiff::civil::DateTime;
use jiff::civil::Time;
use jiff::fmt::rfc2822;
use jiff::fmt::strtime;
use jiff::fmt::temporal;
use jiff::tz::Disambiguation;
use jiff::tz::Offset;
use jiff::tz::TimeZone;
use whim_macros::whim_class;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::value::Value;

const DATE_TIME_ZONE: &str = "Whim\\_Private\\DateTimeZone";
const DATE_TIME_FORMATTER: &str = "Whim\\_Private\\DateTimeFormatter";

const FORMAT_ISO_DATE: i64 = 0;
const FORMAT_ISO_TIME: i64 = 1;
const FORMAT_ISO_DATE_TIME: i64 = 2;
const FORMAT_RFC_3339: i64 = 3;
const FORMAT_RFC_2822: i64 = 4;
const FORMAT_TEMPORAL: i64 = 5;
const FORMAT_HTTP_DATE: i64 = 6;

struct ZoneState {
    timezone: TimeZone,
    identifier: String,
}

#[whim_class("Whim\\_Private\\DateTimeZone", final)]
#[derive(Default)]
pub(crate) struct DateTimeZone {
    state: OnceCell<ZoneState>,
}

default_built_in_state!(DateTimeZone);

#[whim_methods]
impl DateTimeZone {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("parse(string $identifier): null|Whim\\_Private\\DateTimeZone", static)]
    fn parse<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let identifier = arguments.bytes(0);
        let Ok(identifier) = from_utf8(identifier) else {
            return Ok(Value::null());
        };
        let Some(timezone) = parse_timezone(identifier) else {
            return Ok(Value::null());
        };

        let identifier = timezone_identifier(&timezone);
        build_timezone(context, timezone, identifier)
    }

    #[whim_method("system(): null|Whim\\_Private\\DateTimeZone", static)]
    fn system(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let Ok(timezone) = TimeZone::try_system() else {
            return Ok(Value::null());
        };
        let identifier = timezone_identifier(&timezone);
        build_timezone(context, timezone, identifier)
    }

    #[whim_method("fixedOffset(int $seconds): null|Whim\\_Private\\DateTimeZone", static)]
    fn fixed_offset<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let seconds = arguments.int(0);
        let Some(offset) = i32::try_from(seconds)
            .ok()
            .and_then(|seconds| Offset::from_seconds(seconds).ok())
        else {
            return Ok(Value::null());
        };
        let timezone = TimeZone::fixed(offset);
        build_timezone(context, timezone, offset.to_string())
    }

    #[whim_method("identifier(): string")]
    fn identifier(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_timezone(context, |context, state| {
            context.string(state.identifier.as_bytes())
        })
    }

    #[whim_method("offsetAt(int $seconds, int $nanoseconds): null|int")]
    fn offset_at<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(timestamp) = timestamp_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        with_timezone(context, |_, state| {
            Value::int(i64::from(state.timezone.to_offset(timestamp).seconds()))
        })
    }

    #[whim_method("abbreviationAt(int $seconds, int $nanoseconds): null|string")]
    fn abbreviation_at<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(timestamp) = timestamp_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        with_timezone(context, |context, state| {
            let info = state.timezone.to_offset_info(timestamp);
            context.string(info.abbreviation().as_bytes())
        })
    }

    #[whim_method("previousTransition(int $seconds, int $nanoseconds): null|(int, int)")]
    fn previous_transition<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(timestamp) = timestamp_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        with_timezone(context, |context, state| {
            state
                .timezone
                .preceding(timestamp)
                .next()
                .map_or_else(Value::null, |transition| {
                    timestamp_value(context, transition.timestamp())
                })
        })
    }

    #[whim_method("nextTransition(int $seconds, int $nanoseconds): null|(int, int)")]
    fn next_transition<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(timestamp) = timestamp_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        with_timezone(context, |context, state| {
            state
                .timezone
                .following(timestamp)
                .next()
                .map_or_else(Value::null, |transition| {
                    timestamp_value(context, transition.timestamp())
                })
        })
    }
}

enum FormatterKind {
    Pattern(String),
    Standard(i64),
}

#[whim_class("Whim\\_Private\\DateTimeFormatter", final)]
#[derive(Default)]
pub(crate) struct DateTimeFormatter {
    kind: OnceCell<FormatterKind>,
}

default_built_in_state!(DateTimeFormatter);

#[whim_methods]
impl DateTimeFormatter {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "fromPattern(string $pattern): null|Whim\\_Private\\DateTimeFormatter",
        static
    )]
    fn from_pattern<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let pattern = arguments.bytes(0);
        let Ok(pattern) = from_utf8(pattern) else {
            return Ok(Value::null());
        };
        if strtime::format(pattern, &Zoned::UNIX_EPOCH).is_err() {
            return Ok(Value::null());
        }

        build_formatter(context, FormatterKind::Pattern(pattern.to_owned()))
    }

    #[whim_method("standard(int $kind): Whim\\_Private\\DateTimeFormatter", static)]
    fn standard<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let kind = arguments.int(0);
        build_formatter(context, FormatterKind::Standard(kind))
    }

    #[whim_method("formatDate(int $year, int $month, int $day): null|string")]
    fn format_date<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(date) = date_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        with_formatter(context, |context, kind| {
            let formatted = match kind {
                FormatterKind::Pattern(pattern) => strtime::format(pattern, date).ok(),
                FormatterKind::Standard(FORMAT_ISO_DATE) => Some(date.to_string()),
                FormatterKind::Standard(_) => None,
            };
            optional_string(context, formatted)
        })
    }

    #[whim_method("formatTime(int $hour, int $minute, int $second, int $nanosecond): null|string")]
    fn format_time<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(time) = time_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        with_formatter(context, |context, kind| {
            let formatted = match kind {
                FormatterKind::Pattern(pattern) => strtime::format(pattern, time).ok(),
                FormatterKind::Standard(FORMAT_ISO_TIME) => Some(time.to_string()),
                FormatterKind::Standard(_) => None,
            };
            optional_string(context, formatted)
        })
    }

    #[whim_method(
        "formatDateTime(int $year, int $month, int $day, int $hour, int $minute, int $second, int $nanosecond): null|string"
    )]
    fn format_date_time<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(date_time) = date_time_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        with_formatter(context, |context, kind| {
            let formatted = match kind {
                FormatterKind::Pattern(pattern) => strtime::format(pattern, date_time).ok(),
                FormatterKind::Standard(FORMAT_ISO_DATE_TIME) => Some(date_time.to_string()),
                FormatterKind::Standard(_) => None,
            };
            optional_string(context, formatted)
        })
    }

    #[whim_method(
        "formatZonedDateTime(int $year, int $month, int $day, int $hour, int $minute, int $second, int $nanosecond, int $offsetSeconds, string $timezone, string $abbreviation): null|string"
    )]
    fn format_zoned_date_time<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let Some(date_time) = date_time_from_arguments(arguments, 0) else {
            return Ok(Value::null());
        };
        let offset_seconds = arguments.int(7);
        let Some(offset) = i32::try_from(offset_seconds)
            .ok()
            .and_then(|seconds| Offset::from_seconds(seconds).ok())
        else {
            return Ok(Value::null());
        };
        let timezone = arguments.bytes(8);
        let Ok(timezone) = from_utf8(timezone) else {
            return Ok(Value::null());
        };
        let Some(timestamp) = offset.to_timestamp(date_time).ok() else {
            return Ok(Value::null());
        };
        let timezone = parse_timezone(timezone).unwrap_or_else(|| TimeZone::fixed(offset));
        let zoned = timestamp.to_zoned(timezone);

        with_formatter(context, |context, kind| {
            let formatted = format_zoned(kind, &zoned);
            optional_string(context, formatted)
        })
    }
}

#[whim_function("Whim\\_Private\\date_time_is_valid_date(int $year, int $month, int $day): bool")]
pub(crate) fn is_valid_date(arguments: Arguments<'_>) -> Value {
    Value::bool(date_from_arguments(arguments, 0).is_some())
}

#[whim_function(
    "Whim\\_Private\\date_time_is_valid_time(int $hour, int $minute, int $second, int $nanosecond): bool"
)]
pub(crate) fn is_valid_time(arguments: Arguments<'_>) -> Value {
    Value::bool(time_from_arguments(arguments, 0).is_some())
}

#[whim_function("Whim\\_Private\\date_time_parse_date(string $value): null|(int, int, int)")]
pub(crate) fn parse_date(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let date = temporal::DateTimeParser::new().parse_date(value).ok();
    date.map_or_else(Value::null, |date| date_value(context, date))
}

#[whim_function("Whim\\_Private\\date_time_parse_time(string $value): null|(int, int, int, int)")]
pub(crate) fn parse_time(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let time = temporal::DateTimeParser::new().parse_time(value).ok();
    time.map_or_else(Value::null, |time| time_value(context, time))
}

#[whim_function(
    "Whim\\_Private\\date_time_parse_date_time(string $value): null|(int, int, int, int, int, int, int)"
)]
pub(crate) fn parse_date_time(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let date_time = temporal::DateTimeParser::new().parse_datetime(value).ok();
    date_time.map_or_else(Value::null, |date_time| date_time_value(context, date_time))
}

#[whim_function("Whim\\_Private\\date_time_parse_zoned(string $value): null|(int, int, string)")]
pub(crate) fn parse_zoned(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let Some(zoned) = temporal::DateTimeParser::new().parse_zoned(value).ok() else {
        return Value::null();
    };
    let timestamp = zoned.timestamp();
    let identifier = timezone_identifier(zoned.time_zone());
    context.tuple([
        Value::int(timestamp.as_second()),
        Value::int(i64::from(timestamp.subsec_nanosecond())),
        context.string(identifier.as_bytes()),
    ])
}

#[whim_function("Whim\\_Private\\date_time_parse_rfc2822(string $value): null|(int, int, int)")]
pub(crate) fn parse_rfc2822(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let value = arguments.bytes(0);
    let Ok(value) = from_utf8(value) else {
        return Value::null();
    };
    let Ok(zoned) = rfc2822::parse(value) else {
        return Value::null();
    };
    let timestamp = zoned.timestamp();
    context.tuple([
        Value::int(timestamp.as_second()),
        Value::int(i64::from(timestamp.subsec_nanosecond())),
        Value::int(i64::from(zoned.offset().seconds())),
    ])
}

#[whim_function(
    "Whim\\_Private\\date_time_date_metadata(int $year, int $month, int $day): (int, int, int)"
)]
pub(crate) fn date_metadata(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Some(date) = date_from_arguments(arguments, 0) else {
        return context.tuple([Value::int(1), Value::int(1), Value::int(31)]);
    };
    context.tuple([
        Value::int(i64::from(date.weekday().to_monday_one_offset())),
        Value::int(i64::from(date.day_of_year())),
        Value::int(i64::from(date.days_in_month())),
    ])
}

#[whim_function(
    "Whim\\_Private\\date_time_shift_date(int $year, int $month, int $day, int $years, int $months, int $weeks, int $days, int $overflow): null|(int, int, int)"
)]
pub(crate) fn shift_date(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Some(date) = date_from_arguments(arguments, 0) else {
        return Value::null();
    };
    let years = arguments.int(3);
    let months = arguments.int(4);
    let weeks = arguments.int(5);
    let days = arguments.int(6);
    let reject_overflow = arguments.int(7) == 1;
    let shifted = shift_calendar_date(date, years, months, weeks, days, reject_overflow);
    shifted.map_or_else(Value::null, |date| date_value(context, date))
}

#[whim_function(
    "Whim\\_Private\\date_time_shift_date_time(int $year, int $month, int $day, int $hour, int $minute, int $second, int $nanosecond, int $years, int $months, int $weeks, int $days, int $durationSeconds, int $durationNanoseconds, int $overflow): null|(int, int, int, int, int, int, int)"
)]
pub(crate) fn shift_date_time(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Some(date_time) = date_time_from_arguments(arguments, 0) else {
        return Value::null();
    };
    let years = arguments.int(7);
    let months = arguments.int(8);
    let weeks = arguments.int(9);
    let days = arguments.int(10);
    let duration_seconds = arguments.int(11);
    let duration_nanoseconds = arguments.int(12);
    let reject_overflow = arguments.int(13) == 1;
    if !(-999_999_999..=999_999_999).contains(&duration_nanoseconds) {
        return Value::null();
    }
    let Some(duration_nanoseconds) = i32::try_from(duration_nanoseconds).ok() else {
        return Value::null();
    };
    let Some(date) = shift_calendar_date(
        date_time.date(),
        years,
        months,
        weeks,
        days,
        reject_overflow,
    ) else {
        return Value::null();
    };
    let shifted = date
        .to_datetime(date_time.time())
        .checked_add(SignedDuration::new(duration_seconds, duration_nanoseconds))
        .ok();
    shifted.map_or_else(Value::null, |date_time| date_time_value(context, date_time))
}

#[whim_function(
    "Whim\\_Private\\date_time_resolve(Whim\\_Private\\DateTimeZone $timezone, int $year, int $month, int $day, int $hour, int $minute, int $second, int $nanosecond, int $disambiguation): null|(int, int)"
)]
pub(crate) fn resolve(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let timezone = arguments.local(0);
    let Some(state) = state_ref::<DateTimeZone>(&timezone).and_then(|zone| zone.state.get()) else {
        return Value::null();
    };
    let Some(date_time) = date_time_from_arguments(arguments, 1) else {
        return Value::null();
    };
    let strategy = match arguments.int(8) {
        0 => Disambiguation::Compatible,
        1 => Disambiguation::Earlier,
        2 => Disambiguation::Later,
        3 => Disambiguation::Reject,
        _ => return Value::null(),
    };
    let resolved = state
        .timezone
        .to_ambiguous_timestamp(date_time)
        .disambiguate(strategy)
        .ok();
    resolved.map_or_else(Value::null, |timestamp| timestamp_value(context, timestamp))
}

#[whim_function(
    "Whim\\_Private\\date_time_localize(Whim\\_Private\\DateTimeZone $timezone, int $seconds, int $nanoseconds): null|(int, int, int, int, int, int, int, int, string)"
)]
pub(crate) fn localize(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let timezone = arguments.local(0);
    let Some(state) = state_ref::<DateTimeZone>(&timezone).and_then(|zone| zone.state.get()) else {
        return Value::null();
    };
    let Some(timestamp) = timestamp_from_arguments(arguments, 1) else {
        return Value::null();
    };
    let date_time = state.timezone.to_datetime(timestamp);
    let info = state.timezone.to_offset_info(timestamp);
    context.tuple([
        Value::int(i64::from(date_time.year())),
        Value::int(i64::from(date_time.month())),
        Value::int(i64::from(date_time.day())),
        Value::int(i64::from(date_time.hour())),
        Value::int(i64::from(date_time.minute())),
        Value::int(i64::from(date_time.second())),
        Value::int(i64::from(date_time.subsec_nanosecond())),
        Value::int(i64::from(info.offset().seconds())),
        context.string(info.abbreviation().as_bytes()),
    ])
}

fn build_timezone(
    context: &mut Context<'_, '_, '_>,
    timezone: TimeZone,
    identifier: String,
) -> Result<Value, Throw> {
    let object = context.new_built_in_instance(DATE_TIME_ZONE)?;
    let Some(state) = state_ref::<DateTimeZone>(&object) else {
        return Err(context.type_error("the timezone has no built-in state"));
    };
    state
        .state
        .set(ZoneState {
            timezone,
            identifier,
        })
        .map_err(|_| context.type_error("the timezone is already initialized"))?;
    Ok(object)
}

fn build_formatter(context: &mut Context<'_, '_, '_>, kind: FormatterKind) -> Result<Value, Throw> {
    let object = context.new_built_in_instance(DATE_TIME_FORMATTER)?;
    let Some(state) = state_ref::<DateTimeFormatter>(&object) else {
        return Err(context.type_error("the formatter has no built-in state"));
    };
    state
        .kind
        .set(kind)
        .map_err(|_| context.type_error("the formatter is already initialized"))?;
    Ok(object)
}

fn with_timezone<'call>(
    context: &mut Context<'call, '_, '_>,
    operation: impl FnOnce(&Context<'call, '_, '_>, &ZoneState) -> Value,
) -> Result<Value, Throw> {
    let receiver = context.receiver();
    let Some(state) = state_ref::<DateTimeZone>(&receiver).and_then(|zone| zone.state.get()) else {
        return Err(context.type_error("the timezone is not initialized"));
    };
    Ok(operation(context, state))
}

fn with_formatter<'call>(
    context: &mut Context<'call, '_, '_>,
    operation: impl FnOnce(&Context<'call, '_, '_>, &FormatterKind) -> Value,
) -> Result<Value, Throw> {
    let receiver = context.receiver();
    let Some(kind) =
        state_ref::<DateTimeFormatter>(&receiver).and_then(|formatter| formatter.kind.get())
    else {
        return Err(context.type_error("the date-time formatter is not initialized"));
    };
    Ok(operation(context, kind))
}

fn parse_timezone(identifier: &str) -> Option<TimeZone> {
    if let Ok(timezone) = TimeZone::get(identifier) {
        return Some(timezone);
    }

    let timezone = temporal::DateTimeParser::new()
        .parse_time_zone(identifier)
        .ok()?;
    timezone.to_fixed_offset().ok()?;
    Some(timezone)
}

fn timezone_identifier(timezone: &TimeZone) -> String {
    if let Some(identifier) = timezone.iana_name() {
        return identifier.to_owned();
    }
    if let Ok(offset) = timezone.to_fixed_offset() {
        return offset.to_string();
    }
    "Etc/Unknown".to_owned()
}

fn timestamp_from_arguments(arguments: Arguments<'_>, start: usize) -> Option<Timestamp> {
    let seconds = arguments.int(start);
    let nanoseconds = arguments.int(start + 1);
    let nanoseconds = i32::try_from(nanoseconds).ok()?;
    Timestamp::new(seconds, nanoseconds).ok()
}

fn date_from_arguments(arguments: Arguments<'_>, start: usize) -> Option<Date> {
    let year = i16::try_from(arguments.int(start)).ok()?;
    let month = i8::try_from(arguments.int(start + 1)).ok()?;
    let day = i8::try_from(arguments.int(start + 2)).ok()?;
    Date::new(year, month, day).ok()
}

fn time_from_arguments(arguments: Arguments<'_>, start: usize) -> Option<Time> {
    let hour = i8::try_from(arguments.int(start)).ok()?;
    let minute = i8::try_from(arguments.int(start + 1)).ok()?;
    let second = i8::try_from(arguments.int(start + 2)).ok()?;
    let nanosecond = i32::try_from(arguments.int(start + 3)).ok()?;
    Time::new(hour, minute, second, nanosecond).ok()
}

fn date_time_from_arguments(arguments: Arguments<'_>, start: usize) -> Option<DateTime> {
    let date = date_from_arguments(arguments, start)?;
    let time = time_from_arguments(arguments, start + 3)?;
    Some(date.to_datetime(time))
}

fn shift_calendar_date(
    date: Date,
    years: i64,
    months: i64,
    weeks: i64,
    days: i64,
    reject_overflow: bool,
) -> Option<Date> {
    let calendar = Span::new().years(years).months(months);
    let shifted = date.checked_add(calendar).ok()?;
    if reject_overflow && shifted.day() != date.day() {
        return None;
    }
    shifted
        .checked_add(Span::new().weeks(weeks).days(days))
        .ok()
}

fn format_zoned(kind: &FormatterKind, zoned: &Zoned) -> Option<String> {
    match kind {
        FormatterKind::Pattern(pattern) => strtime::format(pattern, zoned).ok(),
        FormatterKind::Standard(FORMAT_ISO_DATE) => Some(zoned.date().to_string()),
        FormatterKind::Standard(FORMAT_ISO_TIME) => Some(zoned.time().to_string()),
        FormatterKind::Standard(FORMAT_ISO_DATE_TIME) => Some(zoned.datetime().to_string()),
        FormatterKind::Standard(FORMAT_RFC_3339) => Some(
            temporal::DateTimePrinter::new()
                .timestamp_with_offset_to_string(&zoned.timestamp(), zoned.offset()),
        ),
        FormatterKind::Standard(FORMAT_RFC_2822) => rfc2822::to_string(zoned).ok(),
        FormatterKind::Standard(FORMAT_TEMPORAL) => Some(zoned.to_string()),
        FormatterKind::Standard(FORMAT_HTTP_DATE) => {
            let mut formatted = String::new();
            rfc2822::DateTimePrinter::new()
                .print_timestamp_rfc9110(&zoned.timestamp(), &mut formatted)
                .ok()?;
            Some(formatted)
        }
        FormatterKind::Standard(_) => None,
    }
}

fn optional_string(context: &Context<'_, '_, '_>, value: Option<String>) -> Value {
    value.map_or_else(Value::null, |value| {
        context.owned_string(value.into_bytes())
    })
}

fn timestamp_value(context: &Context<'_, '_, '_>, timestamp: Timestamp) -> Value {
    context.tuple([
        Value::int(timestamp.as_second()),
        Value::int(i64::from(timestamp.subsec_nanosecond())),
    ])
}

fn date_value(context: &Context<'_, '_, '_>, date: Date) -> Value {
    context.tuple([
        Value::int(i64::from(date.year())),
        Value::int(i64::from(date.month())),
        Value::int(i64::from(date.day())),
    ])
}

fn time_value(context: &Context<'_, '_, '_>, time: Time) -> Value {
    context.tuple([
        Value::int(i64::from(time.hour())),
        Value::int(i64::from(time.minute())),
        Value::int(i64::from(time.second())),
        Value::int(i64::from(time.subsec_nanosecond())),
    ])
}

fn date_time_value(context: &Context<'_, '_, '_>, date_time: DateTime) -> Value {
    context.tuple([
        Value::int(i64::from(date_time.year())),
        Value::int(i64::from(date_time.month())),
        Value::int(i64::from(date_time.day())),
        Value::int(i64::from(date_time.hour())),
        Value::int(i64::from(date_time.minute())),
        Value::int(i64::from(date_time.second())),
        Value::int(i64::from(date_time.subsec_nanosecond())),
    ])
}
