//! The declaration contracts linking enforces: override compatibility,
//! interface conformance, abstract completeness, and base type-argument
//! arity.

use std::rc::Rc;

use hashbrown::HashMap;
use whim_span::Span;

use crate::bytecode::chunk::descriptors::FunctionTypeDescriptor;
use crate::bytecode::chunk::descriptors::FunctionTypeParameterDescriptor;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledBaseReference;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::Variance;
use crate::bytecode::unit::Visibility;
use crate::classes::MethodBodyKind;
use crate::classes::MethodEntry;
use crate::classes::PropertyInfo;
use crate::classes::RuntimeClass;
use crate::engine::Engine;
use crate::engine::diagnostics::DiagnosticLabel;
use crate::limits::MAX_TYPE_DEPTH_U32;
use crate::linker::InterfaceRequirements;
use crate::linker::OverrideCheck;
use crate::linker::Replaced;
use crate::linker::descriptors::descriptor_from_built_in_spec;
use crate::linker::descriptors::substitute_symbolic;
use crate::linker::visibility_name;
use crate::linker::visibility_rank;
use crate::optimizer::descriptors_equal;
use crate::symbols::FunctionTable;
use crate::symbols::SymbolKind;
use crate::unwrap_option_invariant;
use crate::value::atom::Atom;
use crate::value::object::ClassId;
use crate::value::object::TypeEnvironmentId;
use crate::variance::incompatible_parameter;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;

struct DeclaredContract<'a> {
    class: &'a RuntimeClass,
    current_id: ClassId,
    kind: ClassLikeKind,
    name: &'a str,
    path: &'a Atom,
}

