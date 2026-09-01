//! Declaration-time validation of generic defaults against their bounds.

use std::rc::Rc;

use hashbrown::HashMap;

use crate::bytecode::aliases::expand_aliases;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::is_external;
use crate::classes::MethodBodyKind;
use crate::engine::Engine;
use crate::engine::GenericValidationJournalEntry;
use crate::engine::builtins::built_in_type_parameters;
use crate::linker::descriptors::substitute_symbolic;
use crate::symbols::FunctionTable;
use crate::symbols::SymbolKind;
use crate::value::atom::Atom;
use crate::value::object::ClassId;
use crate::vm::VirtualMachineControl;

impl Engine {
    #[cold]
    pub(crate) fn validate_loaded_type_parameter_defaults(
        &mut self,
    ) -> Result<(), VirtualMachineControl> {
        let pending = self
            .units
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.unit_generic_validation[*index].type_parameter_defaults)
            .map(|(index, context)| (index, Rc::clone(context)))
            .collect::<Vec<_>>();

        for (index, context) in pending {
            if self.validate_unit_type_parameter_defaults(&context.unit, &context.path)? {
                self.unit_generic_validation[index].type_parameter_defaults = true;
                self.generic_validation_journal
                    .push(GenericValidationJournalEntry::TypeParameterDefaults(index));
            }
        }

