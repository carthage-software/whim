//! Activating, suspending, and resuming coroutines.

use std::process;
use std::ptr;
use std::rc::Rc;

use whim_loop::Coroutine;
use whim_loop::Resumption;
use whim_loop::Stack;

use crate::core::coroutine::COROUTINE_STACK_BYTES;
use crate::core::coroutine::COROUTINE_STACK_POOL_CAP;
use crate::core::coroutine::CoroutineHandle;
use crate::core::coroutine::CoroutineInput;
use crate::core::coroutine::CoroutineObject;
use crate::core::coroutine::CoroutineState;
use crate::core::coroutine::CoroutineTermination;
use crate::vm::NonNull;
use crate::vm::Throw;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::unreachable_invariant;

impl VirtualMachine<'_> {
    pub(crate) fn pooled_coroutine_stack(&mut self) -> Stack {
        self.engine.coroutine_stack_pool.pop().unwrap_or_else(|| {
            Stack::new(COROUTINE_STACK_BYTES).unwrap_or_else(|_| process::abort())
        })
    }

    /// Calls a coroutine's callback re-entrantly, for the coroutine body.
    /// Activates a coroutine: resumes its coroutine with the input and maps the
    /// outcome. A suspension completes the activation with the suspended
    /// value; a return terminates the coroutine and yields `null`; a throw
    /// unwinds out of this activation; an exit propagates.
    pub(crate) fn activate_coroutine(
        &mut self,
        coroutine: &Rc<CoroutineObject>,
        input: CoroutineInput,
    ) -> Result<Value, VirtualMachineControl> {
        coroutine.state.set(CoroutineState::Running);
        self.engine.coroutine_stack.push(Rc::clone(coroutine));

        let engine_pointer = NonNull::from(&mut *self.engine);
        self.heap.enter_coroutine();
        let outcome = {
            let mut handle = coroutine.coroutine.borrow_mut();
            let Some(active) = handle.as_mut() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("an activated coroutine holds its coroutine") }
            };
            active.resume((engine_pointer, input))
        };

        self.heap.leave_coroutine();
        self.engine.coroutine_stack.pop();
        match outcome {
            Resumption::Suspended(value) => {
                coroutine.state.set(CoroutineState::Suspended);
                Ok(value)
            }
            Resumption::Finished(termination) => {
                coroutine.state.set(CoroutineState::Terminated);
                if let Some(coroutine) = coroutine.coroutine.borrow_mut().take()
                    && self.engine.coroutine_stack_pool.len() < COROUTINE_STACK_POOL_CAP
                {
                    self.engine
                        .coroutine_stack_pool
                        .push(coroutine.into_stack());
                }
                match termination {
                    CoroutineTermination::Returned => Ok(Value::null()),
                    CoroutineTermination::Thrown(value) => Err(VirtualMachineControl::Throw(value)),
                    CoroutineTermination::Exited(code) => Err(VirtualMachineControl::Exit(code)),
                }
            }
        }
    }

    pub(crate) fn suspend_current_coroutine(
        &mut self,
        value: Value,
    ) -> Result<Value, VirtualMachineControl> {
        let Some(current) = self.engine.coroutine_stack.last() else {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.coroutine_error,
                "cannot suspend outside a coroutine".to_string(),
            ));
        };

        let current = Rc::clone(current);
        let yielder = {
            let Some(yielder) = current.yielder.get() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a running coroutine has its suspension point") }
            };

            yielder
        };

        drop(current);

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let (engine_pointer, input) = unsafe { yielder.as_ref() }.suspend(value);
        debug_assert!(
            ptr::eq(engine_pointer.as_ptr(), ptr::from_mut(self.engine)),
            "a suspended coroutine assumes the engine's address is stable across its lifetime"
        );

        match input {
            CoroutineInput::Resume(value) => Ok(value),
            CoroutineInput::Throw(error) => Err(VirtualMachineControl::Throw(error)),
            // SAFETY: the surrounding invariant makes this path unreachable.
            CoroutineInput::Start(_) => unsafe {
                unreachable_invariant("only the first activation starts")
            },
        }
    }
}

