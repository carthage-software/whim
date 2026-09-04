//! Reified type environments: interning, binding, substitution, and the
//! descriptor hashing and equality they key on.

use std::rc::Rc;

use crate::bytecode::aliases::expand_aliases;
use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::limits::MAX_TYPE_DEPTH_U32;
use crate::vm::types::Atom;
use crate::vm::types::BuildHasher;
use crate::vm::types::ClassId;
use crate::vm::types::CompiledTypeParameter;
use crate::vm::types::FixedState;
use crate::vm::types::FunctionTypeDescriptor;
use crate::vm::types::FunctionTypeParameterDescriptor;
use crate::vm::types::Hash;
use crate::vm::types::Hasher;
use crate::vm::types::RuntimeTypeEnvironment;
use crate::vm::types::SymbolKind;
use crate::vm::types::TypeDescriptor;
use crate::vm::types::TypeEnvironmentId;
use crate::vm::types::VirtualMachine;
use crate::vm::types::VirtualMachineControl;
use crate::vm::types::discriminant;

impl VirtualMachine<'_> {
    /// Resolves the innermost binding by walking immutable environment
    /// parents. Canonical environments share those parents, so lookup depth
    /// is the number of active binders rather than the number of calls that
    /// reused them.
    pub(crate) fn type_environment_binding<'a>(
        &'a self,
        mut environment: TypeEnvironmentId,
        name: &Atom,
    ) -> Option<&'a TypeDescriptor> {
        loop {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let entry = unsafe {
                self.engine
                    .tables
                    .type_environments
                    .get_unchecked(environment.0 as usize)
            };
            if let Some((candidate, descriptor)) = &entry.binding
                && candidate == name
            {
                return Some(descriptor);
            }
            environment = entry.parent?;
        }
    }

    fn intern_type_environment(
        &mut self,
        parent: TypeEnvironmentId,
        name: &Atom,
        descriptor: &TypeDescriptor,
    ) -> TypeEnvironmentId {
        let hash = type_environment_hash(parent, name, descriptor);
        if let Some(candidates) = self.engine.tables.type_environment_cache.get(&hash) {
            for candidate in candidates {
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                let environment = unsafe {
                    self.engine
                        .tables
                        .type_environments
                        .get_unchecked(candidate.0 as usize)
                };
                if environment.parent == Some(parent)
                    && environment.binding.as_ref().is_some_and(
                        |(candidate_name, candidate_descriptor)| {
                            candidate_name == name
                                && descriptor_same(candidate_descriptor, descriptor)
                        },
                    )
                {
                    return *candidate;
                }
            }
        }

        let environment = TypeEnvironmentId(self.engine.tables.type_environments.len() as u32);
        self.engine
            .tables
            .type_environments
            .push(RuntimeTypeEnvironment {
                parent: Some(parent),
                binding: Some((name.clone(), descriptor.clone())),
            });
        self.engine
            .tables
            .type_environment_cache
            .entry(hash)
            .or_default()
            .push(environment);
        environment
    }

    pub(crate) fn intern_type_descriptor(
        &mut self,
        descriptor: TypeDescriptor,
    ) -> TypeEnvironmentId {
        self.intern_type_descriptor_ref(&descriptor)
    }

    pub(crate) fn intern_type_descriptor_ref(
        &mut self,
        descriptor: &TypeDescriptor,
    ) -> TypeEnvironmentId {
        let name = self.engine.tables.type_id_atom.clone();
        self.intern_type_environment(TypeEnvironmentId::default(), &name, descriptor)
    }

    /// Binds a declaration's type parameters over an outer lexical or class
    /// environment, supplying trailing defaults and enforcing every bound.
    pub(in crate::vm) fn bind_type_parameters(
        &mut self,
        parameters: &[CompiledTypeParameter],
        supplied: Option<&[TypeDescriptor]>,
        outer: TypeEnvironmentId,
        subject: &[u8],
    ) -> Result<TypeEnvironmentId, VirtualMachineControl> {
        self.bind_type_parameters_from(parameters, supplied, outer, outer, subject)
    }

    /// Binds arguments resolved in a caller environment.
    pub(in crate::vm) fn bind_type_parameters_from(
        &mut self,
        parameters: &[CompiledTypeParameter],
        supplied: Option<&[TypeDescriptor]>,
        argument_environment: TypeEnvironmentId,
        outer: TypeEnvironmentId,
        subject: &[u8],
    ) -> Result<TypeEnvironmentId, VirtualMachineControl> {
        let supplied_count = supplied.map_or(0, <[TypeDescriptor]>::len);
        if parameters.is_empty() {
            if supplied_count != 0 {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "{} is not generic and takes no type arguments",
                        String::from_utf8_lossy(subject)
                    ),
                ));
            }
            return Ok(outer);
        }
        if supplied_count > parameters.len() {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!(
                    "{} expects at most {} type argument(s), {supplied_count} provided",
                    String::from_utf8_lossy(subject),
                    parameters.len()
                ),
            ));
        }
        let mut current = outer;
        let mut arguments = Vec::with_capacity(parameters.len());
        for (index, parameter) in parameters.iter().enumerate() {
            let argument = if let Some(argument) = supplied.and_then(|values| values.get(index)) {
                self.canonical_type_argument(argument, argument_environment)?
            } else if let Some(default) = &parameter.default {
                self.canonical_type_argument(default, current)?
            } else {
                let required = parameters
                    .iter()
                    .filter(|parameter| parameter.default.is_none())
                    .count();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    if required == parameters.len() {
                        format!(
                            "{} expects exactly {required} type argument(s), {supplied_count} provided",
                            String::from_utf8_lossy(subject)
                        )
                    } else {
                        format!(
                            "{} expects {required} to {} type argument(s), {supplied_count} provided",
                            String::from_utf8_lossy(subject),
                            parameters.len()
                        )
                    },
                ));
            };
            current = self.intern_type_environment(current, &parameter.name, &argument);
            arguments.push(argument);
        }

        for (parameter, argument) in parameters.iter().zip(&arguments) {
            for bound in &parameter.bounds {
                let bound = self.substitute_descriptor(bound, current, 0);
                if !self.descriptor_is_subtype(argument, &bound, current, 0)? {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.type_error,
                        format!(
                            "type argument {} for {} does not satisfy bound {}",
                            self.render_descriptor(argument),
                            parameter.name.to_string_lossy(),
                            self.render_descriptor(&bound)
                        ),
                    ));
                }
            }
        }
        Ok(current)
    }

    fn canonical_type_argument(
        &mut self,
        descriptor: &TypeDescriptor,
        environment: TypeEnvironmentId,
    ) -> Result<TypeDescriptor, VirtualMachineControl> {
        if self.engine.autoloader.is_some() {
            self.autoload_type_aliases(descriptor)?;
        }
        let descriptor = if self.engine.tables.type_aliases.is_empty() {
            descriptor.clone()
        } else {
            expand_aliases(descriptor, &self.engine.tables.type_aliases)
        };
        Ok(self.substitute_descriptor(&descriptor, environment, 0))
    }

    /// Loads every alias reachable from a concrete reified argument.
    fn autoload_type_aliases(
        &mut self,
        descriptor: &TypeDescriptor,
    ) -> Result<(), VirtualMachineControl> {
        let mut pending = vec![descriptor.clone()];
        let mut seen = Vec::new();

        while let Some(descriptor) = pending.pop() {
            match descriptor {
                TypeDescriptor::Named {
                    name, arguments, ..
                } => {
                    pending.extend(arguments.unwrap_or_default());
                    if seen.contains(&name) {
                        continue;
                    }
                    seen.push(name.clone());

                    let entry = match self.engine.tables.symbols.get(&name).copied() {
                        Some(entry) => Some(entry),
                        None => {
                            self.run_autoload_chain(SymbolKind::TypeAlias, name.clone())?;
                            self.engine.tables.symbols.get(&name).copied()
                        }
                    };
                    if let Some(entry) = entry
                        && entry.kind == SymbolKind::TypeAlias
                    {
                        let alias = &self.engine.tables.type_aliases[entry.index as usize];
                        pending.push(alias.descriptor.clone());
                        for parameter in &alias.type_parameters {
                            pending.extend(parameter.bounds.iter().cloned());
                            pending.extend(parameter.default.iter().cloned());
                        }
                    }
                }
                TypeDescriptor::Member {
                    class_arguments,
                    member_arguments,
                    ..
                } => {
                    pending.extend(class_arguments.unwrap_or_default());
                    pending.extend(member_arguments.unwrap_or_default());
                }
                TypeDescriptor::Array(Some((key, value)))
                | TypeDescriptor::Dictionary(Some((key, value))) => {
                    pending.push(*key);
                    pending.push(*value);
                }
                TypeDescriptor::Vector(Some(element))
                | TypeDescriptor::Classname(element)
                | TypeDescriptor::Negated(element) => pending.push(*element),
                TypeDescriptor::VectorShape { elements, rest } => {
                    pending.extend(elements);
                    pending.extend(rest.map(|rest| *rest));
                }
                TypeDescriptor::DictionaryShape { entries, rest } => {
                    pending.extend(entries.into_iter().map(|(_, value)| value));
                    if let Some((key, value)) = rest {
                        pending.push(*key);
                        pending.push(*value);
                    }
                }
                TypeDescriptor::Callable(Some(signature)) => {
                    pending.extend(
                        signature
                            .parameters
                            .into_iter()
                            .map(|parameter| parameter.r#type),
                    );
                    pending.push(*signature.return_type);
                }
                TypeDescriptor::Tuple(elements)
                | TypeDescriptor::Union(elements)
                | TypeDescriptor::Intersection(elements) => pending.extend(elements),
                TypeDescriptor::TupleRest { elements, rest } => {
                    pending.extend(elements);
                    pending.push(*rest);
                }
                TypeDescriptor::Wildcard
                | TypeDescriptor::Mixed
                | TypeDescriptor::Void
                | TypeDescriptor::Never
                | TypeDescriptor::Null
                | TypeDescriptor::Bool
                | TypeDescriptor::Int
                | TypeDescriptor::Float
                | TypeDescriptor::String
                | TypeDescriptor::StringLength { .. }
                | TypeDescriptor::Object
                | TypeDescriptor::TrueLiteral
                | TypeDescriptor::FalseLiteral
                | TypeDescriptor::IntLiteral(_)
                | TypeDescriptor::IntRange { .. }
                | TypeDescriptor::FloatLiteral(_)
                | TypeDescriptor::StringLiteral(_)
                | TypeDescriptor::Parameter(_)
                | TypeDescriptor::StaticClass
                | TypeDescriptor::Array(None)
                | TypeDescriptor::Vector(None)
                | TypeDescriptor::Dictionary(None)
                | TypeDescriptor::Callable(None)
                | TypeDescriptor::TupleAny => {}
            }
        }

        Ok(())
    }

    /// Replaces every parameter leaf with its concrete binding.
    pub(crate) fn substitute_descriptor(
        &self,
        descriptor: &TypeDescriptor,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> TypeDescriptor {
        if depth > MAX_TYPE_DEPTH_U32 {
            return descriptor.clone();
        }
        match descriptor {
            TypeDescriptor::Parameter(name) => self
                .type_environment_binding(environment, name)
                .cloned()
                .unwrap_or_else(|| descriptor.clone()),
            TypeDescriptor::Named {
                name,
                arguments,
                recursive,
            } => TypeDescriptor::Named {
                name: name.clone(),
                arguments: arguments.as_ref().map(|arguments| {
                    arguments
                        .iter()
                        .map(|argument| {
                            self.substitute_descriptor(argument, environment, depth + 1)
                        })
                        .collect()
                }),
                recursive: *recursive,
            },
            TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => TypeDescriptor::Member {
                class: class.clone(),
                class_arguments: class_arguments.as_ref().map(|arguments| {
                    arguments
                        .iter()
                        .map(|argument| {
                            self.substitute_descriptor(argument, environment, depth + 1)
                        })
                        .collect()
                }),
                member: member.clone(),
                member_arguments: member_arguments.as_ref().map(|arguments| {
                    arguments
                        .iter()
                        .map(|argument| {
                            self.substitute_descriptor(argument, environment, depth + 1)
                        })
                        .collect()
                }),
            },
            TypeDescriptor::Array(arguments) => {
                TypeDescriptor::Array(arguments.as_ref().map(|(key, value)| {
                    (
                        Box::new(self.substitute_descriptor(key, environment, depth + 1)),
                        Box::new(self.substitute_descriptor(value, environment, depth + 1)),
                    )
                }))
            }
            TypeDescriptor::Vector(element) => {
                TypeDescriptor::Vector(element.as_ref().map(|element| {
                    Box::new(self.substitute_descriptor(element, environment, depth + 1))
                }))
            }
            TypeDescriptor::Dictionary(arguments) => {
                TypeDescriptor::Dictionary(arguments.as_ref().map(|(key, value)| {
                    (
                        Box::new(self.substitute_descriptor(key, environment, depth + 1)),
                        Box::new(self.substitute_descriptor(value, environment, depth + 1)),
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
                                r#type: self.substitute_descriptor(
                                    &parameter.r#type,
                                    environment,
                                    depth + 1,
                                ),
                                optional: parameter.optional,
                            })
                            .collect(),
                        return_type: Box::new(self.substitute_descriptor(
                            &signature.return_type,
                            environment,
                            depth + 1,
                        )),
                    }
                }))
            }
            TypeDescriptor::Classname(inner) => TypeDescriptor::Classname(Box::new(
                self.substitute_descriptor(inner, environment, depth + 1),
            )),
            TypeDescriptor::Negated(inner) => TypeDescriptor::Negated(Box::new(
                self.substitute_descriptor(inner, environment, depth + 1),
            )),
            TypeDescriptor::Tuple(members) => TypeDescriptor::Tuple(
                members
                    .iter()
                    .map(|member| self.substitute_descriptor(member, environment, depth + 1))
                    .collect(),
            ),
            TypeDescriptor::TupleRest { elements, rest } => TypeDescriptor::TupleRest {
                elements: elements
                    .iter()
                    .map(|element| self.substitute_descriptor(element, environment, depth + 1))
                    .collect(),
                rest: Box::new(self.substitute_descriptor(rest, environment, depth + 1)),
            },
            TypeDescriptor::TupleAny => TypeDescriptor::TupleAny,
            TypeDescriptor::Union(members) => TypeDescriptor::Union(
                members
                    .iter()
                    .map(|member| self.substitute_descriptor(member, environment, depth + 1))
                    .collect(),
            ),
            TypeDescriptor::Intersection(members) => TypeDescriptor::intersection(
                members
                    .iter()
                    .map(|member| self.substitute_descriptor(member, environment, depth + 1))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Resolves an instance's concrete class environment at one ancestor in
    /// its declared inheritance graph, substituting each `extends` or
    /// `implements` argument list on the way up.
    pub(crate) fn environment_for_class(
        &mut self,
        class: ClassId,
        environment: TypeEnvironmentId,
        target: ClassId,
        depth: u32,
    ) -> Result<Option<TypeEnvironmentId>, VirtualMachineControl> {
        if class == target {
            return Ok(Some(environment));
        }
        if depth > MAX_TYPE_DEPTH_U32 {
            return Ok(None);
        }
        let memo = (class, environment, target);
        if let Some(found) = self.engine.tables.base_environment_cache.get(&memo) {
            return Ok(*found);
        }

        let specialization = self.engine.tables.classes[class.0 as usize]
            .base_specializations
            .get(&target)
            .cloned();
        if let Some(arguments) = specialization {
            let (parameters, name) = {
                let entry = &self.engine.tables.classes[target.0 as usize];
                (Rc::clone(&entry.type_parameters), entry.name.clone())
            };
            let projected = self.bind_type_parameters(
                &parameters,
                Some(&arguments),
                environment,
                name.as_bytes(),
            )?;
            self.engine
                .tables
                .base_environment_cache
                .insert(memo, Some(projected));
            return Ok(Some(projected));
        }

        let bases = self.engine.tables.classes[class.0 as usize]
            .direct_bases
            .clone();
        for base in bases {
            let (parameters, name) = {
                let entry = &self.engine.tables.classes[base.class.0 as usize];
                (Rc::clone(&entry.type_parameters), entry.name.clone())
            };
            let base_environment = self.bind_type_parameters(
                &parameters,
                base.type_arguments.as_deref(),
                environment,
                name.as_bytes(),
            )?;
            if let Some(found) =
                self.environment_for_class(base.class, base_environment, target, depth + 1)?
            {
                self.engine
                    .tables
                    .base_environment_cache
                    .insert(memo, Some(found));
                return Ok(Some(found));
            }
        }
        self.engine.tables.base_environment_cache.insert(memo, None);
        Ok(None)
    }
}

fn type_environment_hash(
    parent: TypeEnvironmentId,
    name: &Atom,
    descriptor: &TypeDescriptor,
) -> u64 {
    let mut state = DESCRIPTOR_HASHER.build_hasher();
    parent.hash(&mut state);
    name.hash(&mut state);
    hash_descriptor(descriptor, &mut state);
    state.finish()
}

/// The hasher the environment cache keys on.
static DESCRIPTOR_HASHER: FixedState = FixedState::with_seed(0x9e37_79b9_7f4a_7c15);

fn hash_descriptor(descriptor: &TypeDescriptor, state: &mut impl Hasher) {
    discriminant(descriptor).hash(state);
    match descriptor {
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::Object
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::StaticClass
        | TypeDescriptor::TupleAny => {}
        TypeDescriptor::IntLiteral(value) => value.hash(state),
        TypeDescriptor::IntRange { min, max } => {
            min.hash(state);
            max.hash(state);
        }
        TypeDescriptor::StringLength { min, max } => {
            min.hash(state);
            max.hash(state);
        }
        TypeDescriptor::FloatLiteral(value) => value.to_bits().hash(state),
        TypeDescriptor::StringLiteral(value) | TypeDescriptor::Parameter(value) => {
            value.hash(state);
        }
        TypeDescriptor::Named {
            name, arguments, ..
        } => {
            name.hash(state);
            arguments.is_some().hash(state);
            if let Some(arguments) = arguments {
                hash_descriptors(arguments, state);
            }
        }
        TypeDescriptor::Member {
            class,
            class_arguments,
            member,
            member_arguments,
        } => {
            class.hash(state);
            class_arguments.is_some().hash(state);
            if let Some(arguments) = class_arguments {
                hash_descriptors(arguments, state);
            }
            member.hash(state);
            member_arguments.is_some().hash(state);
            if let Some(arguments) = member_arguments {
                hash_descriptors(arguments, state);
            }
        }
        TypeDescriptor::Array(arguments) | TypeDescriptor::Dictionary(arguments) => {
            arguments.is_some().hash(state);
            if let Some((key, value)) = arguments {
                hash_descriptor(key, state);
                hash_descriptor(value, state);
            }
        }
        TypeDescriptor::Vector(element) => {
            element.is_some().hash(state);
            if let Some(element) = element {
                hash_descriptor(element, state);
            }
        }
        TypeDescriptor::VectorShape { elements, rest } => {
            elements.len().hash(state);
            for element in elements {
                hash_descriptor(element, state);
            }
            rest.is_some().hash(state);
            if let Some(rest) = rest {
                hash_descriptor(rest, state);
            }
        }
        TypeDescriptor::DictionaryShape { entries, rest } => {
            entries.len().hash(state);
            for (key, value) in entries {
                match key {
                    ShapeKey::Int(key) => {
                        0u8.hash(state);
                        key.hash(state);
                    }
                    ShapeKey::String(key) => {
                        1u8.hash(state);
                        key.hash(state);
                    }
                }
                hash_descriptor(value, state);
            }
            rest.is_some().hash(state);
            if let Some((key, value)) = rest {
                hash_descriptor(key, state);
                hash_descriptor(value, state);
            }
        }
        TypeDescriptor::Callable(signature) => {
            signature.is_some().hash(state);
            if let Some(signature) = signature {
                signature.parameters.len().hash(state);
                for parameter in &signature.parameters {
                    parameter.optional.hash(state);
                    hash_descriptor(&parameter.r#type, state);
                }
                hash_descriptor(&signature.return_type, state);
            }
        }
        TypeDescriptor::Classname(inner) | TypeDescriptor::Negated(inner) => {
            hash_descriptor(inner, state);
        }
        TypeDescriptor::Tuple(members)
        | TypeDescriptor::Union(members)
        | TypeDescriptor::Intersection(members) => hash_descriptors(members, state),
        TypeDescriptor::TupleRest { elements, rest } => {
            hash_descriptors(elements, state);
            hash_descriptor(rest, state);
        }
    }
}

fn hash_descriptors(descriptors: &[TypeDescriptor], state: &mut impl Hasher) {
    descriptors.len().hash(state);
    for descriptor in descriptors {
        hash_descriptor(descriptor, state);
    }
}

pub(crate) fn descriptor_same(left: &TypeDescriptor, right: &TypeDescriptor) -> bool {
    match (left, right) {
        (TypeDescriptor::Wildcard, TypeDescriptor::Wildcard)
        | (TypeDescriptor::Mixed, TypeDescriptor::Mixed)
        | (TypeDescriptor::Void, TypeDescriptor::Void)
        | (TypeDescriptor::Never, TypeDescriptor::Never)
        | (TypeDescriptor::Null, TypeDescriptor::Null)
        | (TypeDescriptor::Bool, TypeDescriptor::Bool)
        | (TypeDescriptor::Int, TypeDescriptor::Int)
        | (TypeDescriptor::Float, TypeDescriptor::Float)
        | (TypeDescriptor::String, TypeDescriptor::String)
        | (TypeDescriptor::Object, TypeDescriptor::Object)
        | (TypeDescriptor::TrueLiteral, TypeDescriptor::TrueLiteral)
        | (TypeDescriptor::FalseLiteral, TypeDescriptor::FalseLiteral)
        | (TypeDescriptor::StaticClass, TypeDescriptor::StaticClass)
        | (TypeDescriptor::TupleAny, TypeDescriptor::TupleAny) => true,
        (TypeDescriptor::IntLiteral(left), TypeDescriptor::IntLiteral(right)) => left == right,
        (
            TypeDescriptor::StringLength {
                min: left_min,
                max: left_max,
            },
            TypeDescriptor::StringLength {
                min: right_min,
                max: right_max,
            },
        ) => left_min == right_min && left_max == right_max,
        (
            TypeDescriptor::IntRange {
                min: left_min,
                max: left_max,
            },
            TypeDescriptor::IntRange {
                min: right_min,
                max: right_max,
            },
        ) => left_min == right_min && left_max == right_max,
        (TypeDescriptor::FloatLiteral(left), TypeDescriptor::FloatLiteral(right)) => {
            left.to_bits() == right.to_bits()
        }
        (TypeDescriptor::StringLiteral(left), TypeDescriptor::StringLiteral(right))
        | (TypeDescriptor::Parameter(left), TypeDescriptor::Parameter(right)) => left == right,
        (
            TypeDescriptor::Named {
                name: left_name,
                arguments: left_arguments,
                ..
            },
            TypeDescriptor::Named {
                name: right_name,
                arguments: right_arguments,
                ..
            },
        ) => {
            left_name == right_name
                && optional_descriptors_same(left_arguments.as_deref(), right_arguments.as_deref())
        }
        (
            TypeDescriptor::Member {
                class: left_class,
                class_arguments: left_class_arguments,
                member: left_member,
                member_arguments: left_member_arguments,
            },
            TypeDescriptor::Member {
                class: right_class,
                class_arguments: right_class_arguments,
                member: right_member,
                member_arguments: right_member_arguments,
            },
        ) => {
            left_class == right_class
                && optional_descriptors_same(
                    left_class_arguments.as_deref(),
                    right_class_arguments.as_deref(),
                )
                && left_member == right_member
                && optional_descriptors_same(
                    left_member_arguments.as_deref(),
                    right_member_arguments.as_deref(),
                )
        }
        (TypeDescriptor::Vector(left), TypeDescriptor::Vector(right)) => {
            optional_box_same(left.as_deref(), right.as_deref())
        }
        (TypeDescriptor::Array(left), TypeDescriptor::Array(right)) => match (left, right) {
            (None, None) => true,
            (Some((left_key, left_value)), Some((right_key, right_value))) => {
                descriptor_same(left_key, right_key) && descriptor_same(left_value, right_value)
            }
            _ => false,
        },
        (
            TypeDescriptor::VectorShape {
                elements: left_elements,
                rest: left_rest,
            },
            TypeDescriptor::VectorShape {
                elements: right_elements,
                rest: right_rest,
            },
        ) => {
            descriptors_same(left_elements, right_elements)
                && optional_box_same(left_rest.as_deref(), right_rest.as_deref())
        }
        (TypeDescriptor::Dictionary(left), TypeDescriptor::Dictionary(right)) => {
            match (left, right) {
                (None, None) => true,
                (Some((left_key, left_value)), Some((right_key, right_value))) => {
                    descriptor_same(left_key, right_key) && descriptor_same(left_value, right_value)
                }
                _ => false,
            }
        }
        (
            TypeDescriptor::DictionaryShape {
                entries: left_entries,
                rest: left_rest,
            },
            TypeDescriptor::DictionaryShape {
                entries: right_entries,
                rest: right_rest,
            },
        ) => {
            left_entries.len() == right_entries.len()
                && left_entries.iter().zip(right_entries).all(
                    |((left_key, left_value), (right_key, right_value))| {
                        shape_keys_same(left_key, right_key)
                            && descriptor_same(left_value, right_value)
                    },
                )
                && match (left_rest, right_rest) {
                    (None, None) => true,
                    (Some((left_key, left_value)), Some((right_key, right_value))) => {
                        descriptor_same(left_key, right_key)
                            && descriptor_same(left_value, right_value)
                    }
                    _ => false,
                }
        }
        (TypeDescriptor::Callable(left), TypeDescriptor::Callable(right)) => match (left, right) {
            (None, None) => true,
            (Some(left), Some(right)) => {
                left.parameters.len() == right.parameters.len()
                    && left
                        .parameters
                        .iter()
                        .zip(&right.parameters)
                        .all(|(left, right)| {
                            left.optional == right.optional
                                && descriptor_same(&left.r#type, &right.r#type)
                        })
                    && descriptor_same(&left.return_type, &right.return_type)
            }
            _ => false,
        },
        (TypeDescriptor::Classname(left), TypeDescriptor::Classname(right))
        | (TypeDescriptor::Negated(left), TypeDescriptor::Negated(right)) => {
            descriptor_same(left, right)
        }
        (TypeDescriptor::Tuple(left), TypeDescriptor::Tuple(right))
        | (TypeDescriptor::Union(left), TypeDescriptor::Union(right))
        | (TypeDescriptor::Intersection(left), TypeDescriptor::Intersection(right)) => {
            descriptors_same(left, right)
        }
        (
            TypeDescriptor::TupleRest {
                elements: left_elements,
                rest: left_rest,
            },
            TypeDescriptor::TupleRest {
                elements: right_elements,
                rest: right_rest,
            },
        ) => {
            descriptors_same(left_elements, right_elements)
                && descriptor_same(left_rest, right_rest)
        }
        _ => false,
    }
}

fn shape_keys_same(left: &ShapeKey, right: &ShapeKey) -> bool {
    match (left, right) {
        (ShapeKey::Int(left), ShapeKey::Int(right)) => left == right,
        (ShapeKey::String(left), ShapeKey::String(right)) => left == right,
        _ => false,
    }
}

fn optional_box_same(left: Option<&TypeDescriptor>, right: Option<&TypeDescriptor>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => descriptor_same(left, right),
        _ => false,
    }
}

fn optional_descriptors_same(
    left: Option<&[TypeDescriptor]>,
    right: Option<&[TypeDescriptor]>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => descriptors_same(left, right),
        _ => false,
    }
}

fn descriptors_same(left: &[TypeDescriptor], right: &[TypeDescriptor]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| descriptor_same(left, right))
}