        Ok(())
    }

    #[cold]
    pub(crate) fn validate_loaded_type_argument_bounds(
        &mut self,
    ) -> Result<(), VirtualMachineControl> {
        let pending = self
            .units
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.unit_generic_validation[*index].type_argument_bounds)
            .map(|(index, context)| (index, Rc::clone(context)))
            .collect::<Vec<_>>();

        for (index, context) in pending {
            let descriptors = unit_descriptors(&context.unit);
            if self.validate_descriptor_type_argument_bounds_all(descriptors, &context.path)? {
                self.unit_generic_validation[index].type_argument_bounds = true;
                self.generic_validation_journal
                    .push(GenericValidationJournalEntry::TypeArgumentBounds(index));
            }
        }

        Ok(())
    }

    fn validate_descriptor_type_argument_bounds(
        &mut self,
        descriptor: &TypeDescriptor,
        path: &Atom,
    ) -> Result<bool, VirtualMachineControl> {
        let validated = match descriptor {
            TypeDescriptor::Named {
                name,
                arguments: Some(arguments),
                ..
            } => {
                let arguments_validated =
                    self.validate_descriptor_type_argument_bounds_all(arguments.iter(), path)?;
                self.validate_named_type_argument_bounds(name, arguments, path)?
                    && arguments_validated
            }
            TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => {
                let mut validated = true;
                if let Some(arguments) = class_arguments {
                    validated &=
                        self.validate_descriptor_type_argument_bounds_all(arguments.iter(), path)?;
                    validated &=
                        self.validate_named_type_argument_bounds(class, arguments, path)?;
                }

                if let Some(arguments) = member_arguments {
                    validated &=
                        self.validate_descriptor_type_argument_bounds_all(arguments.iter(), path)?;
                    validated &=
                        self.validate_member_type_argument_bounds(class, member, arguments, path)?;
                }
                validated
            }
            TypeDescriptor::Array(Some((key, value)))
            | TypeDescriptor::Dictionary(Some((key, value))) => self
                .validate_descriptor_type_argument_bounds_all(
                    [key.as_ref(), value.as_ref()],
                    path,
                )?,
            TypeDescriptor::Vector(Some(element))
            | TypeDescriptor::Classname(element)
            | TypeDescriptor::Negated(element) => {
                self.validate_descriptor_type_argument_bounds(element, path)?
            }
            TypeDescriptor::VectorShape { elements, rest } => self
                .validate_descriptor_type_argument_bounds_all(
                    elements.iter().chain(rest.iter().map(Box::as_ref)),
                    path,
                )?,
            TypeDescriptor::DictionaryShape { entries, rest } => self
                .validate_descriptor_type_argument_bounds_all(
                    entries.iter().map(|(_, value)| value).chain(
                        rest.iter()
                            .flat_map(|(key, value)| [key.as_ref(), value.as_ref()]),
                    ),
                    path,
                )?,
            TypeDescriptor::Callable(Some(signature)) => self
                .validate_descriptor_type_argument_bounds_all(
                    signature
                        .parameters
                        .iter()
                        .map(|parameter| &parameter.r#type)
                        .chain([signature.return_type.as_ref()]),
                    path,
                )?,
            TypeDescriptor::Tuple(members)
            | TypeDescriptor::Union(members)
            | TypeDescriptor::Intersection(members) => {
                self.validate_descriptor_type_argument_bounds_all(members.iter(), path)?
            }
            TypeDescriptor::TupleRest { elements, rest } => self
                .validate_descriptor_type_argument_bounds_all(
                    elements.iter().chain([rest.as_ref()]),
                    path,
                )?,
            _ => true,
        };

        Ok(validated)
    }

    fn validate_descriptor_type_argument_bounds_all<'a>(
        &mut self,
        descriptors: impl IntoIterator<Item = &'a TypeDescriptor>,
        path: &Atom,
    ) -> Result<bool, VirtualMachineControl> {
        let mut validated = true;
        for descriptor in descriptors {
            validated &= self.validate_descriptor_type_argument_bounds(descriptor, path)?;
        }

        Ok(validated)
    }

    fn validate_named_type_argument_bounds(
        &mut self,
        name: &Atom,
        arguments: &[TypeDescriptor],
        path: &Atom,
    ) -> Result<bool, VirtualMachineControl> {
        let Some(entry) = self.tables.symbols.get(name).copied() else {
            return Ok(false);
        };

        let parameters = match entry.kind {
            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface => self.tables.classes
                [entry.index as usize]
                .type_parameters
                .to_vec(),
            SymbolKind::TypeAlias => self.tables.type_aliases[entry.index as usize]
                .type_parameters
                .clone(),
            SymbolKind::Newtype => self.tables.newtypes[entry.index as usize]
                .type_parameters
                .clone(),
            SymbolKind::Function => match entry.table {
                FunctionTable::User => self.tables.functions[entry.index as usize]
                    .type_parameters()
                    .to_vec(),
                FunctionTable::BuiltIn => built_in_type_parameters(
                    &self.heap,
                    self.tables.built_in_functions[entry.index as usize].type_parameters(),
                ),
            },
            SymbolKind::Constant => return Ok(true),
        };

        self.validate_type_argument_bounds(name, &parameters, arguments, path)
    }

    fn validate_member_type_argument_bounds(
        &mut self,
        class: &Atom,
        member: &Atom,
        arguments: &[TypeDescriptor],
        path: &Atom,
    ) -> Result<bool, VirtualMachineControl> {
        let Some(entry) = self.tables.symbols.get(class).copied() else {
            return Ok(false);
        };

        if !matches!(
            entry.kind,
            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
        ) {
            return Ok(true);
        }

        let class_id = ClassId(entry.index);
        let method = self.tables.classes[entry.index as usize]
            .method(member)
            .or_else(|| {
                self.tables.classes[entry.index as usize]
                    .private_methods
                    .get(&(class_id, member.clone()))
                    .copied()
            });

        let Some(method) = method else {
            return Ok(true);
        };

        let parameters = match method.body {
            MethodBodyKind::Bytecode(function) => self.tables.functions[function.0 as usize]
                .type_parameters()
                .to_vec(),
            MethodBodyKind::BuiltIn(body) => {
                built_in_type_parameters(&self.heap, body.type_parameters)
            }
        };

        let subject = self.heap.intern(format!("{class}::{member}").as_bytes());
        self.validate_type_argument_bounds(&subject, &parameters, arguments, path)
    }

    fn validate_type_argument_bounds(
        &mut self,
        subject: &Atom,
        parameters: &[CompiledTypeParameter],
        arguments: &[TypeDescriptor],
        path: &Atom,
    ) -> Result<bool, VirtualMachineControl> {
        if arguments.len() > parameters.len() {
            return Ok(true);
        }
        if parameters
            .iter()
            .all(|parameter| parameter.bounds.is_empty())
        {
            return Ok(true);
        }

        let mut bindings = HashMap::new();
        let mut resolved_arguments = Vec::with_capacity(parameters.len());
        for (position, parameter) in parameters.iter().enumerate() {
            let argument = if let Some(argument) = arguments.get(position) {
                argument.clone()
            } else {
                let Some(default) = &parameter.default else {
                    return Ok(true);
                };
                substitute_symbolic(default, &bindings)
            };

            let argument = expand_aliases(&argument, &self.tables.type_aliases);
            bindings.insert(parameter.name.clone(), argument.clone());
            resolved_arguments.push(argument);
        }

        let mut fully_linked = true;
        for (position, (parameter, argument)) in
            parameters.iter().zip(&resolved_arguments).enumerate()
        {
            for bound in &parameter.bounds {
                let bound = expand_aliases(
                    &substitute_symbolic(bound, &bindings),
                    &self.tables.type_aliases,
                );

                if contains_late_type(argument) || contains_late_type(&bound) {
                    continue;
                }
                if !self.names_are_linked(argument) || !self.names_are_linked(&bound) {
                    fully_linked = false;
                    continue;
                }

                if self.link_descriptor_is_subtype(argument, &bound)? {
                    continue;
                }

                return Err(VirtualMachineControl::Throw(self.declaration_error(
                    self.tables.well_known.linker_error,
                    format!(
                        "type argument {} supplied to {} does not satisfy the bound of {}",
                        position + 1,
                        subject.to_string_lossy(),
                        parameter.name.to_string_lossy()
                    ),
                    path,
                )));
            }
        }
        Ok(fully_linked)
    }

    fn validate_unit_type_parameter_defaults(
        &mut self,
        unit: &CompiledUnit,
        path: &Atom,
    ) -> Result<bool, VirtualMachineControl> {
        let mut validated = true;
        for alias in &unit.type_aliases {
            if !is_external(&alias.attributes) {
                validated &= self.validate_type_parameter_defaults(
                    &alias.name,
                    &alias.type_parameters,
                    path,
                )?;
            }
        }

        for newtype in &unit.newtypes {
            if !is_external(&newtype.attributes) {
                validated &= self.validate_type_parameter_defaults(
                    &newtype.name,
                    &newtype.type_parameters,
                    path,
                )?;
            }
        }

        for function in &unit.functions {
            if !is_external(&function.attributes) {
                validated &= self.validate_type_parameter_defaults(
                    &function.name,
                    &function.type_parameters,
                    path,
                )?;
            }
        }

        for class in &unit.classes {
            if is_external(&class.attributes) {
                continue;
            }

            validated &=
                self.validate_type_parameter_defaults(&class.name, &class.type_parameters, path)?;
            for method in &class.methods {
                if method
                    .function
                    .type_parameters
                    .iter()
                    .all(|parameter| parameter.default.is_none())
                {
                    continue;
                }

                let subject = self
                    .heap
                    .intern(format!("{}::{}", class.name, method.name).as_bytes());
                validated &= self.validate_type_parameter_defaults(
                    &subject,
                    &method.function.type_parameters,
                    path,
                )?;
            }
        }

        Ok(validated)
    }

    fn validate_type_parameter_defaults(
        &mut self,
        subject: &Atom,
        parameters: &[CompiledTypeParameter],
        path: &Atom,
    ) -> Result<bool, VirtualMachineControl> {
        if parameters
            .iter()
            .all(|parameter| parameter.default.is_none())
        {
            return Ok(true);
        }

        let mut bindings = HashMap::new();
        let mut validated = true;
        for parameter in parameters {
            let Some(default) = &parameter.default else {
                continue;
            };

            let default = expand_aliases(
                &substitute_symbolic(default, &bindings),
                &self.tables.type_aliases,
            );

            for bound in &parameter.bounds {
                let bound = expand_aliases(
                    &substitute_symbolic(bound, &bindings),
                    &self.tables.type_aliases,
                );

                if contains_late_type(&default) || contains_late_type(&bound) {
                    continue;
                }
                if !self.names_are_linked(&default) || !self.names_are_linked(&bound) {
                    validated = false;
                    continue;
                }

                if self.link_descriptor_is_subtype(&default, &bound)? {
                    continue;
                }

                return Err(VirtualMachineControl::Throw(self.declaration_error(
                    self.tables.well_known.linker_error,
                    format!(
                        "the default for type parameter {} on {} does not satisfy its bound",
                        parameter.name.to_string_lossy(),
                        subject.to_string_lossy()
                    ),
                    path,
                )));
            }

            bindings.insert(parameter.name.clone(), default);
        }

        Ok(validated)
    }

    fn names_are_linked(&self, descriptor: &TypeDescriptor) -> bool {
        match descriptor {
            TypeDescriptor::Named {
                name, arguments, ..
            } => {
                self.tables.symbols.contains_key(name)
                    && arguments.as_ref().is_none_or(|arguments| {
                        arguments
                            .iter()
                            .all(|argument| self.names_are_linked(argument))
                    })
            }
            TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => {
                let member_is_linked = self.tables.symbols.get(class).is_some_and(|entry| {
                    matches!(
                        entry.kind,
                        SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                    ) && self.tables.classes[entry.index as usize]
                        .members
                        .contains_key(member)
                });
                member_is_linked
                    && class_arguments.as_ref().is_none_or(|arguments| {
                        arguments
                            .iter()
                            .all(|argument| self.names_are_linked(argument))
                    })
                    && member_arguments.as_ref().is_none_or(|arguments| {
                        arguments
                            .iter()
                            .all(|argument| self.names_are_linked(argument))
                    })
            }
            TypeDescriptor::Array(arguments) => arguments.as_ref().is_none_or(|(key, value)| {
                self.names_are_linked(key) && self.names_are_linked(value)
            }),
            TypeDescriptor::Vector(element) => element
                .as_ref()
                .is_none_or(|element| self.names_are_linked(element)),
            TypeDescriptor::VectorShape { elements, rest } => {
                elements
                    .iter()
                    .all(|element| self.names_are_linked(element))
                    && rest.as_ref().is_none_or(|rest| self.names_are_linked(rest))
            }
            TypeDescriptor::Dictionary(arguments) => {
                arguments.as_ref().is_none_or(|(key, value)| {
                    self.names_are_linked(key) && self.names_are_linked(value)
                })
            }
            TypeDescriptor::DictionaryShape { entries, rest } => {
                entries
                    .iter()
                    .all(|(_, value)| self.names_are_linked(value))
                    && rest.as_ref().is_none_or(|(key, value)| {
                        self.names_are_linked(key) && self.names_are_linked(value)
                    })
            }
            TypeDescriptor::Callable(signature) => signature.as_ref().is_none_or(|signature| {
                signature
                    .parameters
                    .iter()
                    .all(|parameter| self.names_are_linked(&parameter.r#type))
                    && self.names_are_linked(&signature.return_type)
            }),
            TypeDescriptor::Classname(inner) | TypeDescriptor::Negated(inner) => {
                self.names_are_linked(inner)
            }
            TypeDescriptor::Tuple(members)
            | TypeDescriptor::Union(members)
            | TypeDescriptor::Intersection(members) => {
                members.iter().all(|member| self.names_are_linked(member))
            }
            TypeDescriptor::TupleRest { elements, rest } => {
                elements
                    .iter()
                    .all(|element| self.names_are_linked(element))
                    && self.names_are_linked(rest)
            }
            _ => true,
        }
    }
}

fn unit_descriptors(unit: &CompiledUnit) -> Vec<&TypeDescriptor> {
    let mut descriptors = Vec::new();
    collect_chunk_descriptors(&unit.main, &mut descriptors);
    for function in &unit.functions {
        collect_function_descriptors(function, &mut descriptors);
    }

    for alias in &unit.type_aliases {
        descriptors.push(&alias.descriptor);
        collect_parameter_descriptors(&alias.type_parameters, &mut descriptors);
    }

    for newtype in &unit.newtypes {
        descriptors.push(&newtype.backing);
        collect_parameter_descriptors(&newtype.type_parameters, &mut descriptors);
    }

    for class in &unit.classes {
        collect_parameter_descriptors(&class.type_parameters, &mut descriptors);
        if let Some(parent) = &class.parent
            && let Some(arguments) = &parent.type_arguments
        {
            descriptors.extend(arguments);
        }

        for interface in &class.interfaces {
            if let Some(arguments) = &interface.type_arguments {
                descriptors.extend(arguments);
            }
        }

        for constant in &class.constants {
            descriptors.extend(constant.declared_type.iter());
        }

        for property in &class.properties {
            descriptors.extend(property.declared_type.iter());
        }

        for method in &class.methods {
            collect_function_descriptors(&method.function, &mut descriptors);
        }
    }

    descriptors
}

fn collect_function_descriptors<'a>(
    function: &'a CompiledFunction,
    descriptors: &mut Vec<&'a TypeDescriptor>,
) {
    collect_parameter_descriptors(&function.type_parameters, descriptors);
    for parameter in &function.parameters {
        descriptors.extend(parameter.declared_type.iter());
    }

    descriptors.extend(function.return_type.iter());
    collect_chunk_descriptors(&function.chunk, descriptors);
}