impl Engine {
    pub(in crate::linker) fn check_consistent_generic_bounds(
        &mut self,
        current: &CompiledClassLike,
        base: &CompiledBaseReference,
        base_id: ClassId,
        name_text: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        let child_environment = current
            .type_parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    TypeDescriptor::Parameter(parameter.name.clone()),
                )
            })
            .collect::<HashMap<_, _>>();

        let (parent_parameters, parent_name) = {
            let parent = &self.tables.classes[base_id.0 as usize];
            (Rc::clone(&parent.type_parameters), parent.name.clone())
        };
        let mut parent_environment = HashMap::new();
        for (position, parameter) in parent_parameters.iter().enumerate() {
            let argument = if let Some(argument) = base
                .type_arguments
                .as_ref()
                .and_then(|arguments| arguments.get(position))
            {
                substitute_symbolic(argument, &child_environment)
            } else {
                let Some(default) = parameter.default.as_ref() else {
                    continue;
                };
                substitute_symbolic(default, &parent_environment)
            };
            parent_environment.insert(parameter.name.clone(), argument);
        }

        for (position, parameter) in parent_parameters.iter().enumerate() {
            let Some(argument) = parent_environment.get(&parameter.name) else {
                continue;
            };
            let TypeDescriptor::Parameter(child_name) = argument else {
                continue;
            };
            let Some(child) = current
                .type_parameters
                .iter()
                .find(|candidate| candidate.name == *child_name)
            else {
                continue;
            };

            let same_variance = child.variance == parameter.variance;
            let mut same_bounds = child.bounds.len() == parameter.bounds.len();
            if same_bounds {
                for (child_bound, parent_bound) in child.bounds.iter().zip(&parameter.bounds) {
                    let child_bound = substitute_symbolic(child_bound, &child_environment);
                    let parent_bound = substitute_symbolic(parent_bound, &parent_environment);
                    if !self.link_descriptors_equivalent(&child_bound, &parent_bound)? {
                        same_bounds = false;
                        break;
                    }
                }
            }

            if !same_variance || !same_bounds {
                let message = format!(
                    "type parameter {} on {name_text} must preserve the generic contract of {}",
                    position + 1,
                    parent_name,
                );
                return Err(VirtualMachineControl::Throw(self.declaration_error_at(
                    self.tables.well_known.linker_error,
                    message.clone(),
                    path,
                    DiagnosticLabel {
                        span: base.span,
                        message,
                    },
                )));
            }
        }

        Ok(())
    }

    pub(in crate::linker) fn check_base_reference(
        &mut self,
        current: &RuntimeClass,
        current_id: ClassId,
        base: &CompiledBaseReference,
        base_id: ClassId,
        name_text: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        let supplied = base.type_arguments.as_ref().map_or(0, Vec::len);

        let Some((required, total)) = self.tables.classes[base_id.0 as usize].type_parameter_arity
        else {
            return Ok(());
        };

        let base_text = self.tables.classes[base_id.0 as usize]
            .name
            .to_string_lossy()
            .into_owned();

        if supplied < required as usize || supplied > total as usize {
            let message = if total == 0 {
                format!(
                    "{name_text} supplies {supplied} type argument(s) to {base_text}, which is not \
                     generic"
                )
            } else if required == total {
                format!(
                    "{name_text} supplies {supplied} type argument(s) to {base_text}, which takes \
                     exactly {total}"
                )
            } else {
                format!(
                    "{name_text} supplies {supplied} type argument(s) to {base_text}, which takes \
                     {required} to {total}"
                )
            };
            return Err(VirtualMachineControl::Throw(self.declaration_error_at(
                self.tables.well_known.linker_error,
                message.clone(),
                path,
                DiagnosticLabel {
                    span: base.span,
                    message,
                },
            )));
        }

        self.check_base_variance(current, base, base_id, path)?;

        let Some(current_environment) =
            self.symbolic_class_environment(current, current_id, current_id)
        else {
            let message = format!("the type parameters of {name_text} cannot be resolved");
            return Err(VirtualMachineControl::Throw(self.declaration_error_at(
                self.tables.well_known.linker_error,
                message.clone(),
                path,
                DiagnosticLabel {
                    span: base.span,
                    message,
                },
            )));
        };

        let parameters = Rc::clone(&self.tables.classes[base_id.0 as usize].type_parameters);
        let mut base_environment = current_environment.clone();
        let mut arguments = Vec::with_capacity(parameters.len());
        for (position, parameter) in parameters.iter().enumerate() {
            let argument =
                base.type_arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get(position))
                    .map_or_else(
                        || {
                            // SAFETY: arity checking guarantees a missing argument has a default.
                            unsafe {
                                unwrap_option_invariant(
                                    parameter.default.as_ref().map(|default| {
                                        substitute_symbolic(default, &base_environment)
                                    }),
                                    "arity checking guarantees a trailing default",
                                )
                            }
                        },
                        |argument| substitute_symbolic(argument, &current_environment),
                    );
            base_environment.insert(parameter.name.clone(), argument.clone());
            arguments.push(argument);
        }

        for (position, (parameter, argument)) in parameters.iter().zip(&arguments).enumerate() {
            for bound in &parameter.bounds {
                let bound = substitute_symbolic(bound, &base_environment);
                if !self.link_descriptor_is_subtype(argument, &bound)? {
                    let message = format!(
                        "type argument {} supplied by {name_text} does not satisfy the bound of {} on {base_text}",
                        position + 1,
                        parameter.name.to_string_lossy()
                    );
                    return Err(VirtualMachineControl::Throw(self.declaration_error_at(
                        self.tables.well_known.linker_error,
                        message.clone(),
                        path,
                        DiagnosticLabel {
                            span: base.span,
                            message,
                        },
                    )));
                }
            }
        }

        Ok(())
    }

    fn check_base_variance(
        &mut self,
        current: &RuntimeClass,
        base: &CompiledBaseReference,
        base_id: ClassId,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        let Some(arguments) = &base.type_arguments else {
            return Ok(());
        };
        let parameters = Rc::clone(&self.tables.classes[base_id.0 as usize].type_parameters);
        for (parameter, argument) in parameters.iter().zip(arguments) {
            let polarity = match parameter.variance {
                Variance::Invariant => 0,
                Variance::Covariant => 1,
                Variance::Contravariant => -1,
            };
            let incompatible = incompatible_parameter(
                argument,
                polarity,
                &current.type_parameters,
                |name, index| self.named_type_parameter_variance(name, index),
            );
            let Some(incompatible) = incompatible else {
                continue;
            };
            let variance = match incompatible.variance {
                Variance::Invariant => "invariant",
                Variance::Covariant => "covariant",
                Variance::Contravariant => "contravariant",
            };
            let message = format!(
                "the {variance} type parameter `{}` is used in an incompatible position",
                incompatible.name.to_string_lossy()
            );
            return Err(self.linker_error_at(path, base.span, message));
        }

        Ok(())
    }

    fn named_type_parameter_variance(&self, name: &Atom, index: usize) -> Option<Variance> {
        let entry = self.tables.symbols.get(name)?;
        match entry.kind {
            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface => self.tables.classes
                [entry.index as usize]
                .type_parameters
                .get(index)
                .map(|parameter| parameter.variance),
            SymbolKind::TypeAlias => self.tables.type_aliases[entry.index as usize]
                .type_parameters
                .get(index)
                .map(|parameter| parameter.variance),
            SymbolKind::Newtype => self.tables.newtypes[entry.index as usize]
                .type_parameters
                .get(index)
                .map(|parameter| parameter.variance),
            SymbolKind::Function => match entry.table {
                FunctionTable::User => self.tables.functions[entry.index as usize]
                    .type_parameters()
                    .get(index)
                    .map(|parameter| parameter.variance),
                FunctionTable::BuiltIn => self.tables.built_in_functions[entry.index as usize]
                    .type_parameters()
                    .get(index)
                    .map(|parameter| parameter.variance),
            },
            SymbolKind::Constant => None,
        }
    }

    pub(in crate::linker) fn method_shape(&self, entry: &MethodEntry) -> (usize, usize, Vec<Atom>) {
        match entry.body {
            MethodBodyKind::Bytecode(function) => {
                let function = &self.tables.functions[function.0 as usize];
                // SAFETY: the table and prior lookup prove this pointer or index.
                let parameters = unsafe { function.parameters.as_ref() };
                (
                    usize::from(function.required_parameters),
                    usize::from(function.declared_parameters),
                    parameters
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .collect(),
                )
            }
            MethodBodyKind::BuiltIn(body) => (
                body.parameters
                    .iter()
                    .take_while(|parameter| !parameter.optional)
                    .count(),
                body.parameters.len(),
                body.parameters
                    .iter()
                    .map(|parameter| self.heap.intern(parameter.name.as_bytes()))
                    .collect(),
            ),
        }
    }

    pub(in crate::linker) fn symbolic_class_environment(
        &self,
        current: &RuntimeClass,
        current_id: ClassId,
        target: ClassId,
    ) -> Option<HashMap<Atom, TypeDescriptor>> {
        let mut root = HashMap::new();
        for parameter in current.type_parameters.iter() {
            root.insert(
                parameter.name.clone(),
                TypeDescriptor::Parameter(parameter.name.clone()),
            );
        }

        for parameter in current.type_parameters.iter() {
            if parameter.bounds.is_empty() {
                continue;
            }

            let mut members = vec![TypeDescriptor::Parameter(parameter.name.clone())];
            members.extend(
                parameter
                    .bounds
                    .iter()
                    .map(|bound| substitute_symbolic(bound, &root)),
            );

            root.insert(
                parameter.name.clone(),
                TypeDescriptor::Intersection(members),
            );
        }

        if current_id == target {
            return Some(root);
        }
        if let Some(projected) = self.symbolic_projection_environment(current, target, &root) {
            return Some(projected);
        }

        self.walk_symbolic_class_environment(current, current_id, current_id, target, root, 0)
    }

    fn symbolic_projection_environment(
        &self,
        current: &RuntimeClass,
        target: ClassId,
        root: &HashMap<Atom, TypeDescriptor>,
    ) -> Option<HashMap<Atom, TypeDescriptor>> {
        let arguments = current.base_specializations.get(&target)?;
        let declaration = &self.tables.classes[target.0 as usize];
        if arguments.len() != declaration.type_parameters.len() {
            return None;
        }

        let mut projected = root.clone();
        for (parameter, argument) in declaration.type_parameters.iter().zip(arguments) {
            projected.insert(parameter.name.clone(), substitute_symbolic(argument, root));
        }

        Some(projected)
    }

    pub(in crate::linker) fn walk_symbolic_class_environment(
        &self,
        root_class: &RuntimeClass,
        root_id: ClassId,
        class: ClassId,
        target: ClassId,
        environment: HashMap<Atom, TypeDescriptor>,
        depth: u32,
    ) -> Option<HashMap<Atom, TypeDescriptor>> {
        if class == target {
            return Some(environment);
        }

        if depth > MAX_TYPE_DEPTH_U32 {
            return None;
        }

        let bases = if class == root_id {
            &root_class.direct_bases
        } else {
            &self.tables.classes[class.0 as usize].direct_bases
        };

        for base in bases {
            let declaration = &self.tables.classes[base.class.0 as usize];
            let mut next = environment.clone();
            for (position, parameter) in declaration.type_parameters.iter().enumerate() {
                let argument = if let Some(argument) = base
                    .type_arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get(position))
                {
                    substitute_symbolic(argument, &environment)
                } else {
                    let default = parameter.default.as_ref()?;
                    substitute_symbolic(default, &next)
                };
                next.insert(parameter.name.clone(), argument);
            }

            if let Some(found) = self.walk_symbolic_class_environment(
                root_class,
                root_id,
                base.class,
                target,
                next,
                depth + 1,
            ) {
                return Some(found);
            }
        }

        None
    }

    pub(in crate::linker) fn merge_base_specializations(
        &mut self,
        current: &RuntimeClass,
        current_id: ClassId,
        name_text: &str,
        path: &Atom,
    ) -> Result<HashMap<ClassId, Vec<TypeDescriptor>>, VirtualMachineControl> {
        let root = current
            .type_parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    TypeDescriptor::Parameter(parameter.name.clone()),
                )
            })
            .collect();

        let mut inherited = HashMap::new();
        self.collect_base_specializations(
            current,
            current_id,
            current_id,
            &root,
            &mut inherited,
            0,
        );

        let mut merged_bases = HashMap::new();
        for (base_id, specializations) in inherited {
            let Some(mut merged) = specializations.first().cloned() else {
                continue;
            };
            let parameters = Rc::clone(&self.tables.classes[base_id.0 as usize].type_parameters);

            for specialization in specializations.iter().skip(1) {
                if merged.len() != specialization.len() || merged.len() != parameters.len() {
                    let base_text = self.tables.classes[base_id.0 as usize].name.to_string();
                    return Err(VirtualMachineControl::Throw(self.declaration_error(
                        self.tables.well_known.linker_error,
                        format!("{name_text} inherits incompatible specializations of {base_text}"),
                        path,
                    )));
                }

                for ((argument, candidate), parameter) in
                    merged.iter_mut().zip(specialization).zip(parameters.iter())
                {
                    if descriptors_equal(argument, candidate, 0)
                        || self.link_descriptors_equivalent(argument, candidate)?
                    {
                        continue;
                    }

                    match parameter.variance {
                        Variance::Contravariant => {
                            *argument =
                                TypeDescriptor::Union(vec![argument.clone(), candidate.clone()]);
                        }
                        Variance::Covariant => {
                            *argument = TypeDescriptor::intersection(vec![
                                argument.clone(),
                                candidate.clone(),
                            ]);
                        }
                        Variance::Invariant => {
                            let base_text =
                                self.tables.classes[base_id.0 as usize].name.to_string();
                            return Err(VirtualMachineControl::Throw(self.declaration_error(
                                self.tables.well_known.linker_error,
                                format!(
                                    "{name_text} inherits incompatible specializations of \
                                     {base_text}"
                                ),
                                path,
                            )));
                        }
                    }
                }
            }

            merged_bases.insert(base_id, merged);
        }

        Ok(merged_bases)
    }

    fn collect_base_specializations(
        &self,
        root_class: &RuntimeClass,
        root_id: ClassId,
        class: ClassId,
        environment: &HashMap<Atom, TypeDescriptor>,
        inherited: &mut HashMap<ClassId, Vec<Vec<TypeDescriptor>>>,
        depth: u32,
    ) {
        if depth > MAX_TYPE_DEPTH_U32 {
            return;
        }

        let bases = if class == root_id {
            &root_class.direct_bases
        } else {
            &self.tables.classes[class.0 as usize].direct_bases
        };

        for base in bases {
            let declaration = &self.tables.classes[base.class.0 as usize];
            let mut next = environment.clone();
            let mut specialization = Vec::with_capacity(declaration.type_parameters.len());
            for (position, parameter) in declaration.type_parameters.iter().enumerate() {
                let argument = if let Some(argument) = base
                    .type_arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get(position))
                {
                    substitute_symbolic(argument, environment)
                } else {
                    let Some(default) = parameter.default.as_ref() else {
                        return;
                    };
                    substitute_symbolic(default, &next)
                };
                next.insert(parameter.name.clone(), argument.clone());
                specialization.push(argument);
            }

            let specializations = inherited.entry(base.class).or_default();
            if specializations.iter().any(|existing| {
                existing.len() == specialization.len()
                    && existing
                        .iter()
                        .zip(&specialization)
                        .all(|(left, right)| descriptors_equal(left, right, 0))
            }) {
                continue;
            }
            specializations.push(specialization);
            self.collect_base_specializations(
                root_class,
                root_id,
                base.class,
                &next,
                inherited,
                depth + 1,
            );
        }
    }

    pub(in crate::linker) fn method_types(
        &self,
        entry: &MethodEntry,
    ) -> (
        Vec<TypeDescriptor>,
        TypeDescriptor,
        Vec<CompiledTypeParameter>,
    ) {
        match entry.body {
            MethodBodyKind::Bytecode(function) => {
                let function = &self.tables.functions[function.0 as usize];
                (
                    function
                        .parameters()
                        .iter()
                        .map(|parameter| {
                            parameter
                                .declared_type
                                .clone()
                                .unwrap_or(TypeDescriptor::Mixed)
                        })
                        .collect(),
                    function
                        .return_type
                        .as_deref()
                        .cloned()
                        .unwrap_or(TypeDescriptor::Mixed),
                    function.type_parameters().to_vec(),
                )
            }
            MethodBodyKind::BuiltIn(body) => (
                body.parameters
                    .iter()
                    .map(|parameter| {
                        descriptor_from_built_in_spec(&self.heap, &parameter.type_spec)
                    })
                    .collect(),
                descriptor_from_built_in_spec(&self.heap, &body.return_spec),
                body.type_parameters
                    .iter()
                    .map(|parameter| CompiledTypeParameter {
                        name: self.heap.intern(parameter.name.as_bytes()),
                        span: Span::zero(), // built-in methods have no source span for their type parameters
                        variance: parameter.variance,
                        bounds: parameter
                            .bounds
                            .iter()
                            .map(|bound| descriptor_from_built_in_spec(&self.heap, bound))
                            .collect(),
                        default: parameter
                            .default
                            .as_ref()
                            .map(|default| descriptor_from_built_in_spec(&self.heap, default)),
                    })
                    .collect(),
            ),
        }
    }

    pub(in crate::linker) fn check_override(
        &mut self,
        check: &OverrideCheck<'_>,
    ) -> Result<(), VirtualMachineControl> {
        let OverrideCheck {
            current,
            current_id,
            method_name,
            replacement,
            replaced,
            name_text,
            source,
            enforce_constructor,
            path,
        } = *check;
        let method_text = method_name.to_string_lossy().into_owned();
        let role = source.role(&method_text);
        let exempt_constructor = method_name.as_bytes() == b"__construct"
            && matches!(source, Replaced::Inherited)
            && !enforce_constructor;
        let shapes = if exempt_constructor {
            None
        } else {
            Some((self.method_shape(replacement), self.method_shape(replaced)))
        };

        let replaced_noun = source.describe();
        let mut reason = if replaced.is_final {
            Some(format!("{replaced_noun} is final and cannot be overridden"))
        } else if replaced.is_static != replacement.is_static {
            Some(if replaced.is_static {
                format!("{replaced_noun} is static and this one is not")
            } else {
                format!("this method is static and {replaced_noun} is not")
            })
        } else if visibility_rank(replacement.visibility) < visibility_rank(replaced.visibility) {
            Some(format!(
                "it is {} where {replaced_noun} is {}; an override may only widen visibility",
                visibility_name(replacement.visibility),
                visibility_name(replaced.visibility)
            ))
        } else if let Some((new_shape, old_shape)) = shapes {
            let (new_required, new_total, new_names) = new_shape;
            let (old_required, old_total, old_names) = old_shape;
            let mismatched_name = new_names
                .iter()
                .zip(old_names.iter())
                .position(|(new_name, old_name)| new_name != old_name);
            if new_required > old_required {
                Some(format!(
                    "it requires {new_required} argument(s) where {replaced_noun} requires \
                     {old_required}; an override may not demand more"
                ))
            } else if new_total < old_total {
                Some(format!(
                    "it accepts {new_total} argument(s) where {replaced_noun} accepts \
                     {old_total}; an override may only add trailing optional parameters"
                ))
            } else {
                mismatched_name.map(|position| {
                    format!(
                        "parameter {} is named `{}` where {replaced_noun} names it `{}`; a \
                         caller may pass it by name",
                        position + 1,
                        new_names[position].to_string_lossy(),
                        old_names[position].to_string_lossy()
                    )
                })
            }
        } else {
            None
        };

        if reason.is_none() && !exempt_constructor {
            reason = self.override_type_reason(
                current,
                current_id,
                replacement,
                replaced,
                replaced_noun,
            )?;
        }

        let Some(reason) = reason else {
            return Ok(());
        };

        Err(VirtualMachineControl::Throw(self.declaration_error(
            self.tables.well_known.linker_error,
            format!("{name_text}::{method_text} is not compatible with {role}: {reason}"),
            path,
        )))
    }

    pub(in crate::linker) fn override_type_reason(
        &mut self,
        current: &RuntimeClass,
        current_id: ClassId,
        replacement: &MethodEntry,
        replaced: &MethodEntry,
        replaced_noun: &str,
    ) -> Result<Option<String>, VirtualMachineControl> {
        let Some(mut replacement_environment) =
            self.symbolic_class_environment(current, current_id, replacement.declaring_class)
        else {
            return Ok(Some(
                "its declaring-class type arguments cannot be resolved".to_string(),
            ));
        };

        let Some(mut replaced_environment) =
            self.symbolic_class_environment(current, current_id, replaced.declaring_class)
        else {
            return Ok(Some(format!(
                "the type arguments of {replaced_noun} cannot be resolved"
            )));
        };

        let (replacement_parameters, replacement_return, replacement_binders) =
            self.method_types(replacement);
        let (replaced_parameters, replaced_return, replaced_binders) = self.method_types(replaced);

        if let Some(reason) = self.override_binder_reason(
            &replacement_binders,
            &replaced_binders,
            &mut replacement_environment,
            &mut replaced_environment,
            replaced_noun,
        )? {
            return Ok(Some(reason));
        }

        for (position, (replacement_parameter, replaced_parameter)) in replacement_parameters
            .iter()
            .zip(&replaced_parameters)
            .enumerate()
        {
            let replacement_parameter =
                substitute_symbolic(replacement_parameter, &replacement_environment);
            let replaced_parameter = substitute_symbolic(replaced_parameter, &replaced_environment);
            if !self.link_descriptor_is_subtype(&replaced_parameter, &replacement_parameter)? {
                return Ok(Some(format!(
                    "parameter {} is not contravariant with {replaced_noun}",
                    position + 1
                )));
            }
        }

        let mut replacement_return =
            substitute_symbolic(&replacement_return, &replacement_environment);
        let mut replaced_return = substitute_symbolic(&replaced_return, &replaced_environment);
        if current.is_final {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let current_environment = unsafe {
                unwrap_option_invariant(
                    self.symbolic_class_environment(current, current_id, current_id),
                    "the current class environment was resolved above",
                )
            };
            let current_type = TypeDescriptor::Named {
                name: current.name.clone(),
                arguments: (!current.type_parameters.is_empty()).then(|| {
                    current
                        .type_parameters
                        .iter()
                        .map(|parameter| {
                            // SAFETY: the surrounding invariant proves this option contains a value.
                            unsafe {
                                unwrap_option_invariant(
                                    current_environment.get(&parameter.name),
                                    "the current class environment contains every parameter",
                                )
                            }
                            .clone()
                        })
                        .collect()
                }),
                recursive: false,
            };
            replacement_return = resolve_final_static(&replacement_return, &current_type);
            replaced_return = resolve_final_static(&replaced_return, &current_type);
        }
        let projected_return = self
            .project_current_named_descriptor(
                current,
                current_id,
                &replacement_return,
                &replaced_return,
            )
            .unwrap_or_else(|| replacement_return.clone());
        if !self.link_descriptor_is_subtype(&projected_return, &replaced_return)? {
            return Ok(Some(format!(
                "its return type is not covariant with {replaced_noun}"
            )));
        }

        Ok(None)
    }

    fn override_binder_reason(
        &mut self,
        replacement: &[CompiledTypeParameter],
        replaced: &[CompiledTypeParameter],
        replacement_environment: &mut HashMap<Atom, TypeDescriptor>,
        replaced_environment: &mut HashMap<Atom, TypeDescriptor>,
        replaced_noun: &str,
    ) -> Result<Option<String>, VirtualMachineControl> {
        if replacement.len() != replaced.len() {
            return Ok(Some(format!(
                "it declares {} method type parameter(s) where {replaced_noun} declares {}",
                replacement.len(),
                replaced.len()
            )));
        }

        for (position, (replacement, replaced)) in replacement.iter().zip(replaced).enumerate() {
            if replacement.variance != replaced.variance {
                return Ok(Some(format!(
                    "method type parameter {} has different variance from {replaced_noun}",
                    position + 1
                )));
            }

            let symbolic = TypeDescriptor::Parameter(
                self.heap
                    .intern(format!("\0override:{position}").as_bytes()),
            );
            replacement_environment.insert(replacement.name.clone(), symbolic.clone());
            replaced_environment.insert(replaced.name.clone(), symbolic);

            if replacement.bounds.len() != replaced.bounds.len() {
                return Ok(Some(format!(
                    "method type parameter {} has different bounds from {replaced_noun}",
                    position + 1
                )));
            }

            for (replacement_bound, replaced_bound) in
                replacement.bounds.iter().zip(&replaced.bounds)
            {
                let replacement_bound =
                    substitute_symbolic(replacement_bound, replacement_environment);
                let replaced_bound = substitute_symbolic(replaced_bound, replaced_environment);
                if !self.link_descriptors_equivalent(&replacement_bound, &replaced_bound)? {
                    return Ok(Some(format!(
                        "method type parameter {} has different bounds from {replaced_noun}",
                        position + 1
                    )));
                }
            }

            let defaults_match = match (&replacement.default, &replaced.default) {
                (Some(replacement), Some(replaced)) => {
                    let replacement = substitute_symbolic(replacement, replacement_environment);
                    let replaced = substitute_symbolic(replaced, replaced_environment);
                    self.link_descriptors_equivalent(&replacement, &replaced)?
                }
                (None, None) => true,
                _ => false,
            };
            if !defaults_match {
                return Ok(Some(format!(
                    "method type parameter {} has a different default from {replaced_noun}",
                    position + 1
                )));
            }
        }

        Ok(None)
    }

    fn project_current_named_descriptor(
        &self,
        current: &RuntimeClass,
        current_id: ClassId,
        actual: &TypeDescriptor,
        expected: &TypeDescriptor,
    ) -> Option<TypeDescriptor> {
        if let TypeDescriptor::Union(members) = actual {
            let mut changed = false;
            let projected = members
                .iter()
                .map(|member| {
                    self.project_current_named_descriptor(current, current_id, member, expected)
                        .map_or_else(
                            || member.clone(),
                            |projected| {
                                changed = true;
                                projected
                            },
                        )
                })
                .collect();

            return changed.then_some(TypeDescriptor::Union(projected));
        }

        let TypeDescriptor::Named {
            name: actual_name,
            arguments: actual_arguments,
            ..
        } = actual
        else {
            return None;
        };
        if actual_name != &current.name {
            return None;
        }

        let TypeDescriptor::Named {
            name: expected_name,
            ..
        } = expected
        else {
            return None;
        };
        let entry = self.tables.symbols.get(expected_name)?;
        if !matches!(
            entry.kind,
            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
        ) {
            return None;
        }
        let target = ClassId(entry.index);

        let root_environment = match actual_arguments {
            Some(arguments) if arguments.len() == current.type_parameters.len() => current
                .type_parameters
                .iter()
                .zip(arguments)
                .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
                .collect(),
            Some(_) => return None,
            None => self.symbolic_class_environment(current, current_id, current_id)?,
        };
        let projected = self
            .symbolic_projection_environment(current, target, &root_environment)
            .or_else(|| {
                self.walk_symbolic_class_environment(
                    current,
                    current_id,
                    current_id,
                    target,
                    root_environment,
                    0,
                )
            })?;
        let target_class = &self.tables.classes[target.0 as usize];
        let arguments = target_class
            .type_parameters
            .iter()
            .map(|parameter| projected.get(&parameter.name).cloned())
            .collect::<Option<Vec<_>>>()?;

        Some(TypeDescriptor::Named {
            name: target_class.name.clone(),
            arguments: Some(arguments),
            recursive: false,
        })
    }

    pub(in crate::linker) fn link_descriptor_is_subtype(
        &mut self,
        actual: &TypeDescriptor,
        expected: &TypeDescriptor,
    ) -> Result<bool, VirtualMachineControl> {
        let mut vm = VirtualMachine::new(self);
        vm.descriptor_is_subtype(actual, expected, TypeEnvironmentId::default(), 0)
    }

    pub(in crate::linker) fn link_descriptors_equivalent(
        &mut self,
        left: &TypeDescriptor,
        right: &TypeDescriptor,
    ) -> Result<bool, VirtualMachineControl> {
        if !self.link_descriptor_is_subtype(left, right)? {
            return Ok(false);
        }

        self.link_descriptor_is_subtype(right, left)
    }

    pub(in crate::linker) fn check_property_override(
        &mut self,
        current: &RuntimeClass,
        current_id: ClassId,
        replacement: &PropertyInfo,
        replaced: &PropertyInfo,
        name_text: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        if replacement.is_readonly != replaced.is_readonly {
            let property_name = replacement.name.to_string();
            let declaring_name = self.tables.classes[replaced.declaring_class.0 as usize]
                .name
                .to_string();
            let inherited_mutability = if replaced.is_readonly {
                "readonly"
            } else {
                "writable"
            };
            let replacement_mutability = if replacement.is_readonly {
                "readonly"
            } else {
                "writable"
            };
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "{name_text}::${property_name} cannot redeclare {inherited_mutability} \
                     inherited property {declaring_name}::${property_name} as \
                     {replacement_mutability}"
                ),
                path,
            )));
        }

        let Some(replacement_environment) =
            self.symbolic_class_environment(current, current_id, replacement.declaring_class)
        else {
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "the type arguments of {name_text} cannot be resolved while checking its \
                     property declarations"
                ),
                path,
            )));
        };
        let Some(replaced_environment) =
            self.symbolic_class_environment(current, current_id, replaced.declaring_class)
        else {
            let declaring_name = self.tables.classes[replaced.declaring_class.0 as usize]
                .name
                .to_string();
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "the type arguments of inherited properties from {declaring_name} cannot be \
                     resolved"
                ),
                path,
            )));
        };

        let replacement_type = substitute_symbolic(
            replacement
                .declared_type
                .as_ref()
                .unwrap_or(&TypeDescriptor::Mixed),
            &replacement_environment,
        );
        let replaced_type = substitute_symbolic(
            replaced
                .declared_type
                .as_ref()
                .unwrap_or(&TypeDescriptor::Mixed),
            &replaced_environment,
        );
        if self.link_descriptors_equivalent(&replacement_type, &replaced_type)? {
            return Ok(());
        }

        let property_name = replacement.name.to_string();
        let declaring_name = self.tables.classes[replaced.declaring_class.0 as usize]
            .name
            .to_string();
        Err(VirtualMachineControl::Throw(self.declaration_error(
            self.tables.well_known.linker_error,
            format!(
                "{name_text}::${property_name} is not compatible with inherited property \
                 {declaring_name}::${property_name}: property types are invariant"
            ),
            path,
        )))
    }

    pub(in crate::linker) fn check_declared_contracts(
        &mut self,
        class: &RuntimeClass,
        current_id: ClassId,
        compiled: &CompiledClassLike,
        name_text: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        let contract = DeclaredContract {
            class,
            current_id,
            kind: compiled.kind,
            name: name_text,
            path,
        };
        let inherited_contracts = !class.is_abstract
            && class
                .parent
                .is_some_and(|parent| self.tables.classes[parent.0 as usize].is_abstract);
        let interfaces: Vec<_> = if inherited_contracts {
            class.interfaces.iter().copied().collect()
        } else {
            class
                .direct_bases
                .iter()
                .filter(|base| {
                    self.tables.classes[base.class.0 as usize].kind == ClassLikeKind::Interface
                })
                .map(|base| base.class)
                .collect()
        };

        for interface_id in interfaces {
            let interface = &self.tables.classes[interface_id.0 as usize];
            let requirement = InterfaceRequirements {
                name: interface.name.to_string_lossy().into_owned(),
                methods: interface
                    .methods()
                    .map(|(method_name, entry)| (method_name.clone(), entry))
                    .collect(),
                properties: interface.slots.iter().cloned().collect(),
            };
            self.check_required_methods(&contract, &requirement)?;
            self.check_required_properties(&contract, &requirement)?;
        }

        self.check_abstract_methods(&contract)
    }

    fn check_required_methods(
        &mut self,
        contract: &DeclaredContract<'_>,
        requirement: &InterfaceRequirements,
    ) -> Result<(), VirtualMachineControl> {
        for (method_name, required) in &requirement.methods {
            let method_text = method_name.to_string_lossy();
            let Some(provided) = contract.class.method(method_name) else {
                if contract.class.is_abstract {
                    continue;
                }

                return Err(VirtualMachineControl::Throw(self.declaration_error(
                    self.tables.well_known.linker_error,
                    format!(
                        "{} declares that it implements {}, and does not provide {method_text}",
                        contract.name, requirement.name
                    ),
                    contract.path,
                )));
            };

            if provided.is_abstract {
                if contract.class.is_abstract {
                    continue;
                }

                return Err(VirtualMachineControl::Throw(self.declaration_error(
                    self.tables.well_known.linker_error,
                    format!(
                        "{} declares that it implements {}, and leaves {method_text} abstract",
                        contract.name, requirement.name
                    ),
                    contract.path,
                )));
            }

            if !required.is_abstract && provided.declaring_class == required.declaring_class {
                continue;
            }

            self.check_override(&OverrideCheck {
                current: contract.class,
                current_id: contract.current_id,
                method_name,
                replacement: &provided,
                replaced: required,
                name_text: contract.name,
                source: Replaced::Required(&requirement.name),
                enforce_constructor: true,
                path: contract.path,
            })?;
        }

        Ok(())
    }

    fn check_required_properties(
        &mut self,
        contract: &DeclaredContract<'_>,
        requirement: &InterfaceRequirements,
    ) -> Result<(), VirtualMachineControl> {
        for required in &requirement.properties {
            self.check_required_property(contract, requirement, required)?;
        }

        Ok(())
    }

    fn check_required_property(
        &mut self,
        contract: &DeclaredContract<'_>,
        requirement: &InterfaceRequirements,
        required: &PropertyInfo,
    ) -> Result<(), VirtualMachineControl> {
        let property = required.name.to_string();
        let mutability = if required.is_readonly {
            "readonly"
        } else {
            "writable"
        };
        let Some(slot) = contract.class.slot_names.get(&required.name).copied() else {
            if contract.class.is_abstract {
                return Ok(());
            }

            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "{} declares that it implements {}, and does not provide the {mutability} \
                     property ${property}",
                    contract.name, requirement.name
                ),
                contract.path,
            )));
        };
        let provided = &contract.class.slots[slot as usize];
        if provided.visibility != Visibility::Public {
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "{}::${property} must be public to satisfy {}::${property}",
                    contract.name, requirement.name
                ),
                contract.path,
            )));
        }
        if provided.is_readonly != required.is_readonly {
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "{}::${property} must be {mutability} to satisfy {}::${property}",
                    contract.name, requirement.name
                ),
                contract.path,
            )));
        }

        let Some(provided_environment) = self.symbolic_class_environment(
            contract.class,
            contract.current_id,
            provided.declaring_class,
        ) else {
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "the type arguments of {} cannot be resolved while checking its property \
                     declarations",
                    contract.name
                ),
                contract.path,
            )));
        };
        let Some(required_environment) = self.symbolic_class_environment(
            contract.class,
            contract.current_id,
            required.declaring_class,
        ) else {
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "the type arguments of interface {} cannot be resolved while checking {}",
                    requirement.name, contract.name
                ),
                contract.path,
            )));
        };
        let provided_type = substitute_symbolic(
            provided
                .declared_type
                .as_ref()
                .unwrap_or(&TypeDescriptor::Mixed),
            &provided_environment,
        );
        let required_type = substitute_symbolic(
            required
                .declared_type
                .as_ref()
                .unwrap_or(&TypeDescriptor::Mixed),
            &required_environment,
        );
        if self.link_descriptors_equivalent(&provided_type, &required_type)? {
            return Ok(());
        }

        Err(VirtualMachineControl::Throw(self.declaration_error(
            self.tables.well_known.linker_error,
            format!(
                "{}::${property} is not compatible with {}::${property}: property types are \
                 invariant",
                contract.name, requirement.name
            ),
            contract.path,
        )))
    }

    fn check_abstract_methods(
        &mut self,
        contract: &DeclaredContract<'_>,
    ) -> Result<(), VirtualMachineControl> {
        if contract.class.is_abstract {
            return Ok(());
        }

        let mut methods: Vec<String> = contract
            .class
            .methods()
            .filter(|(_, entry)| entry.is_abstract)
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        if methods.is_empty() {
            return Ok(());
        }

        methods.sort();
        let methods = methods.join(", ");
        let reason = match contract.kind {
            ClassLikeKind::Enum => format!(
                "the enum {} does not implement {methods}; an enum's cases are instances and must \
                 implement every method",
                contract.name
            ),
            _ => format!(
                "the class {} is not abstract and does not implement {methods}",
                contract.name
            ),
        };

        Err(VirtualMachineControl::Throw(self.declaration_error(
            self.tables.well_known.linker_error,
            reason,
            contract.path,
        )))
    }
}

