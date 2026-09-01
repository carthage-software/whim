//! Runtime descriptor checks: subtyping, value conformance, and
//! callable compatibility.

use std::rc::Rc;

use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::check_trivial_descriptor;
use crate::bytecode::unit::Visibility;
use crate::classes::ClassMemberEntry;
use crate::classes::MethodBodyKind;
use crate::engine::builtins::built_in_type_parameters;
use crate::limits::MAX_TYPE_DEPTH_U32;
use crate::symbols::FunctionTable;
use crate::symbols::SymbolEntry;
use crate::value::collection::CollectionTypeCheck;
use crate::value::collection::CollectionTypeCheckId;
use crate::value::dict::DictObject;
use crate::value::function::BuiltInId;
use crate::value::function::CallTarget;
use crate::value::function::FuncId;
use crate::value::function::PresetArg;
use crate::value::heap::handle::ManagedRef;
use crate::value::newtype::NewtypeId;
use crate::value::ops;
use crate::value::string::ByteStringObject;
use crate::vm::types::Chunk;
use crate::vm::types::ClassId;
use crate::vm::types::CompiledTypeParameter;
use crate::vm::types::DescriptorIndex;
use crate::vm::types::FunctionObject;
use crate::vm::types::FunctionTypeDescriptor;
use crate::vm::types::Key;
use crate::vm::types::KeyRef;
use crate::vm::types::SymbolKind;
use crate::vm::types::TypeDescriptor;
use crate::vm::types::TypeEnvironmentId;
use crate::vm::types::Value;
use crate::vm::types::Variance;
use crate::vm::types::VirtualMachine;
use crate::vm::types::VirtualMachineControl;
use crate::vm::types::environment::descriptor_same;
use crate::vm::types::is_instance_of;
use crate::vm::types::unreachable_invariant;

fn is_complete_callable_binding(function: &FunctionObject, parameter_count: usize) -> bool {
    function.presets().len() == parameter_count
        && function
            .presets()
            .iter()
            .enumerate()
            .all(|(position, preset)| {
                matches!(preset, PresetArg::Hole(order) if *order == position as u32)
            })
}

enum ExactCallableShape {
    Family,
    Concrete(FunctionTypeDescriptor),
}

fn key_ref_value(key: KeyRef<'_>) -> Value {
    match key {
        KeyRef::Int(key) => Value::int(key),
        KeyRef::Bool(key) => Value::bool(key),
        KeyRef::String(key) => Value::string(key.clone()),
        KeyRef::ShortString(key) => Value::short_string(key),
    }
}

fn key_value(key: Key) -> Value {
    match key {
        Key::Int(key) => Value::int(key),
        Key::Bool(key) => Value::bool(key),
        Key::String(key) => Value::string(key),
        Key::ShortString(key) => Value::short_string(key),
    }
}

