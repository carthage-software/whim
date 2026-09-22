use whim_base::limits::MAX_TYPE_DEPTH;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::unit::CompiledFunction;
use whim_bytecode::unit::CompiledTypeParameter;
use whim_value::atom::Atom;

use crate::type_flow::TypeFlow;
use crate::type_flow::descriptors::substitute_parameters;
use crate::type_flow::same_atom;

impl TypeFlow<'_> {
    pub(super) fn constrained_type(&self, descriptor: &TypeDescriptor) -> TypeDescriptor {
        let descriptor = self.expanded_aliases(descriptor);
        if self.bounded_function.is_none() {
            descriptor.into_owned()
        } else {
            self.refine_bound(&descriptor, &mut Vec::new(), 0)
        }
    }

    fn refine_bound(
        &self,
        descriptor: &TypeDescriptor,
        active: &mut Vec<Atom>,
        depth: usize,
    ) -> TypeDescriptor {
        let Some(function) = self.bounded_function else {
            return descriptor.clone();
        };

        if depth > MAX_TYPE_DEPTH {
            return descriptor.clone();
        }

        match descriptor {
            TypeDescriptor::Parameter(name) if !active.contains(name) => {
                active.push(name.clone());
                let mut members = vec![descriptor.clone()];
                let parameter = function
                    .type_parameters
                    .iter()
                    .find(|parameter| same_atom(&parameter.name, name))
                    .or_else(|| {
                        self.class_type_parameters
                            .iter()
                            .find(|parameter| same_atom(&parameter.name, name))
                    });

                for bound in parameter
                    .into_iter()
                    .flat_map(|parameter| &parameter.bounds)
                {
                    members.push(self.refine_bound(
                        &self.expanded_aliases(bound),
                        active,
                        depth + 1,
                    ));
                }

                for constraint in &function.where_constraints {
                    if same_atom(&constraint.parameter, name) {
                        members.push(self.refine_bound(
                            &self.expanded_aliases(&constraint.bound),
                            active,
                            depth + 1,
                        ));
                    }
                }

                active.pop();
                TypeDescriptor::intersection(members)
            }
            TypeDescriptor::Named { .. }
            | TypeDescriptor::Member { .. }
            | TypeDescriptor::Callable(_)
            | TypeDescriptor::Negated(_) => descriptor.clone(),
            _ => descriptor.map_children(|child| self.refine_bound(child, active, depth + 1)),
        }
    }

    pub(crate) fn where_constraints_proven(
        &self,
        function: &CompiledFunction,
        parameters: &[CompiledTypeParameter],
        arguments: Option<&[TypeDescriptor]>,
    ) -> bool {
        if !function.where_constraints.is_empty()
            && !parameters.is_empty()
            && arguments.is_none_or(|arguments| arguments.len() != parameters.len())
        {
            return false;
        }
        function.where_constraints.iter().all(|constraint| {
            let actual = substitute_parameters(
                &TypeDescriptor::Parameter(constraint.parameter.clone()),
                parameters,
                arguments,
                0,
            );

            let expected = substitute_parameters(&constraint.bound, parameters, arguments, 0);
            self.descriptor_proves(
                &self.constrained_type(&actual),
                &self.expanded_aliases(&expected),
                0,
            )
        })
    }

    pub(crate) fn method_where_constraints_proven(&self, index: usize) -> bool {
        let Some((receiver, method)) = self.resolved_method_receiver_at(index, 0) else {
            return false;
        };

        self.where_constraints_proven(
            &method.function,
            &receiver.class.type_parameters,
            receiver.arguments.as_deref(),
        )
    }
}
