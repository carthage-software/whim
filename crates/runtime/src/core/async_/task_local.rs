//! Per-task values keyed by built-in task-local cells.

use foldhash::fast::FixedState;
use hashbrown::HashMap;
use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::value::Value;
use crate::vm::VirtualMachine;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TaskLocalId(u64);

pub(crate) type TaskLocalValues = HashMap<TaskLocalId, Value, FixedState>;

#[must_use]
pub(crate) const fn new_task_local_values() -> TaskLocalValues {
    HashMap::with_hasher(FixedState::with_seed(0))
}

#[whim_class("Whim\\Async\\TaskLocal<T: !null>", final)]
pub(crate) struct TaskLocal {
    id: TaskLocalId,
}

impl TaskLocal {
    pub(crate) fn new(vm: &mut VirtualMachine<'_>) -> Result<Self, Throw> {
        let id = vm.engine.next_task_local_id;
        let Some(next) = id.checked_add(1) else {
            let class = vm.engine.tables.well_known.overflow_error;
            return Err(vm.throw_well_known_value(
                class,
                "the task-local identity space is exhausted".to_string(),
            ));
        };

        vm.engine.next_task_local_id = next;
        Ok(Self {
            id: TaskLocalId(id),
        })
    }
}

#[whim_methods(generics = "<T: !null>")]
impl TaskLocal {
    #[whim_method("__construct(): void", no_track_caller)]
    const fn construct() {}

    #[whim_method("get(): null|T", no_track_caller, must_use)]
    fn get(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let id = context.state::<Self>()?.id;
        let value = if let Some(task) = context.vm.engine.coroutine_stack.last() {
            task.task_local_values.borrow().get(&id).cloned()
        } else {
            context.vm.engine.main_task_local_values.get(&id).cloned()
        };

        Ok(value.unwrap_or_else(Value::null))
    }

    #[whim_method("set(T $value): void", no_track_caller)]
    fn set(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let id = context.state::<Self>()?.id;
        let value = arguments.local(0);
        if let Some(task) = context.vm.engine.coroutine_stack.last() {
            task.task_local_values.borrow_mut().insert(id, value);
        } else {
            context.vm.engine.main_task_local_values.insert(id, value);
        }

        Ok(Value::null())
    }

    #[whim_method("clear(): void", no_track_caller)]
    fn clear(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let id = context.state::<Self>()?.id;
        if let Some(task) = context.vm.engine.coroutine_stack.last() {
            drop(task.task_local_values.borrow_mut().remove(&id));
        } else {
            drop(context.vm.engine.main_task_local_values.remove(&id));
        }

        Ok(Value::null())
    }
}

impl VirtualMachine<'_> {
    pub(crate) fn clear_main_task_local_values(&mut self) {
        self.engine.main_task_local_values.clear();
    }
}
