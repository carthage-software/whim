//! Unstable process primitives exposed to the Whim standard library.

use std::process;

use whim_macros::whim_function;

use crate::value::Value;

#[whim_function(
    "Whim\\_Private\\getmypid(): (1..)",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn getmypid() -> Value {
    Value::int(i64::from(process::id()))
}