impl VirtualMachine<'_> {
    /// Starts a coroutine: builds its coroutine around the stored callback and
    /// runs it until it suspends or terminates, returning the suspended or
    /// `null` value. The `arguments` are passed to the callback. **Re-enters
    /// the interpreter** (the coroutine's body runs).
    pub(crate) fn coroutine_start(
        &mut self,
        receiver: &Rc<CoroutineObject>,
        arguments: &[Value],
    ) -> Result<Value, Throw> {
        if receiver.state.get() != CoroutineState::Fresh {
            return Err(self.throw_coroutine_error("cannot start a coroutine twice"));
        }

        let Some(callback) = receiver.callback.borrow_mut().take() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a fresh coroutine holds its callback") }
        };

        let stack = self.pooled_coroutine_stack();

        let coroutine_pointer = NonNull::from(&**receiver);
        let coroutine: CoroutineHandle =
            Coroutine::with_stack(stack, move |yielder, (engine, input)| {
                let CoroutineInput::Start(arguments) = input else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("the first activation is a start") }
                };
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                let coroutine = unsafe { coroutine_pointer.as_ref() };
                coroutine.yielder.set(Some(NonNull::from(yielder)));
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                let engine = unsafe { &mut *engine.as_ptr() };
                let mut coroutine_vm = VirtualMachine::new(engine);
                let outcome = coroutine_vm.call_callee_reentrant(&callback, &arguments);
                coroutine.yielder.set(None);
                drop(callback);
                drop(coroutine_vm);
                match outcome {
                    Ok(_) => CoroutineTermination::Returned,
                    Err(VirtualMachineControl::Throw(value)) => CoroutineTermination::Thrown(value),
                    Err(VirtualMachineControl::Exit(code)) => CoroutineTermination::Exited(code),
                }
            });

        *receiver.coroutine.borrow_mut() = Some(coroutine);
        self.activate_coroutine(receiver, CoroutineInput::Start(arguments.to_vec()))
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn coroutine_resume(
        &mut self,
        receiver: &Rc<CoroutineObject>,
        value: Value,
    ) -> Result<Value, Throw> {
        if receiver.state.get() != CoroutineState::Suspended {
            return Err(
                self.throw_coroutine_error("cannot resume a coroutine that is not suspended")
            );
        }

        self.activate_coroutine(receiver, CoroutineInput::Resume(value))
            .map_err(|control| self.control_to_throw(control))
    }

    /// Throws into a suspended coroutine at its suspension point. **Re-enters the
    /// interpreter.**
    pub(crate) fn coroutine_throw(
        &mut self,
        receiver: &Rc<CoroutineObject>,
        error: Value,
    ) -> Result<Value, Throw> {
        let valid = error
            .as_object()
            .is_some_and(|instance| self.engine.is_throwable_instance(instance.class()));
        if !valid {
            let found = error.kind_name();
            return Err(self.throw_well_known_value(
                self.engine.tables.well_known.type_error,
                format!("argument 1 ($error) must be Whim\\Unwind\\Throwable, {found} given"),
            ));
        }

        if receiver.state.get() != CoroutineState::Suspended {
            return Err(
                self.throw_coroutine_error("cannot throw into a coroutine that is not suspended")
            );
        }

        self.activate_coroutine(receiver, CoroutineInput::Throw(error))
            .map_err(|control| self.control_to_throw(control))
    }

    /// Builds a `CoroutineError` as a handler [`Throw`].
    fn throw_coroutine_error(&mut self, message: &str) -> Throw {
        self.throw_well_known_value(
            self.engine.tables.well_known.coroutine_error,
            message.to_string(),
        )
    }
}