fn resolve_final_static(
    descriptor: &TypeDescriptor,
    current_type: &TypeDescriptor,
) -> TypeDescriptor {
    match descriptor {
        TypeDescriptor::StaticClass => current_type.clone(),
        TypeDescriptor::Named {
            name,
            arguments,
            recursive,
        } => TypeDescriptor::Named {
            name: name.clone(),
            arguments: arguments.as_ref().map(|arguments| {
                arguments
                    .iter()
                    .map(|argument| resolve_final_static(argument, current_type))
                    .collect()
            }),
            recursive: *recursive,
        },
        TypeDescriptor::Array(arguments) => {
            TypeDescriptor::Array(arguments.as_ref().map(|(key, value)| {
                (
                    Box::new(resolve_final_static(key, current_type)),
                    Box::new(resolve_final_static(value, current_type)),
                )
            }))
        }
        TypeDescriptor::Vector(element) => TypeDescriptor::Vector(
            element
                .as_ref()
                .map(|element| Box::new(resolve_final_static(element, current_type))),
        ),
        TypeDescriptor::Dictionary(arguments) => {
            TypeDescriptor::Dictionary(arguments.as_ref().map(|(key, value)| {
                (
                    Box::new(resolve_final_static(key, current_type)),
                    Box::new(resolve_final_static(value, current_type)),
                )
            }))
        }
        TypeDescriptor::Callable(signature) => {
            TypeDescriptor::Callable(signature.as_ref().map(|signature| {
                FunctionTypeDescriptor {
                    parameters: signature
                        .parameters
                        .iter()
                        .map(|parameter| FunctionTypeParameterDescriptor {
                            r#type: resolve_final_static(&parameter.r#type, current_type),
                            optional: parameter.optional,
                        })
                        .collect(),
                    return_type: Box::new(resolve_final_static(
                        &signature.return_type,
                        current_type,
                    )),
                }
            }))
        }
        TypeDescriptor::Classname(inner) => {
            TypeDescriptor::Classname(Box::new(resolve_final_static(inner, current_type)))
        }
        TypeDescriptor::Negated(inner) => {
            TypeDescriptor::Negated(Box::new(resolve_final_static(inner, current_type)))
        }
        TypeDescriptor::Tuple(members) => TypeDescriptor::Tuple(
            members
                .iter()
                .map(|member| resolve_final_static(member, current_type))
                .collect(),
        ),
        TypeDescriptor::TupleRest { elements, rest } => TypeDescriptor::TupleRest {
            elements: elements
                .iter()
                .map(|element| resolve_final_static(element, current_type))
                .collect(),
            rest: Box::new(resolve_final_static(rest, current_type)),
        },
        TypeDescriptor::Union(members) => TypeDescriptor::Union(
            members
                .iter()
                .map(|member| resolve_final_static(member, current_type))
                .collect(),
        ),
        TypeDescriptor::Intersection(members) => TypeDescriptor::intersection(
            members
                .iter()
                .map(|member| resolve_final_static(member, current_type))
                .collect(),
        ),
        other => other.clone(),
    }
}
