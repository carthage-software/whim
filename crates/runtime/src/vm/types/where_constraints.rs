use std::rc::Rc;

use whim_bytecode::chunk::descriptors::TypeDescriptor;

use crate::symbols::FunctionLocator;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;

impl VirtualMachine<'_> {
    pub(in crate::vm) fn check_where_constraints(&mut self) -> Result<(), VirtualMachineControl> {
        let frame = self.current_frame();
        let Some(function) = frame.function.get() else {
            return Ok(());
        };
        let environment = frame.type_environment;
        let runtime = &self.engine.tables.functions[function.0 as usize];
        let FunctionLocator::Method { class, method } = runtime.locator else {
            return Ok(());
        };
        let unit = Rc::clone(&runtime.unit);
        let method = &unit.unit.classes[class as usize].methods[method as usize];
        for constraint in &method.where_constraints {
            let argument = self.substitute_descriptor(
                &TypeDescriptor::Parameter(constraint.parameter.clone()),
                environment,
                0,
            );
            let argument = self.resolve_static_argument(&argument);
            let bound = self.canonical_type_argument(&constraint.bound, environment)?;
            let bound = self.resolve_static_argument(&bound);
            if !self.descriptor_is_subtype(&argument, &bound, environment, 0)? {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "type argument {} for {} does not satisfy where constraint {}: {} of {}",
                        self.render_descriptor(&argument),
                        constraint.parameter,
                        constraint.parameter,
                        self.render_descriptor(&bound),
                        method.function.name,
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn resolve_static_argument(&self, descriptor: &TypeDescriptor) -> TypeDescriptor {
        if matches!(descriptor, TypeDescriptor::StaticClass)
            && let Some(called) = self
                .frames
                .last()
                .and_then(|frame| frame.called_class.get())
        {
            let class = &self.engine.tables.classes[called.0 as usize];
            let arguments = self.current_this().and_then(|receiver| {
                (!class.type_parameters.is_empty()).then(|| {
                    class
                        .type_parameters
                        .iter()
                        .map(|parameter| {
                            self.substitute_descriptor(
                                &TypeDescriptor::Parameter(parameter.name.clone()),
                                receiver.type_environment(),
                                0,
                            )
                        })
                        .collect()
                })
            });
            return TypeDescriptor::Named {
                name: class.name.clone(),
                arguments,
                recursive: false,
            };
        }
        descriptor.map_children(|child| self.resolve_static_argument(child))
    }
}
