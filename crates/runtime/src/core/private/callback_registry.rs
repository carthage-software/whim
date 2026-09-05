//! Callback storage for cancellation tokens.

use std::cell::RefCell;

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

struct Slot {
    generation: u32,
    callback: Option<Value>,
}

#[derive(Default)]
struct Registry {
    slots: Vec<Slot>,
    free: Vec<usize>,
    active: usize,
}

impl Registry {
    fn insert(&mut self, callback: Value) -> i64 {
        let (index, generation) = if let Some(index) = self.free.pop() {
            // SAFETY: the surrounding invariant keeps this index in bounds.
            let slot = unsafe { self.slots.get_unchecked_mut(index) };
            slot.generation = (slot.generation.wrapping_add(1) & 0x7fff_ffff).max(1);
            slot.callback = Some(callback);
            (index, slot.generation)
        } else {
            let index = self.slots.len();
            self.slots.push(Slot {
                generation: 1,
                callback: Some(callback),
            });
            (index, 1)
        };
        self.active += 1;

        // SAFETY: the surrounding invariant proves this result is successful.
        let index = unsafe {
            unwrap_result_invariant(
                u32::try_from(index),
                "a callback registry cannot exceed the thirty-two-bit index range",
            )
        };
        let identifier = (u64::from(generation) << 32) | u64::from(index);
        // SAFETY: the surrounding invariant proves this result is successful.
        unsafe {
            unwrap_result_invariant(
                i64::try_from(identifier),
                "callback generations keep identifiers in the signed integer range",
            )
        }
    }

    fn remove(&mut self, identifier: i64) {
        let Ok(identifier) = u64::try_from(identifier) else {
            return;
        };
        let Ok(generation) = u32::try_from(identifier >> 32) else {
            return;
        };
        let Ok(index) = usize::try_from(identifier & u64::from(u32::MAX)) else {
            return;
        };
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        if slot.generation != generation || slot.callback.take().is_none() {
            return;
        }

        self.free.push(index);
        // SAFETY: the surrounding invariant proves this option contains a value.
        self.active = unsafe {
            unwrap_option_invariant(
                self.active.checked_sub(1),
                "removing a live callback decreases a nonzero active count",
            )
        };
    }

    const fn is_empty(&self) -> bool {
        self.active == 0
    }

    fn drain(&mut self) -> Vec<Value> {
        let mut callbacks = Vec::with_capacity(self.active);
        for slot in &mut self.slots {
            if let Some(callback) = slot.callback.take() {
                callbacks.push(callback);
            }
        }

        self.active = 0;
        self.free.clear();
        self.free.extend(0..self.slots.len());
        callbacks
    }
}

#[whim_class("Whim\\_Private\\CallbackRegistry", final, traced)]
#[derive(Default)]
pub(crate) struct CallbackRegistry {
    registry: RefCell<Registry>,
}

default_built_in_state!(CallbackRegistry);

// SAFETY: callbacks are the sole owned values and teardown takes each once.
unsafe impl BuiltInChildren for CallbackRegistry {
    fn enqueue_built_in_children(&mut self, queue: &DropQueue, mode: TeardownMode) {
        for slot in &mut self.registry.get_mut().slots {
            if let Some(callback) = slot.callback.take() {
                queue.release_value(callback, mode);
            }
        }
    }

    fn visit_built_in_children(&self, visitor: &mut TraceVisitor<'_>) {
        for slot in &self.registry.borrow().slots {
            if let Some(child) = slot.callback.as_ref().and_then(Value::collectable_box) {
                visitor.visit(child);
            }
        }
    }
}

#[whim_methods]
impl CallbackRegistry {
    #[whim_method("__construct(): void", no_track_caller, no_trace_boundary)]
    const fn construct() {}

    #[whim_method(
        "insert(fn(): void $callback): int",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn insert(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let callback = arguments.local(0);
        let identifier = context
            .state::<Self>()?
            .registry
            .borrow_mut()
            .insert(callback);
        Ok(Value::int(identifier))
    }

    #[whim_method("remove(int $id): void", no_track_caller, no_trace_boundary)]
    fn remove(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        context
            .state::<Self>()?
            .registry
            .borrow_mut()
            .remove(arguments.int(0));
        Ok(Value::null())
    }

    #[whim_method("isEmpty(): bool", no_track_caller, no_trace_boundary, must_use)]
    fn is_empty(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let empty = context.state::<Self>()?.registry.borrow().is_empty();
        Ok(Value::bool(empty))
    }

    #[whim_method(
        "drain(): vec<fn(): void>",
        no_track_caller,
        no_trace_boundary,
        must_use
    )]
    fn drain(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let callbacks = context.state::<Self>()?.registry.borrow_mut().drain();
        Ok(context.vec(callbacks))
    }
}