impl VirtualMachine<'_> {
    fn exact_type_arguments_are_subtype(
        &mut self,
        actual: Option<&[TypeDescriptor]>,
        expected: Option<&[TypeDescriptor]>,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        let Some(expected) = expected else {
            return Ok(true);
        };
        let Some(actual) = actual else {
            return Ok(false);
        };
        if actual.len() != expected.len() {
            return Ok(false);
        }
        for (actual, expected) in actual.iter().zip(expected) {
            if !self.descriptor_is_subtype(actual, expected, environment, depth + 1)?
                || !self.descriptor_is_subtype(expected, actual, environment, depth + 1)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn exact_function_family_is_subtype(
        &mut self,
        actual: SymbolEntry,
        actual_arguments: Option<&[TypeDescriptor]>,
        expected: SymbolEntry,
        expected_arguments: Option<&[TypeDescriptor]>,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        if actual.kind != SymbolKind::Function
            || expected.kind != SymbolKind::Function
            || actual.table != expected.table
            || actual.index != expected.index
        {
            return Ok(false);
        }
        let Some(expected_arguments) = expected_arguments else {
            return Ok(true);
        };
        let Some(actual_arguments) = actual_arguments else {
            return Ok(false);
        };
        let (parameters, subject) = match actual.table {
            FunctionTable::User => {
                let function = &self.engine.tables.functions[actual.index as usize];
                (function.type_parameters().to_vec(), function.name.clone())
            }
            FunctionTable::BuiltIn => {
                let callable = self.engine.tables.built_in_functions[actual.index as usize].clone();
                (
                    built_in_type_parameters(&self.heap, callable.type_parameters()),
                    self.heap.intern(callable.display_name().as_bytes()),
                )
            }
        };
        let actual_environment = self.bind_type_parameters(
            &parameters,
            Some(actual_arguments),
            environment,
            subject.as_bytes(),
        )?;
        let expected_environment = self.bind_type_parameters(
            &parameters,
            Some(expected_arguments),
            environment,
            subject.as_bytes(),
        )?;
        self.generic_environments_compatible(
            &parameters,
            actual_environment,
            expected_environment,
            depth + 1,
        )
    }

    fn exact_method_family_is_subtype(
        &mut self,
        actual: &TypeDescriptor,
        expected: &TypeDescriptor,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        let TypeDescriptor::Member {
            class: actual_class,
            class_arguments: actual_class_arguments,
            member: actual_member,
            member_arguments: actual_member_arguments,
        } = actual
        else {
            return Ok(false);
        };
        let TypeDescriptor::Member {
            class: expected_class,
            class_arguments: expected_class_arguments,
            member: expected_member,
            member_arguments: expected_member_arguments,
        } = expected
        else {
            return Ok(false);
        };
        if actual_member != expected_member {
            return Ok(false);
        }
        let Some(actual_symbol) = self.resolve_checked_name(actual_class.clone())? else {
            return Ok(false);
        };
        let Some(expected_symbol) = self.resolve_checked_name(expected_class.clone())? else {
            return Ok(false);
        };
        if !matches!(
            actual_symbol.kind,
            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
        ) || !matches!(
            expected_symbol.kind,
            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
        ) {
            return Ok(false);
        }
        let actual_id = ClassId(actual_symbol.index);
        let expected_id = ClassId(expected_symbol.index);
        if !is_instance_of(&self.engine.tables.classes, actual_id, expected_id) {
            return Ok(false);
        }
        let actual_method = self.engine.tables.classes[actual_id.0 as usize].method(actual_member);
        let expected_method = self.engine.tables.classes[expected_id.0 as usize]
            .method(expected_member)
            .or_else(|| {
                self.engine.tables.classes[expected_id.0 as usize]
                    .private_methods
                    .get(&(expected_id, expected_member.clone()))
                    .copied()
            });
        let (Some(actual_method), Some(expected_method)) = (actual_method, expected_method) else {
            return Ok(false);
        };
        if expected_method.visibility == Visibility::Private
            && actual_method.declaring_class != expected_method.declaring_class
        {
            return Ok(false);
        }
        if expected_class_arguments.is_some() {
            let actual_parameters =
                Rc::clone(&self.engine.tables.classes[actual_id.0 as usize].type_parameters);
            let expected_parameters =
                Rc::clone(&self.engine.tables.classes[expected_id.0 as usize].type_parameters);
            let actual_environment = self.bind_type_parameters(
                &actual_parameters,
                actual_class_arguments.as_deref(),
                environment,
                actual_class.as_bytes(),
            )?;
            let Some(projected) =
                self.environment_for_class(actual_id, actual_environment, expected_id, depth + 1)?
            else {
                return Ok(false);
            };
            let expected_environment = self.bind_type_parameters(
                &expected_parameters,
                expected_class_arguments.as_deref(),
                environment,
                expected_class.as_bytes(),
            )?;
            if !self.nominal_environments_compatible(
                expected_id,
                projected,
                expected_environment,
                depth + 1,
            )? {
                return Ok(false);
            }
        }
        self.exact_type_arguments_are_subtype(
            actual_member_arguments.as_deref(),
            expected_member_arguments.as_deref(),
            environment,
            depth + 1,
        )
    }

    fn exact_callable_shape(
        &mut self,
        descriptor: &TypeDescriptor,
        environment: TypeEnvironmentId,
    ) -> Result<Option<ExactCallableShape>, VirtualMachineControl> {
        let (target, parameters, arguments, outer, subject) = match descriptor {
            TypeDescriptor::Named {
                name, arguments, ..
            } => {
                let Some(entry) = self.resolve_checked_name(name.clone())? else {
                    return Ok(None);
                };
                if entry.kind != SymbolKind::Function {
                    return Ok(None);
                }
                match entry.table {
                    FunctionTable::User => {
                        let runtime = &self.engine.tables.functions[entry.index as usize];
                        (
                            CallTarget::User(FuncId(entry.index)),
                            runtime.type_parameters().to_vec(),
                            arguments.as_deref(),
                            environment,
                            runtime.name.clone(),
                        )
                    }
                    FunctionTable::BuiltIn => {
                        let callable =
                            self.engine.tables.built_in_functions[entry.index as usize].clone();
                        (
                            CallTarget::BuiltIn(BuiltInId(entry.index)),
                            built_in_type_parameters(&self.heap, callable.type_parameters()),
                            arguments.as_deref(),
                            environment,
                            self.heap.intern(callable.display_name().as_bytes()),
                        )
                    }
                }
            }
            TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => {
                let Some(entry) = self.resolve_checked_name(class.clone())? else {
                    return Ok(None);
                };
                if !matches!(
                    entry.kind,
                    SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                ) {
                    return Ok(None);
                }
                let owner = ClassId(entry.index);
                let (owner_parameters, owner_name) = {
                    let class = &self.engine.tables.classes[owner.0 as usize];
                    (Rc::clone(&class.type_parameters), class.name.clone())
                };
                if !owner_parameters.is_empty() && class_arguments.is_none() {
                    return Ok(Some(ExactCallableShape::Family));
                }
                let outer = self.bind_type_parameters(
                    &owner_parameters,
                    class_arguments.as_deref(),
                    environment,
                    owner_name.as_bytes(),
                )?;
                let method = self.engine.tables.classes[owner.0 as usize]
                    .method(member)
                    .or_else(|| {
                        self.engine.tables.classes[owner.0 as usize]
                            .private_methods
                            .get(&(owner, member.clone()))
                            .copied()
                    });
                let Some(method) = method else {
                    return Ok(None);
                };
                let target = match method.body {
                    MethodBodyKind::Bytecode(function) => CallTarget::User(function),
                    MethodBodyKind::BuiltIn(_) => {
                        CallTarget::BuiltIn(self.built_in_id_for_method(&method, member.clone()))
                    }
                };
                match target {
                    CallTarget::User(id) => {
                        let runtime = &self.engine.tables.functions[id.0 as usize];
                        (
                            target,
                            runtime.type_parameters().to_vec(),
                            member_arguments.as_deref(),
                            outer,
                            runtime.name.clone(),
                        )
                    }
                    CallTarget::BuiltIn(id) => {
                        let callable = self.engine.tables.built_in_functions[id.0 as usize].clone();
                        (
                            target,
                            built_in_type_parameters(&self.heap, callable.type_parameters()),
                            member_arguments.as_deref(),
                            outer,
                            self.heap.intern(callable.display_name().as_bytes()),
                        )
                    }
                }
            }
            _ => return Ok(None),
        };

        if !parameters.is_empty() && arguments.is_none() {
            return Ok(Some(ExactCallableShape::Family));
        }
        let environment =
            self.bind_type_parameters(&parameters, arguments, outer, subject.as_bytes())?;
        Ok(Some(ExactCallableShape::Concrete(
            self.call_target_type_descriptor(target, environment),
        )))
    }

    fn exact_descriptor_value(
        &mut self,
        descriptor: &TypeDescriptor,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<Option<Value>, VirtualMachineControl> {
        if depth > MAX_TYPE_DEPTH_U32 {
            return Ok(None);
        }

        let descriptor = self.substitute_descriptor(descriptor, environment, depth + 1);
        if let Some(expanded) = self.expand_type_alias_once(&descriptor, depth + 1)? {
            return self.exact_descriptor_value(&expanded, TypeEnvironmentId::default(), depth + 1);
        }

        Ok(match descriptor {
            TypeDescriptor::Null => Some(Value::null()),
            TypeDescriptor::TrueLiteral => Some(Value::bool(true)),
            TypeDescriptor::FalseLiteral => Some(Value::bool(false)),
            TypeDescriptor::IntLiteral(value) => Some(Value::int(value)),
            TypeDescriptor::FloatLiteral(value) => Some(Value::float(value)),
            TypeDescriptor::StringLiteral(value) => {
                Some(Value::from_string_bytes(&self.heap, value.as_bytes()))
            }
            TypeDescriptor::Named {
                name, arguments, ..
            } => {
                let Some(entry) = self.resolve_checked_name(name.clone())? else {
                    return Ok(None);
                };
                if entry.kind != SymbolKind::Constant || arguments.is_some() {
                    return Ok(None);
                }
                Some(self.force_constant(entry.index, name)?)
            }
            TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => {
                if member_arguments.is_some() {
                    return Ok(None);
                }
                let Some(entry) = self.resolve_checked_name(class.clone())? else {
                    return Ok(None);
                };
                if !matches!(
                    entry.kind,
                    SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                ) {
                    return Ok(None);
                }
                let class_id = ClassId(entry.index);
                if let Some(arguments) = class_arguments.as_deref() {
                    let parameters =
                        Rc::clone(&self.engine.tables.classes[class_id.0 as usize].type_parameters);
                    self.bind_type_parameters(
                        &parameters,
                        Some(arguments),
                        environment,
                        class.as_bytes(),
                    )?;
                }
                let member_entry = self.engine.tables.classes[class_id.0 as usize]
                    .members
                    .get(&member)
                    .cloned();
                match member_entry {
                    Some(ClassMemberEntry::Constant(_)) => {
                        Some(self.force_class_constant(class_id, member)?)
                    }
                    Some(ClassMemberEntry::EnumCase(_)) => {
                        self.enum_case_instance(class_id, member)
                    }
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// Whether a positive object check depends only on the object's class and
    /// reified type environment, which are the facts retained by the `is`
    /// inline cache.
    pub(in crate::vm) fn is_check_shape_cacheable(
        &self,
        descriptor: &TypeDescriptor,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> bool {
        if depth > MAX_TYPE_DEPTH_U32 {
            return false;
        }

        match descriptor {
            TypeDescriptor::Member { .. } => false,
            TypeDescriptor::Parameter(name) => self
                .type_environment_binding(environment, name)
                .is_some_and(|binding| {
                    self.is_check_shape_cacheable(binding, environment, depth + 1)
                }),
            TypeDescriptor::Named { name, .. } => {
                self.engine.tables.symbols.get(name).is_some_and(|entry| {
                    matches!(
                        entry.kind,
                        SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                    )
                })
            }
            TypeDescriptor::Union(members) | TypeDescriptor::Intersection(members) => members
                .iter()
                .all(|member| self.is_check_shape_cacheable(member, environment, depth + 1)),
            TypeDescriptor::Negated(inner) => {
                self.is_check_shape_cacheable(inner, environment, depth + 1)
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
            | TypeDescriptor::Object
            | TypeDescriptor::TrueLiteral
            | TypeDescriptor::FalseLiteral
            | TypeDescriptor::IntLiteral(_)
            | TypeDescriptor::IntRange { .. }
            | TypeDescriptor::FloatLiteral(_)
            | TypeDescriptor::StringLiteral(_)
            | TypeDescriptor::Array(_)
            | TypeDescriptor::Vector(_)
            | TypeDescriptor::VectorShape { .. }
            | TypeDescriptor::Dictionary(_)
            | TypeDescriptor::DictionaryShape { .. }
            | TypeDescriptor::Callable(_)
            | TypeDescriptor::Classname(_)
            | TypeDescriptor::Tuple(_)
            | TypeDescriptor::TupleRest { .. }
            | TypeDescriptor::TupleAny
            | TypeDescriptor::StaticClass => true,
        }
    }

    /// Performs an explicit `as`/`?as` conversion. Newtypes are the only
    /// descriptors whose cast changes a value: casting to one adds or
    /// replaces its outer nominal tag, while casting from one to an ordinary
    /// backing type removes all nominal layers.
    pub(in crate::vm) fn cast_value(
        &mut self,
        descriptor: &TypeDescriptor,
        value: &Value,
        called: Option<ClassId>,
        environment: TypeEnvironmentId,
    ) -> Result<Option<Value>, VirtualMachineControl> {
        let mut target = self.substitute_descriptor(descriptor, environment, 0);
        for _ in 0..=MAX_TYPE_DEPTH_U32 {
            let Some(expanded) = self.expand_type_alias_once(&target, 0)? else {
                break;
            };
            target = expanded;
        }

        if let TypeDescriptor::Named {
            name, arguments, ..
        } = &target
            && let Some(entry) = self.resolve_checked_name(name.clone())?
            && entry.kind == SymbolKind::Newtype
        {
            let id = NewtypeId(entry.index);
            let (parameters, backing, subject) = {
                let declaration = &self.engine.tables.newtypes[entry.index as usize];
                (
                    declaration.type_parameters.clone(),
                    declaration.backing.clone(),
                    declaration.name.clone(),
                )
            };
            let target_environment = self.bind_type_parameters(
                &parameters,
                arguments.as_deref(),
                environment,
                subject.as_bytes(),
            )?;
            if let Some(value_id) = value.newtype_id()
                && self.engine.tables.newtype_value(value_id).declaration == id
                && self.generic_environments_compatible(
                    &parameters,
                    self.engine.tables.newtype_value(value_id).type_environment,
                    target_environment,
                    0,
                )?
            {
                return if self.check_descriptor(&target, value, called, environment, 0)? {
                    Ok(Some(value.clone()))
                } else {
                    Ok(None)
                };
            }

            let candidate = value.newtype_id().map_or_else(
                || value.clone(),
                |value_id| {
                    value.clone_with_newtype(self.engine.tables.newtype_value(value_id).parent)
                },
            );
            if let Some(candidate_id) = candidate.newtype_id()
                && self.engine.tables.newtype_value(candidate_id).declaration == id
                && self.generic_environments_compatible(
                    &parameters,
                    self.engine
                        .tables
                        .newtype_value(candidate_id)
                        .type_environment,
                    target_environment,
                    0,
                )?
                && self.check_descriptor(&target, &candidate, called, environment, 0)?
            {
                return Ok(Some(candidate));
            }
            let backing = self.substitute_descriptor(&backing, target_environment, 0);
            if !self.check_descriptor(&backing, &candidate, called, target_environment, 0)? {
                return Ok(None);
            }
            let value_id = self.engine.tables.intern_newtype_value(
                id,
                target_environment,
                candidate.newtype_id(),
            );
            return Ok(Some(Value::newtype(candidate, value_id)));
        }

        let candidate = if value.newtype_id().is_some() {
            value.clone_with_newtype(None)
        } else {
            value.clone()
        };
        if self.check_descriptor(&target, &candidate, called, environment, 0)? {
            Ok(Some(candidate))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn descriptor_is_subtype(
        &mut self,
        actual: &TypeDescriptor,
        expected: &TypeDescriptor,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        if depth > MAX_TYPE_DEPTH_U32 {
            return Ok(false);
        }

        if matches!(actual, TypeDescriptor::Wildcard)
            || matches!(expected, TypeDescriptor::Wildcard)
        {
            return Ok(true);
        }

        let actual = self.substitute_descriptor(actual, environment, depth + 1);
        let expected = self.substitute_descriptor(expected, environment, depth + 1);
        if let Some(expanded) = self.expand_type_alias_once(&actual, depth + 1)? {
            return self.descriptor_is_subtype(
                &expanded,
                &expected,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if let Some(expanded) = self.expand_type_alias_once(&expected, depth + 1)? {
            return self.descriptor_is_subtype(
                &actual,
                &expanded,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }

        if descriptor_same(&actual, &expected) {
            return Ok(true);
        }
        if matches!(actual, TypeDescriptor::Never) {
            return Ok(true);
        }
        if let Some(actual_value) =
            self.exact_descriptor_value(&actual, TypeEnvironmentId::default(), depth + 1)?
        {
            if let Some(expected_value) =
                self.exact_descriptor_value(&expected, TypeEnvironmentId::default(), depth + 1)?
            {
                return Ok(ops::equals(&actual_value, &expected_value));
            }
            return self.check_descriptor(
                &expected,
                &actual_value,
                None,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if let TypeDescriptor::Callable(expected) = &expected
            && let Some(actual) =
                self.exact_callable_shape(&actual, TypeEnvironmentId::default())?
        {
            return Ok(match (actual, expected) {
                (_, None) => true,
                (ExactCallableShape::Concrete(actual), Some(expected)) => {
                    callable_descriptor_compatible(
                        self,
                        &actual,
                        expected,
                        TypeEnvironmentId::default(),
                        depth + 1,
                    )?
                }
                (ExactCallableShape::Family, Some(_)) => false,
            });
        }
        if let (
            TypeDescriptor::Named {
                name: actual_name,
                arguments: actual_arguments,
                ..
            },
            TypeDescriptor::Named {
                name: expected_name,
                arguments: expected_arguments,
                ..
            },
        ) = (&actual, &expected)
        {
            let actual_entry = self.resolve_checked_name(actual_name.clone())?;
            let expected_entry = self.resolve_checked_name(expected_name.clone())?;
            if let (Some(actual_entry), Some(expected_entry)) = (actual_entry, expected_entry)
                && (actual_entry.kind == SymbolKind::Function
                    || expected_entry.kind == SymbolKind::Function)
            {
                return self.exact_function_family_is_subtype(
                    actual_entry,
                    actual_arguments.as_deref(),
                    expected_entry,
                    expected_arguments.as_deref(),
                    environment,
                    depth + 1,
                );
            }
        }
        if matches!(actual, TypeDescriptor::Member { .. })
            && matches!(expected, TypeDescriptor::Member { .. })
            && self.exact_callable_shape(&actual, environment)?.is_some()
            && self.exact_callable_shape(&expected, environment)?.is_some()
        {
            return self.exact_method_family_is_subtype(&actual, &expected, environment, depth + 1);
        }
        if let (
            TypeDescriptor::Named {
                name: actual_name,
                arguments: actual_arguments,
                ..
            },
            TypeDescriptor::Named {
                name: expected_name,
                arguments: expected_arguments,
                ..
            },
        ) = (&actual, &expected)
            && actual_name == expected_name
            && let Some(entry) = self.resolve_checked_name(actual_name.clone())?
            && entry.kind == SymbolKind::Newtype
        {
            if expected_arguments.is_none() {
                return Ok(true);
            }
            let (parameters, subject) = {
                let declaration = &self.engine.tables.newtypes[entry.index as usize];
                (
                    declaration.type_parameters.clone(),
                    declaration.name.clone(),
                )
            };
            let actual_environment = self.bind_type_parameters(
                &parameters,
                actual_arguments.as_deref(),
                environment,
                subject.as_bytes(),
            )?;
            let expected_environment = self.bind_type_parameters(
                &parameters,
                expected_arguments.as_deref(),
                environment,
                subject.as_bytes(),
            )?;
            return self.generic_environments_compatible(
                &parameters,
                actual_environment,
                expected_environment,
                depth + 1,
            );
        }
        if let Some(backing) = self.expand_newtype_once(&actual, environment, depth + 1)? {
            return self.descriptor_is_subtype(
                &backing,
                &expected,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if matches!(expected, TypeDescriptor::Mixed) || matches!(actual, TypeDescriptor::Never) {
            return Ok(true);
        }
        if matches!(
            actual,
            TypeDescriptor::Negated(ref inner) if matches!(inner.as_ref(), TypeDescriptor::Mixed)
        ) {
            return Ok(true);
        }
        if matches!(
            expected,
            TypeDescriptor::Negated(ref inner) if matches!(inner.as_ref(), TypeDescriptor::Never)
        ) {
            return Ok(true);
        }
        if let TypeDescriptor::Negated(inner) = &actual
            && let TypeDescriptor::Negated(inner) = inner.as_ref()
        {
            return self.descriptor_is_subtype(
                inner,
                &expected,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if let TypeDescriptor::Negated(inner) = &expected
            && let TypeDescriptor::Negated(inner) = inner.as_ref()
        {
            return self.descriptor_is_subtype(
                &actual,
                inner,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if let (TypeDescriptor::Negated(actual_inner), TypeDescriptor::Negated(expected_inner)) =
            (&actual, &expected)
        {
            return self.descriptor_is_subtype(
                expected_inner,
                actual_inner,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if let TypeDescriptor::Negated(inner) = &expected {
            return self.descriptors_are_disjoint(
                &actual,
                inner,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if matches!(
            actual,
            TypeDescriptor::Negated(ref inner) if matches!(inner.as_ref(), TypeDescriptor::Never)
        ) {
            return Ok(false);
        }
        if let TypeDescriptor::Intersection(members) = &actual
            && self.intersection_is_definitely_empty(members, depth + 1)?
        {
            return Ok(true);
        }
        if let TypeDescriptor::Union(members) = &actual {
            for member in members {
                if !self.descriptor_is_subtype(member, &expected, environment, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if let TypeDescriptor::Intersection(members) = &expected {
            for member in members {
                if !self.descriptor_is_subtype(&actual, member, environment, depth + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if let TypeDescriptor::Intersection(members) = &actual {
            for member in members {
                if self.descriptor_is_subtype(member, &expected, environment, depth + 1)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if let TypeDescriptor::Union(members) = &expected {
            if Self::union_is_definitely_total(members) {
                return Ok(true);
            }
            for member in members {
                if self.descriptor_is_subtype(&actual, member, environment, depth + 1)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Ok(match (&actual, &expected) {
            (TypeDescriptor::TrueLiteral | TypeDescriptor::FalseLiteral, TypeDescriptor::Bool)
            | (TypeDescriptor::IntLiteral(_), TypeDescriptor::Int)
            | (TypeDescriptor::IntRange { .. }, TypeDescriptor::Int)
            | (TypeDescriptor::FloatLiteral(_), TypeDescriptor::Float)
            | (TypeDescriptor::StringLiteral(_), TypeDescriptor::String) => true,
            (TypeDescriptor::IntLiteral(value), TypeDescriptor::IntRange { min, max }) => {
                min.is_none_or(|min| *value >= min) && max.is_none_or(|max| *value <= max)
            }
            (
                TypeDescriptor::IntRange {
                    min: actual_min,
                    max: actual_max,
                },
                TypeDescriptor::IntRange {
                    min: expected_min,
                    max: expected_max,
                },
            ) => {
                range_lower_contains(*expected_min, *actual_min)
                    && range_upper_contains(*expected_max, *actual_max)
            }
            (TypeDescriptor::Named { name: actual, .. }, TypeDescriptor::Object) => self
                .resolve_checked_name(actual.clone())?
                .is_some_and(|entry| matches!(entry.kind, SymbolKind::Class | SymbolKind::Enum)),
            (TypeDescriptor::Member { class, member, .. }, TypeDescriptor::Object) => {
                let Some(entry) = self.resolve_checked_name(class.clone())? else {
                    return Ok(false);
                };
                if !matches!(
                    entry.kind,
                    SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                ) {
                    return Ok(false);
                }
                matches!(
                    self.engine.tables.classes[entry.index as usize]
                        .members
                        .get(member),
                    Some(ClassMemberEntry::EnumCase(_))
                )
            }
            (
                actual @ TypeDescriptor::Member {
                    class: actual_class,
                    member: actual_member,
                    ..
                },
                expected @ TypeDescriptor::Member {
                    class: expected_class,
                    member: expected_member,
                    ..
                },
            ) => {
                actual_class == expected_class
                    && actual_member == expected_member
                    && descriptor_same(actual, expected)
            }
            (
                TypeDescriptor::Member {
                    class,
                    class_arguments,
                    member,
                    ..
                },
                TypeDescriptor::Named {
                    name, arguments, ..
                },
            ) => {
                let Some(entry) = self.resolve_checked_name(class.clone())? else {
                    return Ok(false);
                };
                if !matches!(
                    entry.kind,
                    SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                ) {
                    return Ok(false);
                }
                if !matches!(
                    self.engine.tables.classes[entry.index as usize]
                        .members
                        .get(member),
                    Some(ClassMemberEntry::EnumCase(_))
                ) {
                    false
                } else {
                    self.descriptor_is_subtype(
                        &TypeDescriptor::Named {
                            name: class.clone(),
                            arguments: class_arguments.clone(),
                            recursive: false,
                        },
                        &TypeDescriptor::Named {
                            name: name.clone(),
                            arguments: arguments.clone(),
                            recursive: false,
                        },
                        environment,
                        depth + 1,
                    )?
                }
            }
            (
                TypeDescriptor::Named {
                    name: actual_name,
                    arguments: actual_arguments,
                    ..
                },
                TypeDescriptor::Named {
                    name: expected_name,
                    arguments: expected_arguments,
                    ..
                },
            ) => {
                let actual_entry = self.resolve_checked_name(actual_name.clone())?;
                let expected_entry = self.resolve_checked_name(expected_name.clone())?;
                match (actual_entry, expected_entry) {
                    (Some(actual_entry), Some(expected_entry))
                        if matches!(
                            actual_entry.kind,
                            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                        ) && matches!(
                            expected_entry.kind,
                            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                        ) =>
                    {
                        if !is_instance_of(
                            &self.engine.tables.classes,
                            ClassId(actual_entry.index),
                            ClassId(expected_entry.index),
                        ) {
                            false
                        } else if expected_arguments.is_none() {
                            true
                        } else {
                            let actual_class = ClassId(actual_entry.index);
                            let expected_class = ClassId(expected_entry.index);
                            let (actual_parameters, actual_subject) = {
                                let class =
                                    &self.engine.tables.classes[actual_entry.index as usize];
                                (Rc::clone(&class.type_parameters), class.name.clone())
                            };
                            if actual_arguments.is_none()
                                && actual_parameters
                                    .iter()
                                    .any(|parameter| parameter.default.is_none())
                            {
                                false
                            } else {
                                let actual_environment = self.bind_type_parameters(
                                    &actual_parameters,
                                    actual_arguments.as_deref(),
                                    TypeEnvironmentId::default(),
                                    actual_subject.as_bytes(),
                                )?;
                                let Some(projected_environment) = self.environment_for_class(
                                    actual_class,
                                    actual_environment,
                                    expected_class,
                                    depth + 1,
                                )?
                                else {
                                    return Ok(false);
                                };
                                let (expected_parameters, expected_subject) = {
                                    let class =
                                        &self.engine.tables.classes[expected_entry.index as usize];
                                    (Rc::clone(&class.type_parameters), class.name.clone())
                                };
                                let expected_environment = self.bind_type_parameters(
                                    &expected_parameters,
                                    expected_arguments.as_deref(),
                                    TypeEnvironmentId::default(),
                                    expected_subject.as_bytes(),
                                )?;
                                self.nominal_environments_compatible(
                                    expected_class,
                                    projected_environment,
                                    expected_environment,
                                    depth + 1,
                                )?
                            }
                        }
                    }
                    _ => false,
                }
            }
            (TypeDescriptor::Array(_), TypeDescriptor::Array(None)) => true,
            (
                TypeDescriptor::Array(Some((actual_key, actual_value))),
                TypeDescriptor::Array(Some((expected_key, expected_value))),
            )
            | (
                TypeDescriptor::Dictionary(Some((actual_key, actual_value))),
                TypeDescriptor::Dictionary(Some((expected_key, expected_value))),
            ) => {
                self.descriptor_is_subtype(actual_key, expected_key, environment, depth + 1)?
                    && self.descriptor_is_subtype(
                        actual_value,
                        expected_value,
                        environment,
                        depth + 1,
                    )?
            }
            (
                TypeDescriptor::Vector(Some(actual_value)),
                TypeDescriptor::Array(Some((expected_key, expected_value))),
            ) => {
                matches!(actual_value.as_ref(), TypeDescriptor::Never)
                    || self.descriptor_is_subtype(
                        &TypeDescriptor::integer_range(Some(0), None),
                        expected_key,
                        environment,
                        depth + 1,
                    )? && self.descriptor_is_subtype(
                        actual_value,
                        expected_value,
                        environment,
                        depth + 1,
                    )?
            }
            (TypeDescriptor::Vector(_), TypeDescriptor::Array(None)) => true,
            (
                TypeDescriptor::VectorShape { elements, rest },
                TypeDescriptor::Array(Some((expected_key, expected_value))),
            ) => {
                let empty = elements.is_empty()
                    && rest
                        .as_deref()
                        .is_none_or(|rest| matches!(rest, TypeDescriptor::Never));
                if empty {
                    true
                } else if !self.descriptor_is_subtype(
                    &TypeDescriptor::integer_range(Some(0), None),
                    expected_key,
                    environment,
                    depth + 1,
                )? {
                    false
                } else {
                    let mut compatible = true;
                    for element in elements {
                        if !self.descriptor_is_subtype(
                            element,
                            expected_value,
                            environment,
                            depth + 1,
                        )? {
                            compatible = false;
                            break;
                        }
                    }
                    if compatible && let Some(rest) = rest {
                        compatible = self.descriptor_is_subtype(
                            rest,
                            expected_value,
                            environment,
                            depth + 1,
                        )?;
                    }
                    compatible
                }
            }
            (TypeDescriptor::VectorShape { .. }, TypeDescriptor::Array(None)) => true,
            (
                TypeDescriptor::Dictionary(Some((actual_key, actual_value))),
                TypeDescriptor::Array(Some((expected_key, expected_value))),
            ) => {
                if matches!(actual_key.as_ref(), TypeDescriptor::Never)
                    && matches!(actual_value.as_ref(), TypeDescriptor::Never)
                {
                    true
                } else {
                    self.descriptor_is_subtype(actual_key, expected_key, environment, depth + 1)?
                        && self.descriptor_is_subtype(
                            actual_value,
                            expected_value,
                            environment,
                            depth + 1,
                        )?
                }
            }
            (TypeDescriptor::Dictionary(_), TypeDescriptor::Array(None)) => true,
            (
                TypeDescriptor::DictionaryShape { entries, rest },
                TypeDescriptor::Array(Some((expected_key, expected_value))),
            ) => {
                let mut compatible = true;
                for (key, value) in entries {
                    let key = match key {
                        ShapeKey::Int(_) => TypeDescriptor::Int,
                        ShapeKey::String(_) => TypeDescriptor::String,
                    };
                    if !self.descriptor_is_subtype(&key, expected_key, environment, depth + 1)?
                        || !self.descriptor_is_subtype(
                            value,
                            expected_value,
                            environment,
                            depth + 1,
                        )?
                    {
                        compatible = false;
                        break;
                    }
                }
                if compatible && let Some((key, value)) = rest {
                    compatible =
                        self.descriptor_is_subtype(key, expected_key, environment, depth + 1)?
                            && self.descriptor_is_subtype(
                                value,
                                expected_value,
                                environment,
                                depth + 1,
                            )?;
                }
                compatible
            }
            (TypeDescriptor::DictionaryShape { .. }, TypeDescriptor::Array(None)) => true,
            (
                TypeDescriptor::Tuple(actual),
                TypeDescriptor::Array(Some((expected_key, expected_value))),
            ) => {
                if actual.is_empty() {
                    true
                } else if !self.descriptor_is_subtype(
                    &TypeDescriptor::integer_range(Some(0), Some(actual.len() as i64 - 1)),
                    expected_key,
                    environment,
                    depth + 1,
                )? {
                    false
                } else {
                    let mut compatible = true;
                    for member in actual {
                        if !self.descriptor_is_subtype(
                            member,
                            expected_value,
                            environment,
                            depth + 1,
                        )? {
                            compatible = false;
                            break;
                        }
                    }
                    compatible
                }
            }
            (TypeDescriptor::Tuple(_), TypeDescriptor::Array(None))
            | (TypeDescriptor::TupleRest { .. }, TypeDescriptor::Array(None))
            | (TypeDescriptor::TupleAny, TypeDescriptor::Array(None)) => true,
            (
                TypeDescriptor::TupleRest { elements, rest },
                TypeDescriptor::Array(Some((expected_key, expected_value))),
            ) => {
                if !self.descriptor_is_subtype(
                    &TypeDescriptor::integer_range(Some(0), None),
                    expected_key,
                    environment,
                    depth + 1,
                )? {
                    false
                } else {
                    let mut compatible = true;
                    for element in elements {
                        if !self.descriptor_is_subtype(
                            element,
                            expected_value,
                            environment,
                            depth + 1,
                        )? {
                            compatible = false;
                            break;
                        }
                    }

                    compatible
                        && self.descriptor_is_subtype(
                            rest,
                            expected_value,
                            environment,
                            depth + 1,
                        )?
                }
            }
            (TypeDescriptor::Vector(Some(actual)), TypeDescriptor::Vector(Some(expected))) => {
                self.descriptor_is_subtype(actual, expected, environment, depth + 1)?
            }
            (TypeDescriptor::Vector(_), TypeDescriptor::Vector(None)) => true,
            (
                TypeDescriptor::VectorShape { elements, rest },
                TypeDescriptor::Vector(Some(expected)),
            ) => {
                let mut compatible = true;
                for element in elements {
                    if !self.descriptor_is_subtype(element, expected, environment, depth + 1)? {
                        compatible = false;
                        break;
                    }
                }
                if compatible && let Some(rest) = rest {
                    compatible =
                        self.descriptor_is_subtype(rest, expected, environment, depth + 1)?;
                }
                compatible
            }
            (TypeDescriptor::VectorShape { .. }, TypeDescriptor::Vector(None)) => true,
            (
                TypeDescriptor::VectorShape {
                    elements: actual,
                    rest: actual_rest,
                },
                TypeDescriptor::VectorShape {
                    elements: expected,
                    rest: expected_rest,
                },
            ) => {
                if actual.len() < expected.len()
                    || expected_rest.is_none() && actual.len() != expected.len()
                {
                    false
                } else {
                    let mut fixed = true;
                    for (actual, expected) in actual.iter().zip(expected) {
                        if !self.descriptor_is_subtype(actual, expected, environment, depth + 1)? {
                            fixed = false;
                            break;
                        }
                    }
                    fixed
                        && match (actual_rest, expected_rest) {
                            (_, None) => true,
                            (Some(actual), Some(expected)) => self.descriptor_is_subtype(
                                actual,
                                expected,
                                environment,
                                depth + 1,
                            )?,
                            (None, Some(_)) => actual.len() > expected.len(),
                        }
                }
            }
            (TypeDescriptor::Dictionary(_), TypeDescriptor::Dictionary(None)) => true,
            (
                TypeDescriptor::DictionaryShape { entries, rest },
                TypeDescriptor::Dictionary(Some((expected_key, expected_value))),
            ) => {
                let mut compatible = true;
                for (_, value) in entries {
                    if !self.descriptor_is_subtype(value, expected_value, environment, depth + 1)? {
                        compatible = false;
                        break;
                    }
                }
                if compatible && let Some((key, value)) = rest {
                    compatible =
                        self.descriptor_is_subtype(key, expected_key, environment, depth + 1)?
                            && self.descriptor_is_subtype(
                                value,
                                expected_value,
                                environment,
                                depth + 1,
                            )?;
                }
                compatible
            }
            (TypeDescriptor::DictionaryShape { .. }, TypeDescriptor::Dictionary(None)) => true,
            (TypeDescriptor::Tuple(actual), TypeDescriptor::Tuple(expected))
                if actual.len() == expected.len() =>
            {
                let mut compatible = true;
                for (actual, expected) in actual.iter().zip(expected) {
                    if !self.descriptor_is_subtype(actual, expected, environment, depth + 1)? {
                        compatible = false;
                        break;
                    }
                }
                compatible
            }
            (
                TypeDescriptor::Tuple(actual),
                TypeDescriptor::TupleRest {
                    elements: expected,
                    rest,
                },
            ) if actual.len() >= expected.len() => {
                let mut compatible = true;
                for (actual, expected) in actual.iter().zip(expected) {
                    if !self.descriptor_is_subtype(actual, expected, environment, depth + 1)? {
                        compatible = false;
                        break;
                    }
                }
                if compatible {
                    for actual in &actual[expected.len()..] {
                        if !self.descriptor_is_subtype(actual, rest, environment, depth + 1)? {
                            compatible = false;
                            break;
                        }
                    }
                }
                compatible
            }
            (
                TypeDescriptor::TupleRest {
                    elements: actual_elements,
                    rest: actual_rest,
                },
                TypeDescriptor::TupleRest {
                    elements: expected_elements,
                    rest: expected_rest,
                },
            ) if actual_elements.len() >= expected_elements.len() => {
                let mut compatible = true;
                for (actual, expected) in actual_elements.iter().zip(expected_elements) {
                    if !self.descriptor_is_subtype(actual, expected, environment, depth + 1)? {
                        compatible = false;
                        break;
                    }
                }
                if compatible {
                    for actual in &actual_elements[expected_elements.len()..] {
                        if !self.descriptor_is_subtype(
                            actual,
                            expected_rest,
                            environment,
                            depth + 1,
                        )? {
                            compatible = false;
                            break;
                        }
                    }
                }
                compatible
                    && self.descriptor_is_subtype(
                        actual_rest,
                        expected_rest,
                        environment,
                        depth + 1,
                    )?
            }
            (TypeDescriptor::TupleRest { elements, rest }, TypeDescriptor::Tuple(expected))
                if matches!(rest.as_ref(), TypeDescriptor::Never)
                    && elements.len() == expected.len() =>
            {
                let mut compatible = true;
                for (actual, expected) in elements.iter().zip(expected) {
                    if !self.descriptor_is_subtype(actual, expected, environment, depth + 1)? {
                        compatible = false;
                        break;
                    }
                }
                compatible
            }
            (TypeDescriptor::Tuple(_), TypeDescriptor::TupleAny)
            | (TypeDescriptor::TupleRest { .. }, TypeDescriptor::TupleAny)
            | (TypeDescriptor::TupleAny, TypeDescriptor::TupleAny) => true,
            (TypeDescriptor::Callable(_), TypeDescriptor::Callable(None)) => true,
            (TypeDescriptor::Callable(Some(actual)), TypeDescriptor::Callable(Some(expected))) => {
                callable_descriptor_compatible(self, actual, expected, environment, depth + 1)?
            }
            (TypeDescriptor::Classname(actual), TypeDescriptor::Classname(expected)) => {
                self.descriptor_is_subtype(actual, expected, environment, depth + 1)?
            }
            _ => false,
        })
    }

    fn union_is_definitely_total(members: &[TypeDescriptor]) -> bool {
        if members.iter().any(|member| {
            matches!(member, TypeDescriptor::Mixed)
                || matches!(
                    member,
                    TypeDescriptor::Negated(inner)
                        if matches!(inner.as_ref(), TypeDescriptor::Never)
                )
        }) {
            return true;
        }
        members.iter().enumerate().any(|(index, member)| {
            members[index + 1..]
                .iter()
                .any(|other| match (member, other) {
                    (TypeDescriptor::Negated(inner), other)
                    | (other, TypeDescriptor::Negated(inner)) => descriptor_same(inner, other),
                    _ => false,
                })
        })
    }

    fn intersection_is_definitely_empty(
        &mut self,
        members: &[TypeDescriptor],
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        if members.iter().any(|member| {
            matches!(member, TypeDescriptor::Never)
                || matches!(
                    member,
                    TypeDescriptor::Negated(inner)
                        if matches!(inner.as_ref(), TypeDescriptor::Mixed)
                )
        }) {
            return Ok(true);
        }
        for (index, member) in members.iter().enumerate() {
            for other in &members[index + 1..] {
                if self.descriptors_are_disjoint(
                    member,
                    other,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn descriptors_are_disjoint(
        &mut self,
        left: &TypeDescriptor,
        right: &TypeDescriptor,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        if depth > MAX_TYPE_DEPTH_U32 {
            return Ok(false);
        }
        let left = self.substitute_descriptor(left, environment, depth + 1);
        let right = self.substitute_descriptor(right, environment, depth + 1);
        if let Some(expanded) = self.expand_type_alias_once(&left, depth + 1)? {
            return self.descriptors_are_disjoint(
                &expanded,
                &right,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if let Some(expanded) = self.expand_type_alias_once(&right, depth + 1)? {
            return self.descriptors_are_disjoint(
                &left,
                &expanded,
                TypeEnvironmentId::default(),
                depth + 1,
            );
        }
        if matches!(left, TypeDescriptor::Never) || matches!(right, TypeDescriptor::Never) {
            return Ok(true);
        }
        if matches!(left, TypeDescriptor::Mixed | TypeDescriptor::Wildcard)
            || matches!(right, TypeDescriptor::Mixed | TypeDescriptor::Wildcard)
            || descriptor_same(&left, &right)
        {
            return Ok(false);
        }
        let left_value =
            self.exact_descriptor_value(&left, TypeEnvironmentId::default(), depth + 1)?;
        let right_value =
            self.exact_descriptor_value(&right, TypeEnvironmentId::default(), depth + 1)?;
        match (left_value, right_value) {
            (Some(left), Some(right)) => return Ok(!ops::equals(&left, &right)),
            (Some(left), None) => {
                return Ok(!self.check_descriptor(
                    &right,
                    &left,
                    None,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )?);
            }
            (None, Some(right)) => {
                return Ok(!self.check_descriptor(
                    &left,
                    &right,
                    None,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )?);
            }
            (None, None) => {}
        }
        if let TypeDescriptor::Intersection(members) = &left
            && self.intersection_is_definitely_empty(members, depth + 1)?
        {
            return Ok(true);
        }
        if let TypeDescriptor::Intersection(members) = &right
            && self.intersection_is_definitely_empty(members, depth + 1)?
        {
            return Ok(true);
        }
        if let TypeDescriptor::Union(members) = &left {
            for member in members {
                if !self.descriptors_are_disjoint(
                    member,
                    &right,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if let TypeDescriptor::Union(members) = &right {
            for member in members {
                if !self.descriptors_are_disjoint(
                    &left,
                    member,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if let TypeDescriptor::Intersection(members) = &left {
            for member in members {
                if self.descriptors_are_disjoint(
                    member,
                    &right,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )? {
                    return Ok(true);
                }
            }
        }
        if let TypeDescriptor::Intersection(members) = &right {
            for member in members {
                if self.descriptors_are_disjoint(
                    &left,
                    member,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )? {
                    return Ok(true);
                }
            }
        }
        match (&left, &right) {
            (TypeDescriptor::Negated(left), TypeDescriptor::Negated(right)) => {
                if matches!(left.as_ref(), TypeDescriptor::Mixed)
                    || matches!(right.as_ref(), TypeDescriptor::Mixed)
                {
                    return Ok(true);
                }
                return Ok(false);
            }
            (TypeDescriptor::Negated(excluded), other)
            | (other, TypeDescriptor::Negated(excluded)) => {
                return self.descriptor_is_subtype(
                    other,
                    excluded,
                    TypeEnvironmentId::default(),
                    depth + 1,
                );
            }
            _ => {}
        }

        if let (Some(left_kind), Some(right_kind)) = (
            self.descriptor_value_kind(&left)?,
            self.descriptor_value_kind(&right)?,
        ) && left_kind != right_kind
        {
            return Ok(true);
        }
        if let (TypeDescriptor::Tuple(left), TypeDescriptor::Tuple(right)) = (&left, &right)
            && left.len() == right.len()
        {
            for (left, right) in left.iter().zip(right) {
                if self.descriptors_are_disjoint(
                    left,
                    right,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Ok(match (&left, &right) {
            (TypeDescriptor::TrueLiteral, TypeDescriptor::FalseLiteral)
            | (TypeDescriptor::FalseLiteral, TypeDescriptor::TrueLiteral) => true,
            (TypeDescriptor::IntLiteral(left), TypeDescriptor::IntLiteral(right)) => left != right,
            (TypeDescriptor::IntLiteral(value), TypeDescriptor::IntRange { min, max })
            | (TypeDescriptor::IntRange { min, max }, TypeDescriptor::IntLiteral(value)) => {
                min.is_some_and(|min| *value < min) || max.is_some_and(|max| *value > max)
            }
            (
                TypeDescriptor::IntRange {
                    min: left_min,
                    max: left_max,
                },
                TypeDescriptor::IntRange {
                    min: right_min,
                    max: right_max,
                },
            ) => {
                left_max.is_some_and(|max| right_min.is_some_and(|min| max < min))
                    || right_max.is_some_and(|max| left_min.is_some_and(|min| max < min))
            }
            (TypeDescriptor::FloatLiteral(left), TypeDescriptor::FloatLiteral(right)) => {
                left != right
            }
            (TypeDescriptor::StringLiteral(left), TypeDescriptor::StringLiteral(right)) => {
                left != right
            }
            (TypeDescriptor::Tuple(left), TypeDescriptor::Tuple(right))
                if left.len() != right.len() =>
            {
                true
            }
            _ => false,
        })
    }

    fn descriptor_value_kind(
        &mut self,
        descriptor: &TypeDescriptor,
    ) -> Result<Option<u8>, VirtualMachineControl> {
        Ok(match descriptor {
            TypeDescriptor::Null => Some(0),
            TypeDescriptor::Bool | TypeDescriptor::TrueLiteral | TypeDescriptor::FalseLiteral => {
                Some(1)
            }
            TypeDescriptor::Int
            | TypeDescriptor::IntLiteral(_)
            | TypeDescriptor::IntRange { .. } => Some(2),
            TypeDescriptor::Float | TypeDescriptor::FloatLiteral(_) => Some(3),
            TypeDescriptor::String
            | TypeDescriptor::StringLiteral(_)
            | TypeDescriptor::Classname(_) => Some(4),
            TypeDescriptor::Vector(_) | TypeDescriptor::VectorShape { .. } => Some(5),
            TypeDescriptor::Dictionary(_) | TypeDescriptor::DictionaryShape { .. } => Some(6),
            TypeDescriptor::Callable(_) => Some(7),
            TypeDescriptor::Tuple(_)
            | TypeDescriptor::TupleRest { .. }
            | TypeDescriptor::TupleAny => Some(8),
            TypeDescriptor::Object => Some(9),
            TypeDescriptor::Member { class, member, .. } => {
                let entry = self.resolve_checked_name(class.clone())?;
                entry
                    .filter(|entry| {
                        if !matches!(
                            entry.kind,
                            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                        ) {
                            return false;
                        }
                        matches!(
                            self.engine.tables.classes[entry.index as usize]
                                .members
                                .get(member),
                            Some(ClassMemberEntry::EnumCase(_))
                        )
                    })
                    .map(|_| 9)
            }
            TypeDescriptor::Named { name, .. } => self
                .resolve_checked_name(name.clone())?
                .filter(|entry| {
                    matches!(
                        entry.kind,
                        SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                    )
                })
                .map(|_| 9),
            _ => None,
        })
    }

    /// Expands one named alias after binding its concrete arguments. Keeping
    /// this at the subtype boundary makes aliases participate identically in
    /// bounds, variance, callable compatibility, and name-type checks.
    fn expand_type_alias_once(
        &mut self,
        descriptor: &TypeDescriptor,
        depth: u32,
    ) -> Result<Option<TypeDescriptor>, VirtualMachineControl> {
        let TypeDescriptor::Named {
            name, arguments, ..
        } = descriptor
        else {
            return Ok(None);
        };
        let Some(entry) = self.resolve_checked_name(name.clone())? else {
            return Ok(None);
        };
        if entry.kind != SymbolKind::TypeAlias {
            return Ok(None);
        }
        let (parameters, aliased) = {
            let alias = &self.engine.tables.type_aliases[entry.index as usize];
            (alias.type_parameters.clone(), alias.descriptor.clone())
        };
        let alias_environment = self.bind_type_parameters(
            &parameters,
            arguments.as_deref(),
            TypeEnvironmentId::default(),
            name.as_bytes(),
        )?;
        Ok(Some(self.substitute_descriptor(
            &aliased,
            alias_environment,
            depth + 1,
        )))
    }

    fn nominal_environments_compatible(
        &mut self,
        class: ClassId,
        actual_environment: TypeEnvironmentId,
        expected_environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        let memo = (class, actual_environment, expected_environment);
        if let Some(verdict) = self.engine.tables.nominal_compatibility_cache.get(&memo) {
            return Ok(*verdict);
        }

        let parameters = Rc::clone(&self.engine.tables.classes[class.0 as usize].type_parameters);
        for parameter in parameters.iter() {
            let Some(actual) = self
                .type_environment_binding(actual_environment, &parameter.name)
                .cloned()
            else {
                self.engine
                    .tables
                    .nominal_compatibility_cache
                    .insert(memo, false);
                return Ok(false);
            };
            let Some(expected) = self
                .type_environment_binding(expected_environment, &parameter.name)
                .cloned()
            else {
                self.engine
                    .tables
                    .nominal_compatibility_cache
                    .insert(memo, false);
                return Ok(false);
            };
            if matches!(expected, TypeDescriptor::Wildcard) {
                continue;
            }
            let compatible = match parameter.variance {
                Variance::Invariant => descriptor_same(&actual, &expected),
                Variance::Covariant => self.descriptor_is_subtype(
                    &actual,
                    &expected,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )?,
                Variance::Contravariant => self.descriptor_is_subtype(
                    &expected,
                    &actual,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )?,
            };
            if !compatible {
                self.engine
                    .tables
                    .nominal_compatibility_cache
                    .insert(memo, false);
                return Ok(false);
            }
        }
        self.engine
            .tables
            .nominal_compatibility_cache
            .insert(memo, true);
        Ok(true)
    }

    fn generic_environments_compatible(
        &mut self,
        parameters: &[CompiledTypeParameter],
        actual_environment: TypeEnvironmentId,
        expected_environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        for parameter in parameters {
            let Some(actual) = self
                .type_environment_binding(actual_environment, &parameter.name)
                .cloned()
            else {
                return Ok(false);
            };
            let Some(expected) = self
                .type_environment_binding(expected_environment, &parameter.name)
                .cloned()
            else {
                return Ok(false);
            };
            if matches!(expected, TypeDescriptor::Wildcard) {
                continue;
            }
            let compatible = match parameter.variance {
                Variance::Invariant => descriptor_same(&actual, &expected),
                Variance::Covariant => self.descriptor_is_subtype(
                    &actual,
                    &expected,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )?,
                Variance::Contravariant => self.descriptor_is_subtype(
                    &expected,
                    &actual,
                    TypeEnvironmentId::default(),
                    depth + 1,
                )?,
            };
            if !compatible {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn expand_newtype_once(
        &mut self,
        descriptor: &TypeDescriptor,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<Option<TypeDescriptor>, VirtualMachineControl> {
        if depth > MAX_TYPE_DEPTH_U32 {
            return Ok(None);
        }
        let TypeDescriptor::Named {
            name, arguments, ..
        } = descriptor
        else {
            return Ok(None);
        };
        let Some(entry) = self.resolve_checked_name(name.clone())? else {
            return Ok(None);
        };
        if entry.kind != SymbolKind::Newtype {
            return Ok(None);
        }
        let (parameters, backing, subject) = {
            let declaration = &self.engine.tables.newtypes[entry.index as usize];
            (
                declaration.type_parameters.clone(),
                declaration.backing.clone(),
                declaration.name.clone(),
            )
        };
        let bound = self.bind_type_parameters(
            &parameters,
            arguments.as_deref(),
            environment,
            subject.as_bytes(),
        )?;
        Ok(Some(self.substitute_descriptor(&backing, bound, depth + 1)))
    }

    fn static_class_value_compatible(
        &mut self,
        value: &Value,
        called: Option<ClassId>,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        let (Some(called), Some(actual)) = (called, value.as_object()) else {
            return Ok(false);
        };
        if !is_instance_of(&self.engine.tables.classes, actual.class(), called) {
            return Ok(false);
        }
        let expected = self.current_this().cloned().filter(|instance| {
            is_instance_of(&self.engine.tables.classes, instance.class(), called)
        });
        let Some(expected) = expected else {
            return Ok(true);
        };
        let Some(actual_environment) = self.environment_for_class(
            actual.class(),
            actual.type_environment(),
            called,
            depth + 1,
        )?
        else {
            return Ok(false);
        };
        let Some(expected_environment) = self.environment_for_class(
            expected.class(),
            expected.type_environment(),
            called,
            depth + 1,
        )?
        else {
            return Ok(false);
        };
        self.nominal_environments_compatible(
            called,
            actual_environment,
            expected_environment,
            depth + 1,
        )
    }

    fn resolved_element_descriptor(
        &mut self,
        descriptor: &TypeDescriptor,
        environment: TypeEnvironmentId,
    ) -> Result<Option<(TypeDescriptor, TypeEnvironmentId)>, VirtualMachineControl> {
        let mut current = descriptor.clone();
        let mut environment = environment;
        let mut hops = 0;
        loop {
            if hops > MAX_TYPE_DEPTH_U32 {
                return Ok(None);
            }

            hops += 1;
            current = match current {
                TypeDescriptor::Parameter(name) => {
                    match self.type_environment_binding(environment, &name).cloned() {
                        Some(bound) => bound,
                        None => return Ok(None),
                    }
                }
                TypeDescriptor::Named {
                    name,
                    arguments,
                    recursive,
                } => {
                    let Some(entry) = self.resolve_checked_name(name.clone())? else {
                        return Ok(None);
                    };

                    if entry.kind != SymbolKind::TypeAlias {
                        return Ok(Some((
                            TypeDescriptor::Named {
                                name,
                                arguments,
                                recursive,
                            },
                            environment,
                        )));
                    }

                    let (parameters, aliased) = {
                        let alias = &self.engine.tables.type_aliases[entry.index as usize];
                        (alias.type_parameters.clone(), alias.descriptor.clone())
                    };

                    environment = self.bind_type_parameters(
                        &parameters,
                        arguments.as_deref(),
                        environment,
                        name.as_bytes(),
                    )?;

                    aliased
                }
                other => return Ok(Some((other, environment))),
            };
        }
    }

    fn check_dictionary_elements(
        &mut self,
        descriptor: &TypeDescriptor,
        element_types: (&TypeDescriptor, &TypeDescriptor),
        dictionary: &ManagedRef<DictObject>,
        called: Option<ClassId>,
        environment: TypeEnvironmentId,
        collection_id: Option<CollectionTypeCheckId>,
    ) -> Result<bool, VirtualMachineControl> {
        let (key_type, value_type) = element_types;
        let check_key = !matches!(key_type, TypeDescriptor::Wildcard);
        let check_value = !matches!(value_type, TypeDescriptor::Wildcard);
        let cache_id = collection_id.or_else(|| self.collection_type_check_id(descriptor));
        if let Some(cache_id) = cache_id {
            match dictionary.type_check(cache_id) {
                CollectionTypeCheck::Clean(_) => return Ok(true),
                CollectionTypeCheck::Dirty { slot, .. } => {
                    let Some(Some((key, value))) = dictionary.entry_at_slot(slot as usize) else {
                        return Ok(false);
                    };
                    let valid = (!check_key
                        || self.check_descriptor(
                            key_type,
                            &key_ref_value(key),
                            called,
                            environment,
                            0,
                        )?)
                        && (!check_value
                            || self.check_descriptor(value_type, value, called, environment, 0)?);
                    if valid {
                        dictionary.mark_type_checked(cache_id);
                    }
                    return Ok(valid);
                }
                CollectionTypeCheck::Unknown => {}
            }
        }

        let resolved_key = if check_key {
            match self.resolved_element_descriptor(key_type, environment)? {
                Some(resolved) => Some(resolved),
                None => return Ok(dictionary.is_empty()),
            }
        } else {
            None
        };
        let resolved_value = if check_value {
            match self.resolved_element_descriptor(value_type, environment)? {
                Some(resolved) => Some(resolved),
                None => return Ok(dictionary.is_empty()),
            }
        } else {
            None
        };

        let mut all = true;
        let mut trivial = true;
        for (key, value) in dictionary.iter() {
            let key_result = resolved_key.as_ref().map_or(Some(true), |(descriptor, _)| {
                check_trivial_descriptor(descriptor, &key_ref_value(key))
            });
            let value_result = resolved_value
                .as_ref()
                .map_or(Some(true), |(descriptor, _)| {
                    check_trivial_descriptor(descriptor, value)
                });
            match (key_result, value_result) {
                (Some(true), Some(true)) => {}
                (Some(false), _) | (_, Some(false)) => {
                    all = false;
                    break;
                }
                _ => {
                    trivial = false;
                    break;
                }
            }
        }

        if !trivial {
            let entries: Vec<_> = dictionary
                .iter()
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect();
            all = true;
            for (key, value) in entries {
                let key_valid = match &resolved_key {
                    Some((descriptor, key_environment)) => self.check_descriptor(
                        descriptor,
                        &key_value(key),
                        called,
                        *key_environment,
                        0,
                    )?,
                    None => true,
                };
                let value_valid = key_valid
                    && match &resolved_value {
                        Some((descriptor, value_environment)) => self.check_descriptor(
                            descriptor,
                            &value,
                            called,
                            *value_environment,
                            0,
                        )?,
                        None => true,
                    };
                if !key_valid || !value_valid {
                    all = false;
                    break;
                }
            }
        }

        if all && let Some(cache_id) = cache_id {
            dictionary.mark_type_checked(cache_id);
        }
        Ok(all)
    }

    fn exact_function_value_matches(
        &mut self,
        entry: SymbolEntry,
        arguments: Option<&[TypeDescriptor]>,
        value: &Value,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        let Some(function) = value.as_function() else {
            return Ok(false);
        };
        let (target, parameters, parameter_count, subject) = match entry.table {
            FunctionTable::User => {
                let target = CallTarget::User(FuncId(entry.index));
                let runtime = &self.engine.tables.functions[entry.index as usize];
                (
                    target,
                    runtime.type_parameters().to_vec(),
                    runtime.parameters().len(),
                    runtime.name.clone(),
                )
            }
            FunctionTable::BuiltIn => {
                let target = CallTarget::BuiltIn(BuiltInId(entry.index));
                let callable = self.engine.tables.built_in_functions[entry.index as usize].clone();
                (
                    target,
                    built_in_type_parameters(&self.heap, callable.type_parameters()),
                    callable.parameters().len(),
                    self.heap.intern(callable.display_name().as_bytes()),
                )
            }
        };
        if function.target() != target
            || function.this().is_some()
            || function.called().is_some()
            || !function.captures().is_empty()
            || !is_complete_callable_binding(function, parameter_count)
        {
            return Ok(false);
        }
        let Some(arguments) = arguments else {
            return Ok(true);
        };
        if !function.type_arguments_bound() {
            return Ok(false);
        }
        let expected_environment = self.bind_type_parameters(
            &parameters,
            Some(arguments),
            environment,
            subject.as_bytes(),
        )?;
        self.generic_environments_compatible(
            &parameters,
            function.type_environment(),
            expected_environment,
            depth + 1,
        )
    }

    fn exact_method_value_matches(
        &mut self,
        owner: ClassId,
        descriptor: &TypeDescriptor,
        value: &Value,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        let TypeDescriptor::Member {
            class_arguments,
            member,
            member_arguments,
            ..
        } = descriptor
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the exact method descriptor is a member") }
        };
        let Some(function) = value.as_function() else {
            return Ok(false);
        };
        if !function.captures().is_empty() {
            return Ok(false);
        }
        let Some(actual_class) = function
            .this()
            .map(|instance| instance.class())
            .or_else(|| function.called())
            .or_else(|| function.scope())
        else {
            return Ok(false);
        };
        if !is_instance_of(&self.engine.tables.classes, actual_class, owner) {
            return Ok(false);
        }
        let expected = self.engine.tables.classes[owner.0 as usize]
            .method(member)
            .or_else(|| {
                self.engine.tables.classes[owner.0 as usize]
                    .private_methods
                    .get(&(owner, member.clone()))
                    .copied()
            });
        let Some(expected) = expected else {
            return Ok(false);
        };
        let actual = function
            .scope()
            .and_then(|scope| {
                self.engine.tables.classes[actual_class.0 as usize]
                    .private_methods
                    .get(&(scope, member.clone()))
                    .copied()
            })
            .or_else(|| self.engine.tables.classes[actual_class.0 as usize].method(member));
        let Some(actual) = actual else {
            return Ok(false);
        };
        if actual.visibility == Visibility::Private
            && actual.declaring_class != expected.declaring_class
        {
            return Ok(false);
        }
        let target = match actual.body {
            MethodBodyKind::Bytecode(function) => CallTarget::User(function),
            MethodBodyKind::BuiltIn(_) => {
                CallTarget::BuiltIn(self.built_in_id_for_method(&actual, member.clone()))
            }
        };
        if function.target() != target {
            return Ok(false);
        }
        let (parameters, parameter_count, subject) = match target {
            CallTarget::User(id) => {
                let runtime = &self.engine.tables.functions[id.0 as usize];
                (
                    runtime.type_parameters().to_vec(),
                    runtime.parameters().len(),
                    runtime.name.clone(),
                )
            }
            CallTarget::BuiltIn(id) => {
                let callable = self.engine.tables.built_in_functions[id.0 as usize].clone();
                (
                    built_in_type_parameters(&self.heap, callable.type_parameters()),
                    callable.parameters().len(),
                    self.heap.intern(callable.display_name().as_bytes()),
                )
            }
        };
        if !is_complete_callable_binding(function, parameter_count) {
            return Ok(false);
        }
        if let (Some(instance), Some(class_arguments)) =
            (function.this(), class_arguments.as_deref())
        {
            let Some(actual_environment) = self.environment_for_class(
                instance.class(),
                instance.type_environment(),
                owner,
                depth + 1,
            )?
            else {
                return Ok(false);
            };
            let (owner_parameters, owner_name) = {
                let owner = &self.engine.tables.classes[owner.0 as usize];
                (Rc::clone(&owner.type_parameters), owner.name.clone())
            };
            let expected_environment = self.bind_type_parameters(
                &owner_parameters,
                Some(class_arguments),
                environment,
                owner_name.as_bytes(),
            )?;
            if !self.nominal_environments_compatible(
                owner,
                actual_environment,
                expected_environment,
                depth + 1,
            )? {
                return Ok(false);
            }
        }
        let Some(member_arguments) = member_arguments.as_deref() else {
            return Ok(true);
        };
        if !function.type_arguments_bound() {
            return Ok(false);
        }
        let expected_environment = self.bind_type_parameters(
            &parameters,
            Some(member_arguments),
            environment,
            subject.as_bytes(),
        )?;
        self.generic_environments_compatible(
            &parameters,
            function.type_environment(),
            expected_environment,
            depth + 1,
        )
    }

    pub(crate) fn check_descriptor(
        &mut self,
        descriptor: &TypeDescriptor,
        value: &Value,
        called: Option<ClassId>,
        environment: TypeEnvironmentId,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        self.check_descriptor_with_collection_id(
            descriptor,
            value,
            called,
            environment,
            None,
            depth,
        )
    }

    pub(in crate::vm) fn check_descriptor_with_collection_id(
        &mut self,
        descriptor: &TypeDescriptor,
        value: &Value,
        called: Option<ClassId>,
        environment: TypeEnvironmentId,
        collection_id: Option<CollectionTypeCheckId>,
        depth: u32,
    ) -> Result<bool, VirtualMachineControl> {
        if depth > MAX_TYPE_DEPTH_U32 {
            return Ok(false);
        }
        if let Some(result) = check_trivial_descriptor(descriptor, value) {
            return Ok(result);
        }
        Ok(match descriptor {
            TypeDescriptor::Wildcard | TypeDescriptor::Mixed => true,
            TypeDescriptor::Void | TypeDescriptor::Never => false,
            TypeDescriptor::Null => value.is_null(),
            TypeDescriptor::Bool => value.is_bool(),
            TypeDescriptor::Int => value.is_int(),
            TypeDescriptor::Float => value.is_float(),
            TypeDescriptor::String => value.is_string(),
            TypeDescriptor::Object => value.is_object(),
            TypeDescriptor::TrueLiteral => value.as_bool() == Some(true),
            TypeDescriptor::FalseLiteral => value.as_bool() == Some(false),
            TypeDescriptor::IntLiteral(expected) => value.as_int() == Some(*expected),
            TypeDescriptor::IntRange { min, max } => value.as_int().is_some_and(|value| {
                min.is_none_or(|min| value >= min) && max.is_none_or(|max| value <= max)
            }),
            TypeDescriptor::FloatLiteral(expected) => value.as_float() == Some(*expected),
            TypeDescriptor::StringLiteral(expected) => value
                .as_string_bytes()
                .is_some_and(|string| string == expected.as_bytes()),
            descriptor @ TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => {
                let Some(entry) = self.resolve_checked_name(class.clone())? else {
                    return Ok(false);
                };
                if !matches!(
                    entry.kind,
                    SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                ) {
                    return Ok(false);
                }
                let class_id = ClassId(entry.index);
                if class_arguments.is_some() {
                    let parameters =
                        Rc::clone(&self.engine.tables.classes[class_id.0 as usize].type_parameters);
                    self.bind_type_parameters(
                        &parameters,
                        class_arguments.as_deref(),
                        environment,
                        class.as_bytes(),
                    )?;
                }
                let member_entry = self.engine.tables.classes[class_id.0 as usize]
                    .members
                    .get(member)
                    .cloned();
                match member_entry {
                    Some(ClassMemberEntry::EnumCase(_)) if member_arguments.is_none() => {
                        let Some(instance) = self.enum_case_instance(class_id, member.clone())
                        else {
                            return Ok(false);
                        };
                        ops::equals(value, &instance)
                    }
                    Some(ClassMemberEntry::Constant(_)) if member_arguments.is_none() => {
                        let expected = self.force_class_constant(class_id, member.clone())?;
                        ops::equals(value, &expected)
                    }
                    Some(ClassMemberEntry::Method(_)) => self.exact_method_value_matches(
                        class_id,
                        descriptor,
                        value,
                        environment,
                        depth + 1,
                    )?,
                    _ => false,
                }
            }
            TypeDescriptor::Named {
                name, arguments, ..
            } => match self.resolve_checked_name(name.clone())? {
                Some(entry) => match entry.kind {
                    SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface => {
                        let Some(instance) = value.as_object() else {
                            return Ok(false);
                        };
                        let target = ClassId(entry.index);
                        if !is_instance_of(&self.engine.tables.classes, instance.class(), target) {
                            false
                        } else if arguments.is_none() {
                            true
                        } else {
                            let actual_environment = self.environment_for_class(
                                instance.class(),
                                instance.type_environment(),
                                target,
                                depth + 1,
                            )?;
                            let Some(actual_environment) = actual_environment else {
                                return Ok(false);
                            };
                            let (parameters, subject) = {
                                let class = &self.engine.tables.classes[target.0 as usize];
                                (Rc::clone(&class.type_parameters), class.name.clone())
                            };
                            let expected_environment = self.bind_type_parameters(
                                &parameters,
                                arguments.as_deref(),
                                environment,
                                subject.as_bytes(),
                            )?;
                            self.nominal_environments_compatible(
                                target,
                                actual_environment,
                                expected_environment,
                                depth + 1,
                            )?
                        }
                    }
                    SymbolKind::TypeAlias => {
                        let (parameters, aliased) = {
                            let alias = &self.engine.tables.type_aliases[entry.index as usize];
                            (alias.type_parameters.clone(), alias.descriptor.clone())
                        };
                        let alias_environment = self.bind_type_parameters(
                            &parameters,
                            arguments.as_deref(),
                            environment,
                            name.as_bytes(),
                        )?;
                        self.check_descriptor(
                            &aliased,
                            value,
                            called,
                            alias_environment,
                            depth + 1,
                        )?
                    }
                    SymbolKind::Newtype => {
                        let Some(value_id) = value.newtype_id() else {
                            return Ok(false);
                        };
                        let tagged = self.engine.tables.newtype_value(value_id);
                        let backing_value = value.clone_with_newtype(tagged.parent);
                        if tagged.declaration.0 != entry.index {
                            self.check_descriptor(
                                &TypeDescriptor::Named {
                                    name: name.clone(),
                                    arguments: arguments.clone(),
                                    recursive: false,
                                },
                                &backing_value,
                                called,
                                environment,
                                depth + 1,
                            )?
                        } else {
                            let (parameters, backing, subject) = {
                                let declaration =
                                    &self.engine.tables.newtypes[entry.index as usize];
                                (
                                    declaration.type_parameters.clone(),
                                    declaration.backing.clone(),
                                    declaration.name.clone(),
                                )
                            };
                            let backing = self.substitute_descriptor(
                                &backing,
                                tagged.type_environment,
                                depth + 1,
                            );
                            if !self.check_descriptor(
                                &backing,
                                &backing_value,
                                called,
                                tagged.type_environment,
                                depth + 1,
                            )? {
                                return Ok(false);
                            }
                            if arguments.is_none() {
                                return Ok(true);
                            }
                            let expected_environment = self.bind_type_parameters(
                                &parameters,
                                arguments.as_deref(),
                                environment,
                                subject.as_bytes(),
                            )?;
                            self.generic_environments_compatible(
                                &parameters,
                                tagged.type_environment,
                                expected_environment,
                                depth + 1,
                            )?
                        }
                    }
                    SymbolKind::Constant => {
                        if arguments.is_some() {
                            false
                        } else {
                            let expected = self.force_constant(entry.index, name.clone())?;
                            ops::equals(value, &expected)
                        }
                    }
                    SymbolKind::Function => self.exact_function_value_matches(
                        entry,
                        arguments.as_deref(),
                        value,
                        environment,
                        depth + 1,
                    )?,
                },
                None => false,
            },
            TypeDescriptor::StaticClass => {
                self.static_class_value_compatible(value, called, depth + 1)?
            }
            TypeDescriptor::Parameter(name) => {
                let bound = self.type_environment_binding(environment, name).cloned();
                match bound {
                    Some(bound) => {
                        self.check_descriptor(&bound, value, called, environment, depth + 1)?
                    }
                    None => false,
                }
            }
            // SAFETY: the surrounding invariant makes this path unreachable.
            TypeDescriptor::Array(None) => unsafe {
                unreachable_invariant("unparameterized array checks are trivial")
            },
            TypeDescriptor::Array(Some((key_type, value_type))) => {
                let check_key = !matches!(key_type.as_ref(), TypeDescriptor::Wildcard);
                let check_value = !matches!(value_type.as_ref(), TypeDescriptor::Wildcard);
                if let Some(vector) = value.as_vec() {
                    let mut all = true;
                    for (index, value) in vector.iter().enumerate() {
                        if check_key
                            && !self.check_descriptor(
                                key_type,
                                &Value::int(index as i64),
                                called,
                                environment,
                                0,
                            )?
                            || check_value
                                && !self.check_descriptor(
                                    value_type,
                                    value,
                                    called,
                                    environment,
                                    0,
                                )?
                        {
                            all = false;
                            break;
                        }
                    }
                    all
                } else if let Some(dictionary) = value.as_dict() {
                    self.check_dictionary_elements(
                        descriptor,
                        (key_type, value_type),
                        dictionary,
                        called,
                        environment,
                        collection_id,
                    )?
                } else if let Some(tuple) = value.as_tuple() {
                    let mut all = true;
                    for (index, value) in tuple.iter().enumerate() {
                        if check_key
                            && !self.check_descriptor(
                                key_type,
                                &Value::int(index as i64),
                                called,
                                environment,
                                0,
                            )?
                            || check_value
                                && !self.check_descriptor(
                                    value_type,
                                    value,
                                    called,
                                    environment,
                                    0,
                                )?
                        {
                            all = false;
                            break;
                        }
                    }
                    all
                } else {
                    false
                }
            }
            TypeDescriptor::Vector(None) => value.is_vec(),
            TypeDescriptor::Vector(Some(element)) => match value.as_vec() {
                Some(vector) => {
                    let cache_id = match collection_id {
                        Some(id) => Some(id),
                        None => self.collection_type_check_id(descriptor),
                    };
                    if let Some(cache_id) = cache_id {
                        match vector.type_check(cache_id) {
                            CollectionTypeCheck::Clean(_) => return Ok(true),
                            CollectionTypeCheck::Dirty { slot, .. } => {
                                let Some(value) = vector.get(slot as usize) else {
                                    return Ok(false);
                                };
                                let valid =
                                    self.check_descriptor(element, value, called, environment, 0)?;
                                if valid {
                                    vector.mark_type_checked(cache_id);
                                }
                                return Ok(valid);
                            }
                            CollectionTypeCheck::Unknown => {}
                        }
                    }
                    let Some((element, environment)) =
                        self.resolved_element_descriptor(element, environment)?
                    else {
                        return Ok(vector.is_empty());
                    };

                    let mut all = true;
                    let mut trivial = true;
                    for value in vector.iter() {
                        match check_trivial_descriptor(&element, value) {
                            Some(true) => {}
                            Some(false) => {
                                all = false;
                                break;
                            }
                            None => {
                                trivial = false;
                                break;
                            }
                        }
                    }
                    if !trivial {
                        all = true;
                        for value in vector.iter() {
                            if !self.check_descriptor(&element, value, called, environment, 0)? {
                                all = false;
                                break;
                            }
                        }
                    }
                    if all && let Some(cache_id) = cache_id {
                        vector.mark_type_checked(cache_id);
                    }
                    all
                }
                None => false,
            },
            TypeDescriptor::VectorShape { elements, rest } => match value.as_vec() {
                Some(vector) => {
                    if vector.len() < elements.len()
                        || rest.is_none() && vector.len() != elements.len()
                    {
                        false
                    } else {
                        let mut fixed = true;
                        for (index, descriptor) in elements.iter().enumerate() {
                            let Some(value) = vector.get(index) else {
                                fixed = false;
                                break;
                            };
                            if !self.check_descriptor(descriptor, value, called, environment, 0)? {
                                fixed = false;
                                break;
                            }
                        }
                        if fixed && let Some(rest) = rest {
                            for value in vector.iter().skip(elements.len()) {
                                if !self.check_descriptor(rest, value, called, environment, 0)? {
                                    fixed = false;
                                    break;
                                }
                            }
                        }
                        fixed
                    }
                }
                None => false,
            },
            TypeDescriptor::Dictionary(None) => value.is_dict(),
            TypeDescriptor::Dictionary(Some((key_type, value_type))) => match value.as_dict() {
                Some(dictionary) => self.check_dictionary_elements(
                    descriptor,
                    (key_type, value_type),
                    dictionary,
                    called,
                    environment,
                    collection_id,
                )?,
                None => false,
            },
            TypeDescriptor::DictionaryShape { entries, rest } => match value.as_dict() {
                Some(dictionary) => {
                    let mut valid = dictionary.len() >= entries.len();
                    if rest.is_none() {
                        valid &= dictionary.len() == entries.len();
                    }
                    for (key, descriptor) in entries {
                        let key = match key {
                            ShapeKey::Int(key) => Key::Int(*key),
                            ShapeKey::String(key) => Key::String(key.to_handle()),
                        };
                        let Some(value) = dictionary.get(&key) else {
                            valid = false;
                            continue;
                        };
                        if !self.check_descriptor(descriptor, value, called, environment, 0)? {
                            valid = false;
                        }
                    }
                    if let Some((key_type, value_type)) = rest {
                        for (key, value) in dictionary.iter() {
                            if entries.iter().any(|(shape_key, _)| match (shape_key, key) {
                                (ShapeKey::Int(expected), KeyRef::Int(actual)) => {
                                    *expected == actual
                                }
                                (ShapeKey::String(expected), KeyRef::String(actual)) => {
                                    expected.as_bytes() == ByteStringObject::handle_bytes(actual)
                                }
                                (ShapeKey::String(expected), KeyRef::ShortString(actual)) => {
                                    expected.as_bytes() == actual.as_bytes()
                                }
                                _ => false,
                            }) {
                                continue;
                            }
                            let key_value = match key {
                                KeyRef::Int(key) => Value::int(key),
                                KeyRef::Bool(key) => Value::bool(key),
                                KeyRef::String(key) => Value::string(key.clone()),
                                KeyRef::ShortString(key) => Value::short_string(key),
                            };
                            if !self.check_descriptor(
                                key_type,
                                &key_value,
                                called,
                                environment,
                                0,
                            )? || !self.check_descriptor(
                                value_type,
                                value,
                                called,
                                environment,
                                0,
                            )? {
                                valid = false;
                            }
                        }
                    }
                    valid
                }
                None => false,
            },
            TypeDescriptor::Callable(None) => value.is_function(),
            TypeDescriptor::Callable(Some(expected)) => match value.as_function() {
                Some(function) => {
                    let actual_environment = self.defaulted_callable_type_environment(function)?;
                    let actual = self.callable_type_descriptor(function, actual_environment);
                    let concrete = self.substitute_descriptor(
                        &TypeDescriptor::Callable(Some(expected.clone())),
                        environment,
                        depth + 1,
                    );
                    let TypeDescriptor::Callable(Some(expected)) = concrete else {
                        // SAFETY: the surrounding invariant makes this path unreachable.
                        unsafe { unreachable_invariant("a callable descriptor stays callable") }
                    };
                    callable_descriptor_compatible(
                        self,
                        &actual,
                        &expected,
                        TypeEnvironmentId::default(),
                        depth + 1,
                    )?
                }
                None => false,
            },
            TypeDescriptor::Classname(inner) => match value.as_string_bytes() {
                Some(string) => {
                    let Some(actual) = self.parse_runtime_type_name(string) else {
                        return Ok(false);
                    };
                    let TypeDescriptor::Named {
                        name, arguments, ..
                    } = &actual
                    else {
                        return Ok(false);
                    };
                    let resolved = self.resolve_checked_name(name.clone())?;
                    match resolved {
                        Some(entry)
                            if matches!(
                                entry.kind,
                                SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface
                            ) =>
                        {
                            if arguments.is_some() {
                                let parameters = Rc::clone(
                                    &self.engine.tables.classes[entry.index as usize]
                                        .type_parameters,
                                );
                                self.bind_type_parameters(
                                    &parameters,
                                    arguments.as_deref(),
                                    TypeEnvironmentId::default(),
                                    name.as_bytes(),
                                )?;
                            }
                            let expected =
                                self.substitute_descriptor(inner, environment, depth + 1);
                            self.descriptor_is_subtype(
                                &actual,
                                &expected,
                                TypeEnvironmentId::default(),
                                depth + 1,
                            )?
                        }
                        _ => false,
                    }
                }
                None => false,
            },
            TypeDescriptor::Tuple(members) => match value.as_tuple() {
                Some(tuple) if tuple.len() == members.len() => {
                    let cache_id = match collection_id {
                        Some(id) => Some(id),
                        None => self.collection_type_check_id(descriptor),
                    };
                    if let Some(cache_id) = cache_id
                        && tuple.type_check(cache_id) == CollectionTypeCheck::Clean(cache_id)
                    {
                        return Ok(true);
                    }
                    let mut all = true;
                    for (member, element) in members.iter().zip(tuple.iter()) {
                        if !self.check_descriptor(member, element, called, environment, 0)? {
                            all = false;
                            break;
                        }
                    }
                    if all && let Some(cache_id) = cache_id {
                        tuple.mark_type_checked(cache_id);
                    }
                    all
                }
                _ => false,
            },
            TypeDescriptor::TupleRest { elements, rest } => match value.as_tuple() {
                Some(tuple) if tuple.len() >= elements.len() => {
                    let cache_id = match collection_id {
                        Some(id) => Some(id),
                        None => self.collection_type_check_id(descriptor),
                    };
                    if let Some(cache_id) = cache_id
                        && tuple.type_check(cache_id) == CollectionTypeCheck::Clean(cache_id)
                    {
                        return Ok(true);
                    }
                    let mut all = true;
                    for (element, value) in elements.iter().zip(tuple.iter()) {
                        if !self.check_descriptor(element, value, called, environment, 0)? {
                            all = false;
                            break;
                        }
                    }
                    if all {
                        for value in tuple.iter().skip(elements.len()) {
                            if !self.check_descriptor(rest, value, called, environment, 0)? {
                                all = false;
                                break;
                            }
                        }
                    }
                    if all && let Some(cache_id) = cache_id {
                        tuple.mark_type_checked(cache_id);
                    }
                    all
                }
                _ => false,
            },
            TypeDescriptor::TupleAny => value.is_tuple(),
            TypeDescriptor::Union(members) => {
                let mut any = false;
                for member in members {
                    if self.check_descriptor(member, value, called, environment, depth + 1)? {
                        any = true;
                        break;
                    }
                }
                any
            }
            TypeDescriptor::Intersection(members) => {
                let mut all = true;
                for member in members {
                    if !self.check_descriptor(member, value, called, environment, depth + 1)? {
                        all = false;
                        break;
                    }
                }
                all
            }
            TypeDescriptor::Negated(inner) => {
                !self.check_descriptor(inner, value, called, environment, depth + 1)?
            }
        })
    }

    pub(crate) fn check_declared_value(
        &mut self,
        descriptor: &TypeDescriptor,
        value: &Value,
    ) -> Result<bool, VirtualMachineControl> {
        self.check_descriptor(descriptor, value, None, TypeEnvironmentId::default(), 0)
    }

    /// Whether a thrown value satisfies a catch descriptor. A catch name is
    /// a symbol use, so the autoload chain may run here.
    pub(in crate::vm) fn descriptor_matches(
        &mut self,
        chunk: &Chunk,
        descriptor: DescriptorIndex,
        value: &Value,
        frame_index: usize,
    ) -> Result<bool, VirtualMachineControl> {
        let called = self.frames[frame_index].called_class.get();
        let environment = self.frames[frame_index].type_environment;
        self.check_descriptor(
            &chunk.type_descriptors[descriptor.index() as usize],
            value,
            called,
            environment,
            0,
        )
    }
}

fn range_lower_contains(expected: Option<i64>, actual: Option<i64>) -> bool {
    match (expected, actual) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(expected), Some(actual)) => actual >= expected,
    }
}

fn range_upper_contains(expected: Option<i64>, actual: Option<i64>) -> bool {
    match (expected, actual) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(expected), Some(actual)) => actual <= expected,
    }
}

fn callable_descriptor_compatible(
    vm: &mut VirtualMachine<'_>,
    actual: &FunctionTypeDescriptor,
    expected: &FunctionTypeDescriptor,
    environment: TypeEnvironmentId,
    depth: u32,
) -> Result<bool, VirtualMachineControl> {
    let actual_required = actual
        .parameters
        .iter()
        .filter(|parameter| !parameter.optional)
        .count();
    let expected_required = expected
        .parameters
        .iter()
        .filter(|parameter| !parameter.optional)
        .count();
    if actual_required > expected_required || actual.parameters.len() < expected.parameters.len() {
        return Ok(false);
    }
    for (actual, expected) in actual.parameters.iter().zip(&expected.parameters) {
        if !vm.descriptor_is_subtype(&expected.r#type, &actual.r#type, environment, depth + 1)? {
            return Ok(false);
        }
    }
    vm.descriptor_is_subtype(
        &actual.return_type,
        &expected.return_type,
        environment,
        depth + 1,
    )
}
