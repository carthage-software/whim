//! The surface built-in handlers and the core library reach the engine
//! through.

use whim_value::Value;
use whim_value::atom::Atom;
use whim_value::heap::handle::ManagedRef;
use whim_value::object::ClassId;
use whim_value::object::InstanceObject;

use crate::core::symbols::strip_leading_backslash;
use crate::vm::Throw;
use crate::vm::VirtualMachine;

impl VirtualMachine<'_> {
    pub(crate) fn call_function_value(
        &mut self,
        callee: &Value,
        arguments: &[Value],
    ) -> Result<Value, Throw> {
        let outcome = self.call_callee_reentrant(callee, arguments);
        outcome.map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn call_method(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        name: Atom,
        arguments: &[Value],
    ) -> Result<Value, Throw> {
        self.invoke_method(receiver, name, arguments)
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn resolve_class(&mut self, name: Atom) -> Option<ClassId> {
        self.lookup_class_autoloading(strip_leading_backslash(&self.heap, name))
            .unwrap_or_default()
    }

    pub(crate) fn throw(&mut self, class_name: Atom, message: &str, code: i64) -> Throw {
        let class = self
            .resolve_class_symbol(&class_name)
            .unwrap_or(self.engine.tables.well_known.error);
        Throw(self.build_error(class, message.to_string(), code))
    }
}
