use std::rc::Rc;

use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::unit::CompiledTypeParameter;
use whim_value::object::ClassId;
use whim_value::object::TypeEnvironmentId;

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
        let unit = Rc::clone(&runtime.unit);
        let function = match runtime.locator {
            FunctionLocator::TopLevel(index) => &unit.unit.functions[index as usize],
            FunctionLocator::Method { class, method } => {
                &unit.unit.classes[class as usize].methods[method as usize].function
            }
        };
        for constraint in &function.where_constraints {
            if function
                .type_parameters
                .iter()
                .any(|parameter| parameter.name == constraint.parameter)
            {
                continue;
            }
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
                        function.name,
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn resolve_static_argument(&self, descriptor: &TypeDescriptor) -> TypeDescriptor {
        if !matches!(descriptor, TypeDescriptor::StaticClass) {
            return descriptor.map_children(|child| self.resolve_static_argument(child));
        }
        let Some(called) = self
            .frames
            .last()
            .and_then(|frame| frame.called_class.get())
        else {
            return descriptor.clone();
        };
        self.resolve_static_descriptor(
            descriptor,
            called,
            self.current_this()
                .map_or_else(TypeEnvironmentId::default, |receiver| {
                    receiver.type_environment()
                }),
        )
    }

    pub(in crate::vm) fn resolve_parameter_bounds(
        &self,
        parameters: &mut [CompiledTypeParameter],
        called: ClassId,
        environment: TypeEnvironmentId,
    ) {
        for parameter in parameters {
            for bound in &mut parameter.bounds {
                *bound = self.resolve_static_descriptor(bound, called, environment);
            }
            if let Some(default) = &mut parameter.default {
                *default = self.resolve_static_descriptor(default, called, environment);
            }
        }
    }

    fn resolve_static_descriptor(
        &self,
        descriptor: &TypeDescriptor,
        called: ClassId,
        environment: TypeEnvironmentId,
    ) -> TypeDescriptor {
        if matches!(descriptor, TypeDescriptor::StaticClass) {
            let class = &self.engine.tables.classes[called.0 as usize];
            let arguments = (!class.type_parameters.is_empty()).then(|| {
                class
                    .type_parameters
                    .iter()
                    .map(|parameter| {
                        self.substitute_descriptor(
                            &TypeDescriptor::Parameter(parameter.name.clone()),
                            environment,
                            0,
                        )
                    })
                    .collect()
            });
            return TypeDescriptor::Named {
                name: class.name.clone(),
                arguments,
                recursive: false,
            };
        }
        descriptor.map_children(|child| self.resolve_static_descriptor(child, called, environment))
    }
}
