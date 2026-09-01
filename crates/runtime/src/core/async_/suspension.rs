//! Generic suspension objects used by the Whim-written async library.

use std::cell::Cell;
use std::cell::RefCell;

use whim_loop::TaskId;
use whim_macros::whim_class;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::BuiltInChildren;
use crate::builtin::convert::state_ref;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::value::Value;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::queue::DropQueue;
use crate::vm::VirtualMachine;

enum Outcome {
    Resume(Value),
    Throw(Value),
}

#[whim_class("Whim\\_Private\\Suspension<T>", final, traced)]
pub(crate) struct Suspension {
    task: Option<TaskId>,
    resolved: Cell<bool>,
    suspended: Cell<bool>,
    outcome: RefCell<Option<Outcome>>,
}

impl Suspension {
    #[expect(
        clippy::needless_pass_by_ref_mut,
        clippy::unnecessary_wraps,
        reason = "built-in state initializers use one fallible mutable-VM ABI"
    )]
    pub(crate) fn new(vm: &mut VirtualMachine<'_>) -> Result<Self, Throw> {
        Ok(Self {
            task: vm.loop_current_task(),
            resolved: Cell::new(false),
            suspended: Cell::new(false),
            outcome: RefCell::new(None),
        })
    }
}

// SAFETY: only a stored outcome may own a value, and teardown takes it once.
unsafe impl BuiltInChildren for Suspension {
    fn enqueue_built_in_children(&mut self, queue: &mut DropQueue, mode: TeardownMode) {
        if let Some(Outcome::Resume(value) | Outcome::Throw(value)) = self.outcome.get_mut().take()
        {
            queue.release_value(value, mode);
        }
    }

    fn visit_built_in_children(&self, visitor: &mut TraceVisitor<'_>) {
        let outcome = self.outcome.borrow();
        let Some(Outcome::Resume(value) | Outcome::Throw(value)) = outcome.as_ref() else {
            return;
        };
        if let Some(child) = value.collectable_box() {
            visitor.visit(child);
        }
    }
}

fn state_of<'a>(cx: &mut Context<'_, '_, '_>, value: &'a Value) -> Result<&'a Suspension, Throw> {
    state_ref::<Suspension>(value).ok_or_else(|| cx.type_error("the value is not a suspension"))
}

fn consume_outcome(outcome: Outcome) -> Result<Value, Throw> {
    match outcome {
        Outcome::Resume(value) => Ok(value),
        Outcome::Throw(error) => Err(Throw(error)),
    }
}

fn already_resolved(cx: &mut Context<'_, '_, '_>) -> Throw {
    let class = cx.vm.intern(b"Whim\\Unwind\\RuntimeException");
    cx.vm
        .throw(class, "the suspension has already been resolved", 0)
}

#[whim_methods(generics = "<T>")]
impl Suspension {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("suspend(): T")]
    fn suspend(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        if let Some(outcome) = state_of(cx, &receiver)?.outcome.borrow_mut().take() {
            return consume_outcome(outcome);
        }

        let state = state_of(cx, &receiver)?;
        if state.resolved.get() || state.suspended.replace(true) {
            return Err(already_resolved(cx));
        }
        let task = state.task;

        let outcome = match task {
            Some(_) => cx.vm.loop_suspend(),
            None => loop {
                if let Some(outcome) = state_of(cx, &receiver)?.outcome.borrow_mut().take() {
                    break consume_outcome(outcome);
                }
                if !cx.vm.loop_run_once()? {
                    let class = cx.vm.intern(b"Whim\\Unwind\\RuntimeException");
                    break Err(cx.vm.throw(
                        class,
                        "the event loop drained without completing the awaited operation",
                        0,
                    ));
                }
            },
        };

        state_of(cx, &receiver)?.suspended.set(false);
        outcome
    }

    #[whim_method("resume(T $value): void")]
    fn resume(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        let value = arguments.local(0);
        let state = state_of(cx, &receiver)?;
        if state.resolved.replace(true) {
            return Err(already_resolved(cx));
        }

        if state.suspended.get()
            && let Some(task) = state.task
        {
            cx.vm.loop_resume(task, value);
        } else {
            *state.outcome.borrow_mut() = Some(Outcome::Resume(value));
        }
        Ok(Value::null())
    }

    #[whim_method("resumeEagerly(T $value): void")]
    fn resume_eagerly(
        cx: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        let value = arguments.local(0);
        let state = state_of(cx, &receiver)?;
        if state.resolved.replace(true) {
            return Err(already_resolved(cx));
        }

        if state.suspended.get()
            && let Some(task) = state.task
        {
            cx.vm.loop_resume_front(task, value);
        } else {
            *state.outcome.borrow_mut() = Some(Outcome::Resume(value));
        }
        Ok(Value::null())
    }

    #[whim_method("throw(Whim\\Unwind\\Throwable $error): void")]
    fn throw_error(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let receiver = cx.receiver();
        let error = arguments.local(0);
        let state = state_of(cx, &receiver)?;
        if state.resolved.replace(true) {
            return Err(already_resolved(cx));
        }

        if state.suspended.get()
            && let Some(task) = state.task
        {
            cx.vm.loop_throw(task, error);
        } else {
            *state.outcome.borrow_mut() = Some(Outcome::Throw(error));
        }
        Ok(Value::null())
    }
}

#[whim_function("Whim\\_Private\\create_suspension<T>(): Whim\\_Private\\Suspension<T>")]
fn create_suspension(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    cx.new_built_in_instance_typed("Whim\\_Private\\Suspension", &[TypeSpec::Parameter("T")])
}