fn collect_parameter_descriptors<'a>(
    parameters: &'a [CompiledTypeParameter],
    descriptors: &mut Vec<&'a TypeDescriptor>,
) {
    for parameter in parameters {
        descriptors.extend(&parameter.bounds);
        descriptors.extend(parameter.default.iter());
    }
}

fn collect_chunk_descriptors<'a>(chunk: &'a Chunk, descriptors: &mut Vec<&'a TypeDescriptor>) {
    descriptors.extend(&chunk.type_descriptors);
}

fn contains_late_type(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Parameter(_) | TypeDescriptor::StaticClass => true,
        TypeDescriptor::Named { arguments, .. } => arguments
            .as_ref()
            .is_some_and(|arguments| arguments.iter().any(contains_late_type)),
        TypeDescriptor::Member {
            class_arguments,
            member_arguments,
            ..
        } => {
            class_arguments
                .as_ref()
                .is_some_and(|arguments| arguments.iter().any(contains_late_type))
                || member_arguments
                    .as_ref()
                    .is_some_and(|arguments| arguments.iter().any(contains_late_type))
        }
        TypeDescriptor::Array(arguments) => arguments
            .as_ref()
            .is_some_and(|(key, value)| contains_late_type(key) || contains_late_type(value)),
        TypeDescriptor::Vector(element) => element
            .as_ref()
            .is_some_and(|element| contains_late_type(element)),
        TypeDescriptor::VectorShape { elements, rest } => {
            elements.iter().any(contains_late_type)
                || rest.as_ref().is_some_and(|rest| contains_late_type(rest))
        }
        TypeDescriptor::Dictionary(arguments) => arguments
            .as_ref()
            .is_some_and(|(key, value)| contains_late_type(key) || contains_late_type(value)),
        TypeDescriptor::DictionaryShape { entries, rest } => {
            entries.iter().any(|(_, value)| contains_late_type(value))
                || rest.as_ref().is_some_and(|(key, value)| {
                    contains_late_type(key) || contains_late_type(value)
                })
        }
        TypeDescriptor::Callable(signature) => signature.as_ref().is_some_and(|signature| {
            signature
                .parameters
                .iter()
                .any(|parameter| contains_late_type(&parameter.r#type))
                || contains_late_type(&signature.return_type)
        }),
        TypeDescriptor::Classname(inner) | TypeDescriptor::Negated(inner) => {
            contains_late_type(inner)
        }
        TypeDescriptor::Tuple(members)
        | TypeDescriptor::Union(members)
        | TypeDescriptor::Intersection(members) => members.iter().any(contains_late_type),
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.iter().any(contains_late_type) || contains_late_type(rest)
        }
        _ => false,
    }
}
