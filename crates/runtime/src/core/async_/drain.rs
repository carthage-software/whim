//! `Whim\Async\drain`: run the event loop until every task has finished.

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::value::Value;

#[whim_function("Whim\\_Private\\drain(): void")]
fn drain(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    cx.vm.drain_event_loop()
}
