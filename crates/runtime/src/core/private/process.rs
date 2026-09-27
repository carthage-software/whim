//! Process identifiers.

use std::process;

use whim_macros::whim_function;
use whim_value::Value;

#[whim_function(
    "Whim\\Process\\get_id(): (1u..)",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn getmypid() -> Value {
    Value::uint(u64::from(process::id()))
}
