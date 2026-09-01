//! Driving the event loop, and the concept-free primitives the async standard
//! library is built on.

use std::os::fd::RawFd;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use whim_loop::Activation;
use whim_loop::Interest;
use whim_loop::ReadyActivation;
use whim_loop::Scheduler;
use whim_loop::TaskId;

use crate::core::coroutine::CoroutineObject;
use crate::core::coroutine::CoroutineState;
use crate::vm::ClassId;
use crate::vm::InstanceObject;
use crate::vm::Throw;
use crate::vm::TypeDescriptor;
use crate::vm::TypeEnvironmentId;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::unreachable_invariant;

const MICROTASK_BATCH_SIZE: usize = 64;

/// Caps an unrepresentable deadline at one year.
fn deadline_after(duration: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(duration)
        .unwrap_or_else(|| now + Duration::from_hours(8_760))
}

impl VirtualMachine<'_> {
    pub(crate) fn drain_event_loop(&mut self) -> Result<Value, Throw> {
        self.ensure_scheduler()?;
        match self.run_event_loop() {
            Ok(()) => Ok(Value::null()),
            Err(control) => Err(self.control_to_throw(control)),
        }
    }

    fn ensure_scheduler(&mut self) -> Result<(), Throw> {
        if self.engine.scheduler.is_some() {
            return Ok(());
        }

        match Scheduler::new(Value::null()) {
            Ok(scheduler) => {
                self.engine.scheduler = Some(scheduler);
                Ok(())
            }
            Err(error) => Err(self.throw_well_known_value(
                self.engine.tables.well_known.coroutine_error,
                format!("could not start the event loop: {error}"),
            )),
        }
    }

    fn scheduler_mut(&mut self) -> &mut Scheduler<Rc<CoroutineObject>, Value> {
        match self.engine.scheduler.as_mut() {
            Some(scheduler) => scheduler,
            // SAFETY: the surrounding invariant makes this path unreachable.
            None => unsafe {
                unreachable_invariant("the event loop runs with a scheduler installed")
            },
        }
    }

    pub(crate) fn build_built_in_instance(&mut self, class: ClassId) -> Result<Value, Throw> {
        let declaration = &self.engine.tables.classes[class.0 as usize];
        if declaration.built_in_state_hooks.is_empty() {
            return Err(self.throw_well_known_value(
                self.engine.tables.well_known.type_error,
                "the class does not carry built-in state".to_string(),
            ));
        }
        let hooks = Rc::clone(&declaration.built_in_state_hooks);
        let state_initializers = Rc::clone(&declaration.built_in_state_initializers);

        let slot_count = declaration.slots.len();
        let heap = Rc::clone(&self.heap);
        let instance = InstanceObject::with_built_in_states(
            &heap,
            class,
            slot_count,
            &hooks,
            |index, destination| state_initializers[index](self, destination),
        )?;
        Ok(Value::object(instance))
    }

    pub(crate) fn build_built_in_instance_typed(
        &mut self,
        class: ClassId,
        supplied: &[TypeDescriptor],
        outer: TypeEnvironmentId,
    ) -> Result<Value, Throw> {
        if self.engine.tables.classes[class.0 as usize]
            .built_in_state_hooks
            .is_empty()
        {
            return Err(self.throw_well_known_value(
                self.engine.tables.well_known.type_error,
                "the class does not carry built-in state".to_string(),
            ));
        }

        let (parameters, name) = {
            let declaration = &self.engine.tables.classes[class.0 as usize];
            (
                Rc::clone(&declaration.type_parameters),
                declaration.name.clone(),
            )
        };
        let environment = self
            .bind_type_parameters(&parameters, Some(supplied), outer, name.as_bytes())
            .map_err(|control| self.control_to_throw(control))?;
        self.new_instance_in_environment(class, environment)
            .map_err(|control| self.control_to_throw(control))
    }

    fn build_coroutine(callback: &Value) -> Rc<CoroutineObject> {
        Rc::new(CoroutineObject::new(callback.clone()))
    }

    pub(crate) fn loop_current_task(&self) -> Option<TaskId> {
        self.engine
            .scheduler
            .as_ref()
            .and_then(Scheduler::current_task)
    }

    pub(crate) fn loop_has_tasks(&self) -> bool {
        self.engine
            .scheduler
            .as_ref()
            .is_some_and(Scheduler::has_tasks)
    }

    pub(crate) fn loop_suspend(&mut self) -> Result<Value, Throw> {
        self.suspend_current_coroutine(Value::null())
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn loop_resume(&mut self, task: TaskId, value: Value) {
        if let Some(scheduler) = self.engine.scheduler.as_mut() {
            scheduler.wake(task, value);
        }
    }

    pub(crate) fn loop_resume_front(&mut self, task: TaskId, value: Value) {
        if let Some(scheduler) = self.engine.scheduler.as_mut() {
            scheduler.wake_front(task, value);
        }
    }

    pub(crate) fn loop_throw(&mut self, task: TaskId, error: Value) {
        if let Some(scheduler) = self.engine.scheduler.as_mut() {
            scheduler.wake_throw(task, error);
        }
    }

    pub(crate) fn loop_defer(&mut self, callback: Value) -> Result<TaskId, Throw> {
        self.ensure_scheduler()?;
        let coroutine = Self::build_coroutine(&callback);
        Ok(self.scheduler_mut().spawn(coroutine, Vec::new()))
    }

    pub(crate) fn loop_queue(&mut self, callback: Value) -> Result<TaskId, Throw> {
        self.ensure_scheduler()?;
        let coroutine = Self::build_coroutine(&callback);
        Ok(self.scheduler_mut().queue(coroutine, Vec::new()))
    }

    pub(crate) fn loop_delay(
        &mut self,
        callback: Value,
        duration: Duration,
    ) -> Result<TaskId, Throw> {
        self.ensure_scheduler()?;
        let coroutine = Self::build_coroutine(&callback);
        let id = self.scheduler_mut().spawn_armed(coroutine, Vec::new());
        let deadline = deadline_after(duration);
        self.scheduler_mut().arm_timer(id, deadline);
        Ok(id)
    }

    pub(crate) fn loop_on_fd(
        &mut self,
        callback: Value,
        fd: RawFd,
        interest: Interest,
    ) -> Result<TaskId, Throw> {
        self.ensure_scheduler()?;
        let coroutine = Self::build_coroutine(&callback);
        let id = self.scheduler_mut().spawn_armed(coroutine, Vec::new());
        // SAFETY: the synchronous wait retains the descriptor value until this watcher finishes.
        let armed = unsafe { self.scheduler_mut().arm_descriptor(id, fd, interest) };
        if let Err(error) = armed {
            self.scheduler_mut().cancel(id);
            return Err(self.throw_well_known_value(
                self.engine.tables.well_known.coroutine_error,
                format!("could not watch the descriptor: {error}"),
            ));
        }

        Ok(id)
    }

    pub(crate) fn loop_record_error(&mut self, error: Value) -> Result<u64, Throw> {
        self.ensure_scheduler()?;
        Ok(self.scheduler_mut().record_error(error))
    }

    pub(crate) fn loop_forget_error(&mut self, id: u64) {
        if let Some(scheduler) = self.engine.scheduler.as_mut() {
            scheduler.forget_error(id);
        }
    }

    pub(crate) fn loop_cancel(&mut self, id: TaskId) {
        let Some(scheduler) = self.engine.scheduler.as_mut() else {
            return;
        };

        scheduler.disarm(id);
        let Some(coroutine) = scheduler.task_handle(id) else {
            return;
        };

        match coroutine.state.get() {
            CoroutineState::Fresh | CoroutineState::Terminated => {
                self.scheduler_mut().cancel(id);
            }
            CoroutineState::Suspended => {
                let error = self.cancellation_error();
                self.engine.cancelled_tasks.insert(id);
                self.scheduler_mut().wake_throw(id, error);
            }
            CoroutineState::Running => {
                self.engine.cancelled_tasks.insert(id);
            }
        }
    }

    fn cancellation_error(&mut self) -> Value {
        self.throw_well_known_value(
            self.engine.tables.well_known.coroutine_error,
            "the task was cancelled".to_string(),
        )
        .0
    }

    pub(crate) fn loop_unreference(&mut self, id: TaskId) {
        if let Some(scheduler) = self.engine.scheduler.as_mut() {
            scheduler.unreference(id);
        }
    }

    pub(crate) fn loop_park_current_for(&mut self, duration: Duration) {
        let deadline = deadline_after(duration);
        self.scheduler_mut().park_current_on_timer(deadline);
    }

    pub(crate) fn loop_run_once(&mut self) -> Result<bool, Throw> {
        self.ensure_scheduler()?;
        match self.pump_event_loop() {
            Ok(progressed) => Ok(progressed),
            Err(control) => Err(self.control_to_throw(control)),
        }
    }

    pub(crate) fn loop_watcher_pending(&self, id: TaskId) -> bool {
        self.engine
            .scheduler
            .as_ref()
            .is_some_and(|scheduler| scheduler.has_task(id))
    }

    pub(crate) fn loop_await_fd(
        &mut self,
        callback: Value,
        fd: RawFd,
        interest: Interest,
    ) -> Result<(), Throw> {
        let watcher = self.loop_on_fd(callback, fd, interest)?;
        while self.loop_watcher_pending(watcher) {
            if !self.loop_run_once()? {
                self.loop_cancel(watcher);
                return Err(self.deadlock_error());
            }
        }

        Ok(())
    }

    fn run_event_loop(&mut self) -> Result<(), VirtualMachineControl> {
        while self.pump_event_loop()? {}
        if let Some(error) = self.scheduler_mut().take_pending_error() {
            return Err(VirtualMachineControl::Throw(error));
        }

        Ok(())
    }

    fn pump_event_loop(&mut self) -> Result<bool, VirtualMachineControl> {
        let mut ran = false;
        let mut ready_count = self.scheduler_mut().ready_count();
        loop {
            let microtask_count = self
                .scheduler_mut()
                .microtask_count()
                .min(MICROTASK_BATCH_SIZE);

            for _ in 0..microtask_count {
                let Some(ready) = self.scheduler_mut().next_microtask_activation() else {
                    break;
                };

                ran = true;
                self.activate_task(ready)?;
            }

            if self.scheduler_mut().microtask_count() != 0 {
                if let Err(error) = self.scheduler_mut().poll_reactor_nonblocking() {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.coroutine_error,
                        format!("the event loop's reactor failed: {error}"),
                    ));
                }

                if let Some(ready) = self.scheduler_mut().next_ready_activation() {
                    self.activate_task(ready)?;
                }

                return Ok(true);
            }

            if ready_count == 0 {
                break;
            }

            ready_count -= 1;
            let Some(ready) = self.scheduler_mut().next_ready_activation() else {
                continue;
            };

            ran = true;
            self.activate_task(ready)?;
        }

        if ran {
            if let Err(error) = self.scheduler_mut().poll_reactor_nonblocking() {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.coroutine_error,
                    format!("the event loop's reactor failed: {error}"),
                ));
            }

            return Ok(true);
        }

        if self.scheduler_mut().is_idle() {
            return Ok(false);
        }

        if !self.scheduler_mut().has_wake_source() {
            return Ok(false);
        }

        if let Err(error) = self.scheduler_mut().poll_reactor() {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.coroutine_error,
                format!("the event loop's reactor failed: {error}"),
            ));
        }

        Ok(true)
    }

    fn activate_task(
        &mut self,
        ready: ReadyActivation<Rc<CoroutineObject>, Value>,
    ) -> Result<(), VirtualMachineControl> {
        let coroutine = ready.coroutine;
        let outcome = match ready.activation {
            Activation::Start(arguments) => self.coroutine_start(&coroutine, &arguments),
            Activation::Resume(value) => self.coroutine_resume(&coroutine, value),
            Activation::Throw(error) => self.coroutine_throw(&coroutine, error),
        };

        self.scheduler_mut().clear_current();
        let finalizer = self.engine.finalizer_tasks.contains(&ready.id);
        match outcome {
            Ok(_) => {
                if Self::coroutine_terminated(&coroutine) {
                    self.scheduler_mut().finish(ready.id);
                    self.engine.cancelled_tasks.remove(&ready.id);
                    if finalizer {
                        self.engine.finalizer_tasks.remove(&ready.id);
                    }
                } else if self.engine.cancelled_tasks.contains(&ready.id) {
                    let error = self.cancellation_error();
                    self.scheduler_mut().wake_throw(ready.id, error);
                }
            }
            Err(throw) => {
                if let Some(code) = self.pending_exit.take() {
                    self.scheduler_mut().finish(ready.id);
                    self.engine.finalizer_tasks.remove(&ready.id);
                    self.engine.cancelled_tasks.remove(&ready.id);
                    return Err(VirtualMachineControl::Exit(code));
                }

                self.scheduler_mut().finish(ready.id);
                if finalizer {
                    self.engine.finalizer_tasks.remove(&ready.id);
                }

                if !self.engine.cancelled_tasks.remove(&ready.id) {
                    return Err(VirtualMachineControl::Throw(throw.0));
                }
            }
        }

        Ok(())
    }

    pub(in crate::vm) fn run_finalizer_turn(&mut self) -> Result<(), VirtualMachineControl> {
        self.pump_event_loop().map(|_| ())
    }

    fn deadlock_error(&mut self) -> Throw {
        self.throw_well_known_value(
            self.engine.tables.well_known.coroutine_error,
            "the event loop drained without completing the awaited operation".to_string(),
        )
    }

    fn coroutine_terminated(coroutine: &Rc<CoroutineObject>) -> bool {
        coroutine.state.get() == CoroutineState::Terminated
    }

    pub(crate) fn loop_park_on_fd(&mut self, fd: RawFd, interest: Interest) -> Result<(), Throw> {
        self.loop_arm_fd(fd, interest)?;
        self.loop_suspend()?;
        Ok(())
    }

    pub(crate) fn loop_arm_fd(&mut self, fd: RawFd, interest: Interest) -> Result<(), Throw> {
        if self.loop_current_task().is_none() {
            return Err(self.throw_well_known_value(
                self.engine.tables.well_known.coroutine_error,
                "cannot wait on a descriptor outside the event loop".to_string(),
            ));
        }

        // SAFETY: the suspended descriptor operation retains its descriptor until it resumes.
        let armed = unsafe {
            self.scheduler_mut()
                .park_current_on_descriptor(fd, interest)
        };
        if let Err(error) = armed {
            return Err(self.throw_well_known_value(
                self.engine.tables.well_known.coroutine_error,
                format!("could not watch the descriptor: {error}"),
            ));
        }

        Ok(())
    }
}
