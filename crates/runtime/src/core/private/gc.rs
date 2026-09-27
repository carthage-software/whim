//! Explicit access to the cycle collector.

use whim_macros::whim_function;
use whim_value::Value;

use crate::builtin::Context;

/// Runs one cycle collection and returns the number of reclaimed values.
#[whim_function("Whim\\GC\\collect_cycles(): uint")]
fn collect_cycles(context: &Context<'_, '_, '_>) -> Value {
    let collected = context.vm.engine.heap.collect_cycles();
    Value::uint(collected as u64)
}
