//! The ring buffer used by Whim data structures.

use std::cell::RefCell;
use std::collections::VecDeque;

use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::BuiltInChildren;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::queue::DropQueue;

#[whim_class("Whim\\_Private\\RingBuffer<T>", final, traced)]
#[derive(Default)]
pub(crate) struct RingBuffer {
    values: RefCell<VecDeque<Value>>,
}

default_built_in_state!(RingBuffer);

// SAFETY: the deque owns every stored value and teardown drains it once.
unsafe impl BuiltInChildren for RingBuffer {
    fn enqueue_built_in_children(&mut self, queue: &DropQueue, mode: TeardownMode) {
        for value in self.values.get_mut().drain(..) {
            queue.release_value(value, mode);
        }
    }

    fn visit_built_in_children(&self, visitor: &mut TraceVisitor<'_>) {
        for value in self.values.borrow().iter() {
            if let Some(child) = value.collectable_box() {
                visitor.visit(child);
            }
        }
    }
}

#[whim_methods(generics = "<T>")]
impl RingBuffer {
    #[whim_method("__construct(): void", no_track_caller, no_trace_boundary)]
    const fn construct() {}

    #[whim_method("count(): (0..)", no_track_caller, no_trace_boundary, must_use)]
    fn count(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let count = context.state::<Self>()?.values.borrow().len();
        // SAFETY: the surrounding invariant proves this result is successful.
        let count = unsafe {
            unwrap_result_invariant(
                i64::try_from(count),
                "a ring buffer cannot exhaust the signed integer range",
            )
        };
        Ok(Value::int(count))
    }

    #[whim_method("isEmpty(): bool", no_track_caller, no_trace_boundary, must_use)]
    fn is_empty(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let empty = context.state::<Self>()?.values.borrow().is_empty();
        Ok(Value::bool(empty))
    }

    #[whim_method("pushFront(T $value): void", no_track_caller, no_trace_boundary)]
    fn push_front(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let value = arguments.local(0);
        context
            .state::<Self>()?
            .values
            .borrow_mut()
            .push_front(value);
        Ok(Value::null())
    }

    #[whim_method("pushBack(T $value): void", no_track_caller, no_trace_boundary)]
    fn push_back(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let value = arguments.local(0);
        context
            .state::<Self>()?
            .values
            .borrow_mut()
            .push_back(value);
        Ok(Value::null())
    }

    #[whim_method("popFront(): null|(T,)", no_track_caller, no_trace_boundary, must_use)]
    fn pop_front(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let value = context.state::<Self>()?.values.borrow_mut().pop_front();
        Ok(optional_value(context, value))
    }

    #[whim_method("popFrontUnsafe(): T", no_track_caller, no_trace_boundary, must_use)]
    fn pop_front_unsafe(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let value = context.state::<Self>()?.values.borrow_mut().pop_front();
        // SAFETY: the surrounding invariant proves this option contains a value.
        Ok(unsafe { unwrap_option_invariant(value, "the ring buffer is not empty") })
    }

    #[whim_method("popBack(): null|(T,)", no_track_caller, no_trace_boundary, must_use)]
    fn pop_back(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let value = context.state::<Self>()?.values.borrow_mut().pop_back();
        Ok(optional_value(context, value))
    }

    #[whim_method("peekFront(): null|(T,)", no_track_caller, no_trace_boundary, must_use)]
    fn peek_front(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let value = context.state::<Self>()?.values.borrow().front().cloned();
        Ok(optional_value(context, value))
    }

    #[whim_method("peekBack(): null|(T,)", no_track_caller, no_trace_boundary, must_use)]
    fn peek_back(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let value = context.state::<Self>()?.values.borrow().back().cloned();
        Ok(optional_value(context, value))
    }

    #[whim_method("clear(): void", no_track_caller, no_trace_boundary)]
    fn clear(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        context.state::<Self>()?.values.borrow_mut().clear();
        Ok(Value::null())
    }

    #[whim_method("toVec(): vec<T>", no_track_caller, no_trace_boundary, must_use)]
    fn to_vec(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let values = context.state::<Self>()?.values.borrow().clone();
        Ok(context.vec(values))
    }
}

fn optional_value(context: &Context<'_, '_, '_>, value: Option<Value>) -> Value {
    value.map_or_else(Value::null, |value| context.tuple([value]))
}
