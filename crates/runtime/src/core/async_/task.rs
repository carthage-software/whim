//! Generic event-loop tasks exposed to the Whim-written async library.

use std::cell::Cell;
use std::time::Duration;

use whim_loop::TaskId as LoopTaskId;
use whim_macros::whim_class;
use whim_macros::whim_closure;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;

const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;

#[whim_class("Whim\\_Private\\TaskId", final, readonly)]
#[whim_property("public int $id")]
#[derive(Default)]
pub(crate) struct TaskId {
    task: Cell<Option<LoopTaskId>>,
}

default_built_in_state!(TaskId);

#[whim_methods]
impl TaskId {
    #[whim_method("__construct(int $id): void", visibility = "private")]
    const fn construct() {}
}

fn duration_from_parts(seconds: i64, nanoseconds: i64) -> Duration {
    if seconds < 0 || (seconds == 0 && nanoseconds <= 0) {
        return Duration::ZERO;
    }

    let seconds = seconds.cast_unsigned();
    // SAFETY: the surrounding invariant proves this result is successful.
    let nanoseconds = unsafe {
        unwrap_result_invariant(
            u32::try_from(nanoseconds.clamp(0, NANOSECONDS_PER_SECOND - 1)),
            "clamped duration nanoseconds fit in u32",
        )
    };
    Duration::new(seconds, nanoseconds)
}

fn duration_of(cx: &mut Context<'_, '_, '_>, value: &Value) -> Result<Duration, Throw> {
    let duration = value
        .as_object()
        .cloned()
        .ok_or_else(|| cx.type_error("the duration must be an object"))?;
    let duration = Value::object(duration);
    let seconds = cx.get_property(&duration, "seconds")?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let seconds = unsafe {
        unwrap_option_invariant(seconds.as_int(), "`Duration::seconds` is declared as int")
    };
    let nanoseconds = cx.get_property(&duration, "nanoseconds")?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let nanoseconds = unsafe {
        unwrap_option_invariant(
            nanoseconds.as_int(),
            "`Duration::nanoseconds` is declared as int",
        )
    };

    Ok(duration_from_parts(seconds, nanoseconds))
}

pub(crate) fn task_value(cx: &mut Context<'_, '_, '_>, id: LoopTaskId) -> Result<Value, Throw> {
    let task = cx.new_built_in_instance("Whim\\_Private\\TaskId")?;
    let Some(state) = state_ref::<TaskId>(&task) else {
        return Err(cx.type_error("the task identifier has no built-in state"));
    };
    state.task.set(Some(id));
    // SAFETY: the surrounding invariant proves this result is successful.
    let id = unsafe {
        unwrap_result_invariant(
            i64::try_from(id.get()),
            "a task identifier cannot exhaust the signed integer range",
        )
    };
    let value = Value::int(id);
    cx.set_property(&task, "id", value)?;
    Ok(task)
}

fn task_of(cx: &mut Context<'_, '_, '_>, value: &Value) -> Result<LoopTaskId, Throw> {
    state_ref::<TaskId>(value)
        .and_then(|task| task.task.get())
        .ok_or_else(|| cx.type_error("the value is not a live task identifier"))
}

#[whim_function(
    "Whim\\_Private\\delay_task(Whim\\Time\\Duration $duration, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
fn delay_task(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let duration = arguments.local(0);
    let duration = duration_of(cx, &duration)?;
    let callback = arguments.local(1);
    let task = cx.vm.loop_delay(callback, duration)?;
    task_value(cx, task)
}

#[whim_closure("(): void")]
fn repeat_runner(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let callback = cx.capture(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let seconds = unsafe {
        unwrap_option_invariant(
            cx.capture(1).as_int(),
            "a repeat runner captures integer seconds",
        )
    };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let nanoseconds = unsafe {
        unwrap_option_invariant(
            cx.capture(2).as_int(),
            "a repeat runner captures integer nanoseconds",
        )
    };
    let duration = duration_from_parts(seconds, nanoseconds);

    loop {
        cx.vm.loop_park_current_for(duration);
        cx.vm.loop_suspend()?;
        cx.vm.call_function_value(&callback, &[])?;
    }
}

#[whim_function(
    "Whim\\_Private\\repeat_task(Whim\\Time\\Duration $duration, (fn(): void) $callback): Whim\\_Private\\TaskId"
)]
fn repeat_task(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let duration = arguments.local(0);
    let duration = duration_of(cx, &duration)?;
    let callback = arguments.local(1);
    // SAFETY: the surrounding invariant proves this result is successful.
    let seconds = unsafe {
        unwrap_result_invariant(
            i64::try_from(duration.as_secs()),
            "a Whim duration cannot exhaust the signed integer range",
        )
    };
    let seconds = Value::int(seconds);
    let nanoseconds = Value::int(i64::from(duration.subsec_nanos()));
    let runner = cx.closure(repeat_runner_spec(), &[callback, seconds, nanoseconds]);
    let task = cx.vm.loop_defer(runner)?;
    task_value(cx, task)
}

#[whim_function("Whim\\_Private\\defer_task((fn(): void) $callback): Whim\\_Private\\TaskId")]
fn defer_task(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let callback = arguments.local(0);
    let task = cx.vm.loop_defer(callback)?;
    task_value(cx, task)
}

#[whim_function("Whim\\_Private\\queue_task((fn(): void) $callback): Whim\\_Private\\TaskId")]
fn queue_task(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let callback = arguments.local(0);
    let task = cx.vm.loop_queue(callback)?;
    task_value(cx, task)
}

macro_rules! task_operation {
    ($name:ident, $signature:literal, $operation:ident) => {
        #[whim_function($signature)]
        fn $name(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
            let task = arguments.local(0);
            let task = task_of(cx, &task)?;
            cx.vm.$operation(task);
            Ok(Value::null())
        }
    };
}

task_operation!(
    cancel_task,
    "Whim\\_Private\\cancel_task(Whim\\_Private\\TaskId $task): void",
    loop_cancel
);
task_operation!(
    unreference_task,
    "Whim\\_Private\\unreference_task(Whim\\_Private\\TaskId $task): void",
    loop_unreference
);
#[whim_function("Whim\\_Private\\record_unhandled(Whim\\Unwind\\Throwable $error): int")]
fn record_unhandled(
    cx: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let error = arguments.local(0);
    let id = cx.vm.loop_record_error(error)?;
    // SAFETY: the surrounding invariant proves this result is successful.
    let id = unsafe {
        unwrap_result_invariant(
            i64::try_from(id),
            "an unhandled-error identifier cannot exhaust the signed integer range",
        )
    };
    Ok(Value::int(id))
}

#[whim_function("Whim\\_Private\\forget_unhandled(int $id): void")]
fn forget_unhandled(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) {
    let id = arguments.int(0);
    if let Ok(id) = u64::try_from(id) {
        cx.vm.loop_forget_error(id);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::duration_from_parts;

    #[test]
    fn duration_preserves_large_positive_seconds() {
        let seconds = i64::from(u32::MAX) + 1;
        assert_eq!(
            duration_from_parts(seconds, 0),
            Duration::from_secs(seconds.cast_unsigned()),
        );
    }
}
