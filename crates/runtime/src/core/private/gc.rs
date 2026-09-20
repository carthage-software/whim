//! Explicit access to the cycle collector.

use whim_base::unwrap_result_invariant;
use whim_macros::whim_function;
use whim_value::Value;

use crate::builtin::Context;

/// Runs one cycle collection and returns the number of reclaimed values.
#[whim_function("Whim\\GC\\collect_cycles(): int")]
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
