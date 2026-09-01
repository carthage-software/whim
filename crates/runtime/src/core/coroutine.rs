//! Coroutine state required by the Rust-backed core.

use std::cell::Cell;
use std::cell::RefCell;
use std::ptr::NonNull;

use whim_loop::Coroutine;
use whim_loop::Yielder;

use crate::core::async_::task_local::TaskLocalValues;
use crate::core::async_::task_local::new_task_local_values;
use crate::engine::Engine;
use crate::value::Value;

pub(crate) enum CoroutineInput {
    Start(Vec<Value>),
    Resume(Value),
    /// `throw($error)`: the suspension completes by throwing.
    Throw(Value),
}

pub(crate) enum CoroutineTermination {
    Returned,
    /// A throw unwound out of the callback.
    Thrown(Value),
    Exited(u8),
}

pub(crate) type CoroutineHandle =
    Coroutine<(NonNull<Engine>, CoroutineInput), Value, CoroutineTermination>;

pub(crate) type CoroutineYielder = Yielder<(NonNull<Engine>, CoroutineInput), Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoroutineState {
    Fresh,
    Running,
    Suspended,
    /// The callback returned or threw.
    Terminated,
}

pub(crate) struct CoroutineObject {
    /// The callback, taken when the coroutine starts.
    pub callback: RefCell<Option<Value>>,
    /// The coroutine, present from start until termination.
    pub coroutine: RefCell<Option<CoroutineHandle>>,
    pub state: Cell<CoroutineState>,
    /// The active suspension point, set while the coroutine runs.
    pub yielder: Cell<Option<NonNull<CoroutineYielder>>>,
    pub task_local_values: RefCell<TaskLocalValues>,
}

pub(crate) const COROUTINE_STACK_BYTES: usize = 1024 * 1024;

/// Limits retained stacks after a burst of tasks.
pub(crate) const COROUTINE_STACK_POOL_CAP: usize = 256;

impl CoroutineObject {
    pub(crate) const fn new(callback: Value) -> Self {
        Self {
            callback: RefCell::new(Some(callback)),
            coroutine: RefCell::new(None),
            state: Cell::new(CoroutineState::Fresh),
            yielder: Cell::new(None),
            task_local_values: RefCell::new(new_task_local_values()),
        }
    }
}

/// Resets suspended stacks without unwinding during teardown.
impl Drop for CoroutineObject {
    fn drop(&mut self) {
        if self.state.get() == CoroutineState::Suspended
            && let Some(handle) = self.coroutine.get_mut().as_mut()
        {
            // SAFETY: the handle is suspended and will never resume after drop.
            unsafe { handle.force_reset() };
        }
    }
}
