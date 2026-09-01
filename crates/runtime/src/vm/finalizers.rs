//! Running object destructors at VM-safe boundaries.

use std::iter;

use whim_macros::whim_closure;

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::classes::MethodBodyKind;
use crate::engine::builtins::BuiltInCallable;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::function::CallTarget;
use crate::value::function::FunctionObject;
use crate::value::heap::FinalizerOrigin;
use crate::value::heap::PendingFinalizer;
use crate::value::heap::handle::ManagedRef;
use crate::value::object::InstanceObject;
use crate::value::object::TypeEnvironmentId;
use crate::vm::MethodContext;
use crate::vm::UserCallContext;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;

/// Callback for a cycle destructor run in its own coroutine.
#[whim_closure("(): void")]
fn run_cycle_destructor(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let object = context.capture(0);
    let name = context.vm.intern(b"__destruct");
    context.call_method(&object, name, &[])?;
    Ok(Value::null())
}

impl VirtualMachine<'_> {
    fn invoke_destructor(
        &mut self,
        object: &ManagedRef<InstanceObject>,
    ) -> Result<(), VirtualMachineControl> {
        // SAFETY: the surrounding invariant proves this option contains a value.
        let entry = unsafe {
            unwrap_option_invariant(
                self.engine.tables.classes[object.class().0 as usize].destructor,
                "a finalizable object has a concrete destructor",
            )
        };

        let type_environment = self
            .environment_for_class(
                object.class(),
                object.type_environment(),
                entry.declaring_class,
                0,
            )?
            .unwrap_or_else(TypeEnvironmentId::default);

        match entry.body {
            MethodBodyKind::Bytecode(function) => {
                self.call_user_with_context(UserCallContext {
                    function,
                    this: Some(object.clone()),
                    captures: &[],
                    arguments: &[],
                    method: Some(MethodContext {
                        scope: entry.declaring_class,
                        called: object.class(),
                        is_constructor: false,
                    }),
                    declared_scope: None,
                    type_environment,
                    type_arguments_bound: false,
                })?;
            }
            MethodBodyKind::BuiltIn(body) => {
                let name = self.engine.tables.destructor_name.clone();
                self.invoke_built_in_callable_called(
                    BuiltInCallable::Method { body, name },
                    Some(object),
                    &[],
                    Some(object.class()),
                    type_environment,
                    false,
                    &[],
                )?;
            }
        }

        Ok(())
    }

    /// Defers one cycle destructor as its own coroutine, so a suspended
    /// destructor does not block the others.
    fn defer_cycle_destructor(
        &mut self,
        object: ManagedRef<InstanceObject>,
    ) -> Result<(), VirtualMachineControl> {
        let spec = run_cycle_destructor_spec();
        let signature = self.heap.intern(spec.signature.as_bytes());
        let target = self.engine.intern_built_in_function(spec);

        let callback = FunctionObject::partial(
            &self.heap,
            CallTarget::BuiltIn(target),
            None,
            [Value::object(object)],
            iter::empty(),
            signature,
            None,
            None,
            TypeEnvironmentId::default(),
            false,
        );

        let task = self
            .loop_defer(Value::function(callback))
            .map_err(|Throw(value)| VirtualMachineControl::Throw(value))?;

        self.engine.finalizer_tasks.insert(task);
        Ok(())
    }

    pub(crate) fn drain_finalizers(
        &mut self,
        terminating: bool,
    ) -> Result<(), VirtualMachineControl> {
        if self.draining_finalizers {
            return Ok(());
        }

        self.draining_finalizers = true;
        let mut deferred = false;

        loop {
            let pending = self.heap.take_pending_finalizers();
            if pending.is_empty() {
                break;
            }

            let mut pending = pending.into_iter();
            while let Some(finalizer) = pending.next() {
                let PendingFinalizer { object, origin } = finalizer;
                if origin == FinalizerOrigin::CycleInCoroutine {
                    if let Err(control) = self.defer_cycle_destructor(object) {
                        self.draining_finalizers = false;
                        self.heap.return_pending_finalizers(pending);
                        return Err(control);
                    }

                    deferred = true;
                    continue;
                }

                let outcome = self.invoke_destructor(&object);
                drop(object);
                if let Err(control) = outcome {
                    self.draining_finalizers = false;
                    match control {
                        VirtualMachineControl::Exit(code) => {
                            self.heap.abandon_finalizers();
                            drop(pending);
                            drop(self.heap.take_pending_finalizers());
                            return Err(VirtualMachineControl::Exit(code));
                        }
                        VirtualMachineControl::Throw(value) if terminating => {
                            self.heap.abandon_finalizers();
                            drop(pending);
                            drop(self.heap.take_pending_finalizers());
                            return Err(VirtualMachineControl::Throw(value));
                        }
                        VirtualMachineControl::Throw(value) => {
                            self.heap.return_pending_finalizers(pending);
                            return Err(VirtualMachineControl::Throw(value));
                        }
                    }
                }
            }
        }

        if deferred {
            self.draining_finalizers = false;
            if let Some(current) = self.loop_current_task() {
                self.loop_resume(current, Value::null());
                self.loop_suspend()
                    .map_err(|Throw(value)| VirtualMachineControl::Throw(value))?;
            } else {
                self.run_finalizer_turn()?;
            }
        }

        self.draining_finalizers = false;
        Ok(())
    }

    /// Releases the stack, schedules every live finalizable object, and runs
    /// shutdown destructors before class metadata is torn down.
    pub(crate) fn run_shutdown_finalizers(&mut self) -> Result<(), VirtualMachineControl> {
        self.frames.clear();
        self.stack.clear();
        self.stack_initialized_len = 0;
        self.pending_unwinds.clear();

        loop {
            self.heap.schedule_shutdown_finalizers();
            if !self.heap.has_pending_finalizers() {
                return Ok(());
            }
            self.drain_finalizers(true)?;
        }
    }
}
