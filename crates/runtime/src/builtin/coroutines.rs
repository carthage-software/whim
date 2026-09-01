//! Context methods for the autoload callback and the event loop.

use std::os::fd::RawFd;

use whim_loop::Interest;

use crate::builtin::Context;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::linker::descriptors::descriptor_from_built_in_spec;
use crate::value::Value;
use crate::value::object::TypeEnvironmentId;

impl Context<'_, '_, '_> {
    pub(crate) fn new_instance(&mut self, name: &str) -> Result<Value, Throw> {
        let class = self.resolve_named_class(name)?;
        self.vm
            .new_instance_in_environment(class, TypeEnvironmentId::default())
            .map_err(|control| self.vm.control_to_throw(control))
    }

    pub(crate) fn new_built_in_instance(&mut self, name: &str) -> Result<Value, Throw> {
        let class = self.resolve_named_class(name)?;
        self.vm.build_built_in_instance(class)
    }

    /// Builds a typed instance of an internal built-in class without invoking
    /// its source-inaccessible constructor.
    pub(crate) fn new_built_in_instance_typed(
        &mut self,
        name: &str,
        type_arguments: &[TypeSpec],
    ) -> Result<Value, Throw> {
        let class = self.resolve_named_class(name)?;
        let supplied = type_arguments
            .iter()
            .map(|argument| descriptor_from_built_in_spec(self.vm.heap(), argument))
            .collect::<Vec<_>>();
        let environment = self.type_environment;
        self.vm
            .build_built_in_instance_typed(class, &supplied, environment)
    }

    pub(crate) fn io_wait_until_readable(&mut self, fd: RawFd) -> Result<(), Throw> {
        self.io_wait_until(fd, Interest::Readable)
    }

    pub(crate) fn io_wait_until_writable(&mut self, fd: RawFd) -> Result<(), Throw> {
        self.io_wait_until(fd, Interest::Writable)
    }

    /// Waits for `fd` to become ready for `interest`, whichever context the
    /// caller is in: inside a coroutine the current task parks on the descriptor
    /// and suspends; at `{main}` there is no current task to park, so a watcher is
    /// placed on the descriptor and the loop is driven until it fires.
    pub(crate) fn io_wait_until(&mut self, fd: RawFd, interest: Interest) -> Result<(), Throw> {
        if self.vm.loop_current_task().is_some() {
            return self.vm.loop_park_on_fd(fd, interest);
        }

        let watcher = self.closure(descriptor_ready_spec(), &[]);
        self.vm.loop_await_fd(watcher, fd, interest)
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "built-in closure handlers use a uniform fallible ABI"
)]
#[whim_macros::whim_closure("(): void")]
const fn descriptor_ready(_cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    Ok(Value::null())
}
