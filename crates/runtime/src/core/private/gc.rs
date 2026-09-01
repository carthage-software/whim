//! Explicit access to the cycle collector.

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::unwrap_result_invariant;
use crate::value::Value;

/// Runs one cycle collection and returns the number of reclaimed values.
#[whim_function("Whim\\_Private\\collect_cycles(): int")]
fn collect_cycles(context: &Context<'_, '_, '_>) -> Value {
    let collected = context.vm.engine.heap.collect_cycles();
    // SAFETY: the surrounding invariant proves this result is successful.
    let collected = unsafe {
        unwrap_result_invariant(
            i64::try_from(collected),
            "the heap cannot contain more than the signed integer range of values",
        )
    };
    Value::int(collected)
}
