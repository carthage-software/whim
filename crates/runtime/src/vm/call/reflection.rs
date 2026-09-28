use whim_bytecode::chunk::descriptors::CalleeDescriptor;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_value::Value;
use whim_value::atom::Atom;
use whim_value::function::CallTarget;
use whim_value::object::ClassId;
use whim_value::object::TypeEnvironmentId;

use crate::classes::MethodEntry;
use crate::classes::is_instance_of;
use crate::classes::visibility_allows;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;

impl VirtualMachine<'_> {
    pub(crate) fn invoke_reflected(
        &mut self,
        callee: &Value,
        descriptor: &CalleeDescriptor,
        type_arguments: &[TypeDescriptor],
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let shape = self.resolve_described_callee(callee, descriptor)?;
        let (environment, bound) = self.bind_callee_type_arguments(
            &shape,
            (!type_arguments.is_empty()).then_some(type_arguments),
            TypeEnvironmentId::default(),
        )?;

        let target = shape.target;
        let result = self.call_shape_reentrant(shape, arguments, environment, bound)?;
        if let CallTarget::User(function) = target {
            self.remember_frameless_discarded_result(function, self.caller_discards_result());
        }

        Ok(result)
    }

    pub(crate) fn reflected_method_target(
        &mut self,
        entry: MethodEntry,
        name: &Atom,
        target: &Value,
    ) -> Result<CalleeDescriptor, VirtualMachineControl> {
        if *name == self.engine.tables.constructor_name
            || *name == self.engine.tables.destructor_name
        {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("cannot invoke the lifecycle method {name} through reflection"),
            ));
        }

        let class = if entry.is_static {
            let Some(bytes) = target.as_string_bytes() else {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    "a static method requires a classname target".to_string(),
                ));
            };

            self.resolve_class_name(self.heap.intern(bytes.strip_prefix(b"\\").unwrap_or(bytes)))?
        } else {
            let Some(receiver) = target.as_object() else {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    "an instance method requires an object target".to_string(),
                ));
            };

            receiver.class()
        };

        if !is_instance_of(&self.engine.tables.classes, class, entry.declaring_class) {
            let declaring = &self.engine.tables.classes[entry.declaring_class.0 as usize].name;
            let target = &self.engine.tables.classes[class.0 as usize].name;
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("cannot invoke {declaring}::{name} on {target}: {target} is not a subtype of {declaring}"),
            ));
        }

        if !visibility_allows(
            &self.engine.tables.classes,
            entry.visibility,
            entry.declaring_class,
            self.current_frame().class_scope.get(),
        ) {
            let declaring = &self.engine.tables.classes[entry.declaring_class.0 as usize].name;
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot access method {declaring}::{name} from this scope"),
            ));
        }

        Ok(if entry.is_static {
            CalleeDescriptor::StaticMethod {
                class: None,
                name: name.clone(),
            }
        } else {
            CalleeDescriptor::Method(name.clone())
        })
    }

    pub(crate) fn instantiate_reflected(
        &mut self,
        class: ClassId,
        type_arguments: &[TypeDescriptor],
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let created = self.new_instance_typed(
            class,
            (!type_arguments.is_empty()).then_some(type_arguments),
            TypeEnvironmentId::default(),
        )?;

        let constructor = self.engine.tables.constructor_name.clone();
        if self.engine.tables.classes[class.0 as usize]
            .method(&constructor)
            .is_some()
        {
            let shape =
                self.resolve_described_callee(&created, &CalleeDescriptor::Method(constructor))?;
            let (environment, bound) = self.callee_type_environment(&shape)?;
            self.call_shape_reentrant(shape, arguments, environment, bound)?;
        } else if !arguments.is_empty() {
            let name = &self.engine.tables.classes[class.0 as usize].name;
            return Err(self.throw_well_known(
                self.engine.tables.well_known.argument_count_error,
                format!(
                    "{name} has no constructor, {} arguments given",
                    arguments.len()
                ),
            ));
        }

        Ok(created)
    }
}
