use hashbrown::HashMap;
use whim_base::limits::MAX_TYPE_DEPTH_U32;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::render::type_descriptor;
use whim_bytecode::unit::CompiledWhereConstraint;
use whim_optimizer::descriptors_equal;
use whim_value::atom::Atom;

use crate::classes::MethodBodyKind;
use crate::classes::MethodEntry;
use crate::engine::Engine;
use crate::linker::descriptors::substitute_symbolic;
use crate::vm::VirtualMachineControl;

impl Engine {
    pub(in crate::linker) fn method_where_constraints(
        &self,
        method: &MethodEntry,
    ) -> &[CompiledWhereConstraint] {
        match method.body {
            MethodBodyKind::Bytecode(function) => {
                self.tables.functions[function.0 as usize].where_constraints()
            }
            MethodBodyKind::BuiltIn(_) => &[],
        }
    }

    pub(in crate::linker) fn override_where_reason(
        &mut self,
        replacement: &MethodEntry,
        replaced: &MethodEntry,
        replacement_environment: &HashMap<Atom, TypeDescriptor>,
        replaced_environment: &HashMap<Atom, TypeDescriptor>,
    ) -> Result<Option<String>, VirtualMachineControl> {
        let mut requirements = self.method_where_constraints(replacement).to_vec();
        for parameter in self.method_types(replacement).2 {
            for bound in parameter.bounds {
                requirements.push(CompiledWhereConstraint {
                    parameter: parameter.name.clone(),
                    bound,
                    span: parameter.span,
                });
            }
        }
        if requirements.is_empty() {
            return Ok(None);
        }
        let mut assumptions = self
            .method_where_constraints(replaced)
            .iter()
            .map(|constraint| {
                (
                    substitute_symbolic(
                        &TypeDescriptor::Parameter(constraint.parameter.clone()),
                        replaced_environment,
                    ),
                    substitute_symbolic(&constraint.bound, replaced_environment),
                )
            })
            .collect::<Vec<_>>();
        let (_, _, parameters) = self.method_types(replaced);
        for parameter in parameters {
            let argument = substitute_symbolic(
                &TypeDescriptor::Parameter(parameter.name),
                replaced_environment,
            );
            for bound in parameter.bounds {
                assumptions.push((
                    argument.clone(),
                    substitute_symbolic(&bound, replaced_environment),
                ));
            }
        }
        for constraint in requirements {
            let argument = substitute_symbolic(
                &TypeDescriptor::Parameter(constraint.parameter.clone()),
                replacement_environment,
            );
            let bound = substitute_symbolic(&constraint.bound, replacement_environment);
            let argument = refine_where_type(&argument, &assumptions, &mut Vec::new(), 0);
            let bound = refine_where_type(&bound, &assumptions, &mut Vec::new(), 0);
            if !self.link_descriptor_is_subtype(&argument, &bound)? {
                return Ok(Some(format!(
                    "constraint {}: {} is stronger than the inherited contract",
                    constraint.parameter,
                    type_descriptor(&constraint.bound, &|value| value.to_string()),
                )));
            }
        }
        Ok(None)
    }
}

fn refine_where_type(
    descriptor: &TypeDescriptor,
    assumptions: &[(TypeDescriptor, TypeDescriptor)],
    active: &mut Vec<usize>,
    depth: u32,
) -> TypeDescriptor {
    if depth > MAX_TYPE_DEPTH_U32 {
        return descriptor.clone();
    }
    let refined =
        descriptor.map_children(|child| refine_where_type(child, assumptions, active, depth + 1));
    let mut members = vec![refined];
    for (index, (argument, bound)) in assumptions.iter().enumerate() {
        if !active.contains(&index) && descriptors_equal(descriptor, argument, 0) {
            active.push(index);
            members.push(refine_where_type(bound, assumptions, active, depth + 1));
            active.pop();
        }
    }
    TypeDescriptor::intersection(members)
}
