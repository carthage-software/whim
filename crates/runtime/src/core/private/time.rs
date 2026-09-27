//! Unstable clock primitives exposed to the Whim standard library.

use std::sync::OnceLock;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use whim_macros::whim_function;
use whim_value::Value;

use crate::builtin::Context;

#[whim_function("Whim\\_Private\\get_system_time(): (int, uint)")]
pub(crate) fn get_system_time(scope: &Context<'_, '_, '_>) -> Value {
    let (seconds, nanoseconds) = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => (
            saturating_seconds(duration.as_secs()),
            i64::from(duration.subsec_nanos()),
        ),
        Err(error) => {
            let duration = error.duration();
            let nanoseconds = i64::from(duration.subsec_nanos());
            if nanoseconds == 0 {
                (-saturating_seconds(duration.as_secs()), 0)
            } else {
                (
                    saturating_seconds(duration.as_secs())
                        .saturating_neg()
                        .saturating_sub(1),
                    1_000_000_000 - nanoseconds,
                )
            }
        }
    };

    let seconds = Value::int(seconds);
    let nanoseconds = Value::uint(nanoseconds.cast_unsigned());
    scope.tuple([seconds, nanoseconds])
}

#[whim_function("Whim\\_Private\\get_high_resolution_time(): (uint, uint)")]
pub(crate) fn get_high_resolution_time(scope: &Context<'_, '_, '_>) -> Value {
    let elapsed = high_resolution_origin().elapsed();
    let seconds = Value::uint(elapsed.as_secs());
    let nanoseconds = Value::uint(u64::from(elapsed.subsec_nanos()));
    scope.tuple([seconds, nanoseconds])
}

#[whim_function("Whim\\_Private\\get_high_resolution_nanoseconds(): uint")]
pub(crate) fn get_high_resolution_nanoseconds() -> Value {
    let elapsed = high_resolution_origin().elapsed();
    let nanoseconds = elapsed
        .as_secs()
        .saturating_mul(1_000_000_000)
        .saturating_add(u64::from(elapsed.subsec_nanos()));

    Value::uint(nanoseconds)
}

fn saturating_seconds(seconds: u64) -> i64 {
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

fn high_resolution_origin() -> &'static Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now)
}
