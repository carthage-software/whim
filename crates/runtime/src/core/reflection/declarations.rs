//! Symbol, member, callable, and generic declaration reflection.

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::bytecode::chunk::descriptors::FunctionTypeDescriptor;
use crate::bytecode::chunk::descriptors::FunctionTypeParameterDescriptor;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledProperty;
use crate::bytecode::unit::Variance;
use crate::bytecode::unit::Visibility;
use crate::classes::MethodEntry;
use crate::classes::PropertyDefault;
use crate::classes::PropertyInfo;
use crate::classes::RuntimeBase;
use crate::classes::RuntimeClass;
use crate::core::reflection::Operation;
use crate::core::reflection::metadata;
use crate::core::reflection::model::CallableKey;
use crate::core::reflection::model::DeclarationKey;
use crate::core::reflection::model::GenericOwner;
use crate::core::reflection::model::MemberKey;
use crate::core::reflection::model::MemberKind;
use crate::core::reflection::model::ReflectedType;
use crate::core::reflection::model::ReflectionData;
use crate::core::reflection::model::TypeParameterKey;
use crate::core::reflection::objects;
use crate::core::reflection::support;
use crate::core::reflection::types;
use crate::symbols::SymbolKind;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::function::FuncId;
use crate::value::object::ClassId;
use crate::vm::VirtualMachine;

#[expect(clippy::too_many_lines, reason = "symbol reflection dispatch")]
pub(crate) fn symbol_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    name: &Atom,
) -> Result<Value, Throw> {
    let Some(entry) = context.vm.engine.tables.symbols.get(name).copied() else {
        return Err(context.type_error("the reflected symbol is no longer loaded"));
    };
    match operation {
        Operation::Name => Ok(Value::string(name.to_handle())),
        Operation::ShortName => Ok(context.string(short_name(name.as_bytes()))),
        Operation::NamespaceName => Ok(context.string(namespace_name(name.as_bytes()))),
        Operation::SymbolKind => symbol_kind(context, entry.kind),
        Operation::TypeParameters => type_parameters(context, &symbol_owner(name, entry.kind)),
        Operation::TypeParameter => {
            type_parameter(context, arguments, symbol_owner(name, entry.kind))
        }
        Operation::Type => symbol_type(context, name, entry.kind),
        Operation::AliasedType if entry.kind == SymbolKind::TypeAlias => {
            let alias = &context.vm.engine.tables.type_aliases[entry.index as usize];
            objects::r#type(
                context,
                ReflectedType::owned(alias.descriptor.clone(), GenericOwner::Symbol(name.clone())),
            )
        }
        Operation::BackingType if entry.kind == SymbolKind::Newtype => {
            let newtype = &context.vm.engine.tables.newtypes[entry.index as usize];
            objects::r#type(
                context,
                ReflectedType::owned(newtype.backing.clone(), GenericOwner::Symbol(name.clone())),
            )
        }
        Operation::Value if entry.kind == SymbolKind::Constant => context
            .vm
            .force_constant(entry.index, name.clone())
            .map_err(|control| context.vm.control_to_throw(control)),
        Operation::ValueType if entry.kind == SymbolKind::Constant => {
            let value = context
                .vm
                .force_constant(entry.index, name.clone())
                .map_err(|control| context.vm.control_to_throw(control))?;
            let descriptor = context.vm.runtime_type_descriptor(&value, 0);
            objects::r#type(context, ReflectedType::new(descriptor))
        }
        Operation::DirectBaseTypes => direct_base_types(context, ClassId(entry.index), None),
        Operation::BaseTypes if is_class_like(entry.kind) => {
            class_base_types(context, ClassId(entry.index))
        }
        Operation::PermittedSubtypeNames if is_class_like(entry.kind) => {
            Ok(permitted_subtype_names(context, ClassId(entry.index)))
        }
        Operation::DeclaredMethods => methods(context, ClassId(entry.index), true),
        Operation::Methods => methods(context, ClassId(entry.index), false),
        Operation::Method => method(context, arguments, ClassId(entry.index)),
        Operation::DeclaredProperties => properties(context, ClassId(entry.index), true),
        Operation::Properties => properties(context, ClassId(entry.index), false),
        Operation::Property => property(context, arguments, ClassId(entry.index)),
        Operation::DeclaredConstants => constants(context, ClassId(entry.index), true),
        Operation::Constants => constants(context, ClassId(entry.index), false),
        Operation::Constant => constant(context, arguments, ClassId(entry.index)),
        Operation::IsAbstract if entry.kind == SymbolKind::Class => {
            Ok(Value::bool(class(context.vm, entry.index).is_abstract))
        }
        Operation::IsFinal if entry.kind == SymbolKind::Class => {
            Ok(Value::bool(class(context.vm, entry.index).is_final))
        }
        Operation::IsReadonly if entry.kind == SymbolKind::Class => {
            Ok(Value::bool(class(context.vm, entry.index).is_readonly))
        }
        Operation::IsInstantiable if entry.kind == SymbolKind::Class => Ok(Value::bool(
            is_instantiable(context.vm, ClassId(entry.index)),
        )),
        Operation::IsCloneable if entry.kind == SymbolKind::Class => Ok(Value::bool(
            class(context.vm, entry.index)
                .built_in_state_hooks
                .is_empty(),
        )),
        Operation::AttributeDefinition if entry.kind == SymbolKind::Class => {
            let class = ClassId(entry.index);
            if context.vm.engine.tables.classes[class.0 as usize]
                .attribute_flags
                .is_some()
            {
                objects::build(
                    context,
                    ReflectionData::AttributeDefinition(class),
                    Vec::new(),
                )
            } else {
                Ok(Value::null())
            }
        }
        Operation::ParentType if entry.kind == SymbolKind::Class => {
            let class = ClassId(entry.index);
            let parent = context.vm.engine.tables.classes[class.0 as usize]
                .direct_bases
                .iter()
                .find(|base| {
                    context.vm.engine.tables.classes[base.class.0 as usize].kind
                        == ClassLikeKind::Class
                })
                .cloned();
            reflect_optional_base(context, class, parent)
        }
        Operation::DirectInterfaceTypes
            if matches!(entry.kind, SymbolKind::Class | SymbolKind::Enum) =>
        {
            direct_base_types(
                context,
                ClassId(entry.index),
                Some(ClassLikeKind::Interface),
            )
        }
        Operation::InterfaceTypes if matches!(entry.kind, SymbolKind::Class | SymbolKind::Enum) => {
            interface_types(context, ClassId(entry.index))
        }
        Operation::Constructor if entry.kind == SymbolKind::Class => named_method(
            context,
            ClassId(entry.index),
            context.vm.engine.tables.constructor_name.clone(),
        ),
        Operation::Destructor if entry.kind == SymbolKind::Class => named_method(
            context,
            ClassId(entry.index),
            context.vm.engine.tables.destructor_name.clone(),
        ),
        Operation::DirectParentTypes if entry.kind == SymbolKind::Interface => direct_base_types(
            context,
            ClassId(entry.index),
            Some(ClassLikeKind::Interface),
        ),
        Operation::ParentTypes if entry.kind == SymbolKind::Interface => {
            interface_types(context, ClassId(entry.index))
        }
        Operation::BackingType if entry.kind == SymbolKind::Enum => {
            enum_backing_type(context, ClassId(entry.index))
        }
        Operation::Cases if entry.kind == SymbolKind::Enum => {
            enum_cases(context, ClassId(entry.index))
        }
        Operation::Case if entry.kind == SymbolKind::Enum => {
            enum_case(context, arguments, ClassId(entry.index))
        }
        Operation::Parameters
        | Operation::Parameter
        | Operation::RequiredParameterCount
        | Operation::ReturnType
        | Operation::CallableType
            if entry.kind == SymbolKind::Function =>
        {
            callable_dispatch(
                context,
                arguments,
                operation,
                &CallableKey::Function(name.clone()),
            )
        }
        _ => Err(context.type_error("the operation is not valid for this reflected symbol")),
    }
}

#[expect(clippy::too_many_lines, reason = "member reflection dispatch")]
pub(crate) fn member_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    member: &MemberKey,
) -> Result<Value, Throw> {
    match operation {
        Operation::Name => Ok(Value::string(member.name.to_handle())),
        Operation::DeclaringType => {
            let name = context.vm.engine.tables.classes[member.class.0 as usize]
                .name
                .clone();
            objects::symbol(context, name)
        }
        Operation::Visibility => {
            let Some(value) = member_visibility(context.vm, member) else {
                return Err(context.type_error("the reflected member is no longer loaded"));
            };
            visibility(context, value)
        }
        Operation::IsStatic if member.kind == MemberKind::Method => {
            let Some(method) = method_entry(context.vm, member) else {
                return Err(context.type_error("the reflected method is no longer loaded"));
            };
            Ok(Value::bool(method.is_static))
        }
        Operation::IsAbstract if member.kind == MemberKind::Method => {
            let Some(method) = method_entry(context.vm, member) else {
                return Err(context.type_error("the reflected method is no longer loaded"));
            };
            Ok(Value::bool(method.is_abstract))
        }
        Operation::IsFinal if member.kind == MemberKind::Method => {
            let Some(method) = method_entry(context.vm, member) else {
                return Err(context.type_error("the reflected method is no longer loaded"));
            };
            Ok(Value::bool(method.is_final))
        }
        Operation::IsConstructor if member.kind == MemberKind::Method => Ok(Value::bool(
            member.name == context.vm.engine.tables.constructor_name,
        )),
        Operation::IsDestructor if member.kind == MemberKind::Method => Ok(Value::bool(
            member.name == context.vm.engine.tables.destructor_name,
        )),
        Operation::Prototypes => prototypes(context, member),
        Operation::Type if member.kind != MemberKind::Property => member_type(context, member),
        Operation::IsStatic if member.kind == MemberKind::Property => {
            let Some((is_static, _, _)) = property_info(context.vm, member) else {
                return Err(context.type_error("the reflected property is no longer loaded"));
            };
            Ok(Value::bool(is_static))
        }
        Operation::IsReadonly if member.kind == MemberKind::Property => {
            let Some((_, _, property)) = property_info(context.vm, member) else {
                return Err(context.type_error("the reflected property is no longer loaded"));
            };
            Ok(Value::bool(property.is_readonly))
        }
        Operation::IsPromoted if member.kind == MemberKind::Property => Ok(Value::bool(
            compiled_property(context.vm, member).is_some_and(|value| value.is_promoted),
        )),
        Operation::DeclaredType
            if matches!(
                member.kind,
                MemberKind::Property | MemberKind::ClassConstant
            ) =>
        {
            member_declared_type(context, member)
        }
        Operation::HasDefaultValue if member.kind == MemberKind::Property => {
            Ok(Value::bool(property_has_default(context.vm, member)))
        }
        Operation::DefaultValue if member.kind == MemberKind::Property => {
            property_default(context, member)
        }
        Operation::IsStaticInitialized if member.kind == MemberKind::Property => {
            static_initialized(context, member)
        }
        Operation::StaticValue if member.kind == MemberKind::Property => {
            static_value(context, member)
        }
        Operation::Value if member.kind == MemberKind::ClassConstant => {
            class_constant_value(context, member)
        }
        Operation::ValueType if member.kind == MemberKind::ClassConstant => {
            let value = force_class_constant(context, member)?;
            let descriptor = context.vm.runtime_type_descriptor(&value, 0);
            objects::r#type(context, ReflectedType::new(descriptor))
        }
        Operation::Enum if member.kind == MemberKind::EnumCase => {
            let name = context.vm.engine.tables.classes[member.class.0 as usize]
                .name
                .clone();
            objects::symbol(context, name)
        }
        Operation::BackingValue if member.kind == MemberKind::EnumCase => {
            let value = context.vm.engine.tables.classes[member.class.0 as usize]
                .enum_case(&member.name)
                .and_then(|case| case.backing.clone());
            Ok(value.unwrap_or_else(Value::null))
        }
        Operation::Value if member.kind == MemberKind::EnumCase => context
            .vm
            .enum_case_value(member.class, member.name.clone())
            .ok_or_else(|| context.type_error("the reflected enum case is no longer loaded")),
        Operation::Parameters
        | Operation::Parameter
        | Operation::RequiredParameterCount
        | Operation::ReturnType
        | Operation::CallableType
        | Operation::TypeParameters
        | Operation::TypeParameter
            if member.kind == MemberKind::Method =>
        {
            callable_dispatch(
                context,
                arguments,
                operation,
                &CallableKey::Method {
                    class: member.class,
                    name: member.name.clone(),
                },
            )
        }
        _ => Err(context.type_error("the operation is not valid for this reflected member")),
    }
}

pub(crate) fn callable_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    callable: &CallableKey,
) -> Result<Value, Throw> {
    let Some(info) = support::callable_info(context.vm, callable) else {
        return Err(context.type_error("the reflected callable is no longer loaded"));
    };
    match operation {
        Operation::Name => Ok(Value::string(info.name.to_handle())),
        Operation::Parameters => {
            let mut parameters = Vec::with_capacity(info.parameters.len());
            for position in 0..info.parameters.len() {
                parameters.push(parameter_reflection(context, callable, position)?);
            }
            Ok(context.vec(parameters))
        }
        Operation::Parameter => callable_parameter(context, arguments, callable, &info),
        Operation::RequiredParameterCount => Ok(index_value(info.required_parameters())),
        Operation::ReturnType => {
            let Some(descriptor) = info.return_type else {
                return Ok(Value::null());
            };
            objects::r#type(
                context,
                ReflectedType::owned(descriptor, GenericOwner::Callable(callable.clone()))
                    .in_optional_class(info.declaring_class),
            )
        }
        Operation::CallableType => callable_type(context, arguments, callable, &info),
        Operation::TypeParameters => {
            type_parameters(context, &GenericOwner::Callable(callable.clone()))
        }
        Operation::TypeParameter => {
            type_parameter(context, arguments, GenericOwner::Callable(callable.clone()))
        }
        _ => Err(context.type_error("the operation is not valid for this reflected callable")),
    }
}

pub(crate) fn parameter_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    callable: &CallableKey,
    position: usize,
) -> Result<Value, Throw> {
    let Some(info) = support::callable_info(context.vm, callable) else {
        return Err(context.type_error("the reflected callable is no longer loaded"));
    };
    let Some(parameter) = info.parameters.get(position) else {
        return Err(context.type_error("the reflected parameter is no longer declared"));
    };
    match operation {
        Operation::Name => Ok(Value::string(parameter.name.to_handle())),
        Operation::Position => Ok(index_value(position)),
        Operation::DeclaringCallable => {
            objects::declaration(context, callable_declaration(callable))
        }
        Operation::DeclaredType => optional_declared_type(
            context,
            parameter.declared_type.as_ref(),
            callable,
            info.declaring_class,
        ),
        Operation::Type => {
            let Some(descriptor) = parameter.declared_type.as_ref() else {
                return Ok(Value::null());
            };
            let reflected =
                ReflectedType::owned(descriptor.clone(), GenericOwner::Callable(callable.clone()))
                    .in_optional_class(info.declaring_class);
            let bindings = environment_argument(context, &arguments, 0)?;
            objects::r#type(
                context,
                types::resolve_type(context.vm, &reflected, &bindings, None),
            )
        }
        Operation::IsOptional => Ok(Value::bool(parameter.has_default)),
        Operation::HasDefaultValue => Ok(Value::bool(parameter.default.is_some())),
        Operation::DefaultValue => {
            let Some(default) = parameter.default.as_ref() else {
                return Ok(Value::null());
            };
            support::evaluate_initializer(context.vm, default, info.unit.as_ref())
        }
        Operation::IsSensitive => Ok(Value::bool(parameter.sensitive)),
        Operation::PromotedProperty => promoted_property(context, callable, parameter),
        _ => Err(context.type_error("the operation is not valid for this reflected parameter")),
    }
}

pub(crate) fn closure_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    function: FuncId,
) -> Result<Value, Throw> {
    let callable = CallableKey::Closure(function);
    let Some(info) = support::callable_info(context.vm, &callable) else {
        return Err(context.type_error("the reflected closure is no longer loaded"));
    };
    match operation {
        Operation::IsShort => Ok(Value::bool(info.is_short_closure)),
        Operation::Captures => {
            let mut captures = Vec::with_capacity(info.capture_names.len());
            for position in 0..info.capture_names.len() {
                captures.push(objects::build(
                    context,
                    ReflectionData::Capture { function, position },
                    Vec::new(),
                )?);
            }
            Ok(context.vec(captures))
        }
        _ => callable_dispatch(context, arguments, operation, &callable),
    }
}

pub(crate) fn capture_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    function: FuncId,
    position: usize,
) -> Result<Value, Throw> {
    let Some(info) = support::callable_info(context.vm, &CallableKey::Closure(function)) else {
        return Err(context.type_error("the reflected closure is no longer loaded"));
    };
    let Some(name) = info.capture_names.get(position) else {
        return Err(context.type_error("the reflected capture is no longer declared"));
    };
    match operation {
        Operation::Name => Ok(context.string(
            name.as_bytes()
                .strip_prefix(b"$")
                .unwrap_or(name.as_bytes()),
        )),
        Operation::Position => Ok(index_value(position)),
        Operation::IsReceiver => Ok(Value::bool(info.captures_this && position == 0)),
        Operation::Location => Ok(Value::null()),
        _ => Err(context.type_error("the operation is not valid for this reflected capture")),
    }
}

pub(crate) fn type_parameter_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    key: &TypeParameterKey,
) -> Result<Value, Throw> {
    let Some(parameters) = support::type_parameters(context.vm, &key.owner) else {
        return Err(context.type_error("the reflected type-parameter owner is no longer loaded"));
    };
    let Some(parameter) = parameters.get(key.position) else {
        return Err(context.type_error("the reflected type parameter is no longer declared"));
    };
    match operation {
        Operation::Name => Ok(Value::string(parameter.name.to_handle())),
        Operation::Position => Ok(index_value(key.position)),
        Operation::DeclaringDeclaration => {
            objects::declaration(context, generic_owner_declaration(&key.owner))
        }
        Operation::Variance => variance(context, parameter.variance),
        Operation::Bounds => {
            let parent = ReflectedType::owned(TypeDescriptor::Mixed, key.owner.clone());
            let mut bounds = Vec::with_capacity(parameter.bounds.len());
            for bound in &parameter.bounds {
                bounds.push(objects::r#type(context, types::child_type(bound, &parent))?);
            }
            Ok(context.vec(bounds))
        }
        Operation::Default => {
            let Some(default) = parameter.default.as_ref() else {
                return Ok(Value::null());
            };
            objects::r#type(
                context,
                ReflectedType::owned(default.clone(), key.owner.clone()),
            )
        }
        Operation::Type => objects::r#type(
            context,
            ReflectedType::owned(
                TypeDescriptor::Parameter(parameter.name.clone()),
                key.owner.clone(),
            ),
        ),
        Operation::Location => {
            let unit = support::generic_owner_unit(context.vm, &key.owner);
            let Some(unit) = unit.as_deref() else {
                return Ok(Value::null());
            };
            metadata::reflect_location(context, unit, parameter.span)
        }
        _ => {
            Err(context.type_error("the operation is not valid for this reflected type parameter"))
        }
    }
}

fn symbol_type(
    context: &mut Context<'_, '_, '_>,
    name: &Atom,
    kind: SymbolKind,
) -> Result<Value, Throw> {
    let owner = GenericOwner::Symbol(name.clone());
    let arguments = own_parameter_references(context.vm, &owner);
    let descriptor = TypeDescriptor::Named {
        name: name.clone(),
        arguments: non_empty_arguments(arguments),
        recursive: false,
    };
    let mut reflected = ReflectedType::owned(descriptor, owner);
    if is_class_like(kind) {
        reflected.declaring_class = context.vm.resolve_class_symbol(name);
    }
    objects::r#type(context, reflected)
}

fn member_type(context: &mut Context<'_, '_, '_>, member: &MemberKey) -> Result<Value, Throw> {
    let class_name = context.vm.engine.tables.classes[member.class.0 as usize]
        .name
        .clone();
    let class_owner = GenericOwner::Symbol(class_name.clone());
    let member_owner = GenericOwner::Callable(CallableKey::Method {
        class: member.class,
        name: member.name.clone(),
    });
    let class_arguments = own_parameter_references(context.vm, &class_owner);
    let member_arguments = (member.kind == MemberKind::Method)
        .then(|| own_parameter_references(context.vm, &member_owner))
        .and_then(non_empty_arguments);
    let descriptor = TypeDescriptor::Member {
        class: class_name,
        class_arguments: non_empty_arguments(class_arguments),
        member: member.name.clone(),
        member_arguments,
    };
    objects::r#type(
        context,
        ReflectedType::owned(descriptor, member_owner).in_class(member.class),
    )
}

fn direct_base_types(
    context: &mut Context<'_, '_, '_>,
    class: ClassId,
    filter: Option<ClassLikeKind>,
) -> Result<Value, Throw> {
    let runtime = &context.vm.engine.tables.classes[class.0 as usize];
    let owner = GenericOwner::Symbol(runtime.name.clone());
    let bases = runtime
        .direct_bases
        .iter()
        .filter(|base| {
            filter.is_none_or(|kind| {
                context.vm.engine.tables.classes[base.class.0 as usize].kind == kind
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut values = Vec::with_capacity(bases.len());
    for base in bases {
        let name = context.vm.engine.tables.classes[base.class.0 as usize]
            .name
            .clone();
        values.push(objects::r#type(
            context,
            ReflectedType::owned(
                TypeDescriptor::Named {
                    name,
                    arguments: base.type_arguments,
                    recursive: false,
                },
                owner.clone(),
            )
            .in_class(base.class),
        )?);
    }
    Ok(context.vec(values))
}

fn class_base_types(context: &mut Context<'_, '_, '_>, class: ClassId) -> Result<Value, Throw> {
    let own = class_reflected_type(context.vm, class);
    let mut ids = Vec::new();
    collect_bases(context.vm, class, &mut ids);
    let mut values = Vec::with_capacity(ids.len());
    for base in ids {
        if let Some(specialization) = types::class_specialization(context.vm, &own, base) {
            values.push(objects::r#type(context, specialization)?);
        }
    }
    Ok(context.vec(values))
}

fn interface_types(context: &mut Context<'_, '_, '_>, class: ClassId) -> Result<Value, Throw> {
    let own = class_reflected_type(context.vm, class);
    let interfaces = context.vm.engine.tables.classes[class.0 as usize]
        .interfaces
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let mut values = Vec::with_capacity(interfaces.len());
    for interface in interfaces {
        if let Some(specialization) = types::class_specialization(context.vm, &own, interface) {
            values.push(objects::r#type(context, specialization)?);
        }
    }
    Ok(context.vec(values))
}

fn class_reflected_type(vm: &VirtualMachine<'_>, class: ClassId) -> ReflectedType {
    let name = vm.engine.tables.classes[class.0 as usize].name.clone();
    let owner = GenericOwner::Symbol(name.clone());
    ReflectedType::owned(
        TypeDescriptor::Named {
            name,
            arguments: non_empty_arguments(own_parameter_references(vm, &owner)),
            recursive: false,
        },
        owner,
    )
    .in_class(class)
}

fn reflect_optional_base(
    context: &mut Context<'_, '_, '_>,
    class: ClassId,
    base: Option<RuntimeBase>,
) -> Result<Value, Throw> {
    let Some(base) = base else {
        return Ok(Value::null());
    };
    let owner = GenericOwner::Symbol(
        context.vm.engine.tables.classes[class.0 as usize]
            .name
            .clone(),
    );
    let name = context.vm.engine.tables.classes[base.class.0 as usize]
        .name
        .clone();
    objects::r#type(
        context,
        ReflectedType::owned(
            TypeDescriptor::Named {
                name,
                arguments: base.type_arguments,
                recursive: false,
            },
            owner,
        )
        .in_class(base.class),
    )
}

fn permitted_subtype_names(context: &Context<'_, '_, '_>, class: ClassId) -> Value {
    let Some(names) = context.vm.engine.tables.classes[class.0 as usize]
        .sealed_to
        .as_ref()
    else {
        return Value::null();
    };
    context.vec(names.iter().map(|name| Value::string(name.to_handle())))
}

fn methods(
    context: &mut Context<'_, '_, '_>,
    class: ClassId,
    declared: bool,
) -> Result<Value, Throw> {
    let runtime = &context.vm.engine.tables.classes[class.0 as usize];
    let mut entries = runtime
        .methods()
        .filter(|(_, entry)| !declared || entry.declaring_class == class)
        .map(|(name, entry)| (name.clone(), entry))
        .collect::<Vec<_>>();
    if declared {
        for ((owner, name), entry) in &runtime.private_methods {
            if *owner == class && !entries.iter().any(|(candidate, _)| candidate == name) {
                entries.push((name.clone(), *entry));
            }
        }
    }
    entries.sort_unstable_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
    let mut values = Vec::with_capacity(entries.len());
    for (name, entry) in entries {
        values.push(member_reflection(
            context,
            MemberKey {
                class: entry.declaring_class,
                name,
                kind: MemberKind::Method,
            },
        )?);
    }
    Ok(context.vec(values))
}

fn method(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    class: ClassId,
) -> Result<Value, Throw> {
    let name = context.vm.intern(arguments.bytes(0));
    named_method(context, class, name)
}

fn named_method(
    context: &mut Context<'_, '_, '_>,
    class: ClassId,
    name: Atom,
) -> Result<Value, Throw> {
    let Some(entry) = context.vm.engine.tables.classes[class.0 as usize].method(&name) else {
        return Ok(Value::null());
    };
    member_reflection(
        context,
        MemberKey {
            class: entry.declaring_class,
            name,
            kind: MemberKind::Method,
        },
    )
}

fn properties(
    context: &mut Context<'_, '_, '_>,
    class: ClassId,
    declared: bool,
) -> Result<Value, Throw> {
    let runtime = &context.vm.engine.tables.classes[class.0 as usize];
    let mut keys = runtime
        .slots
        .iter()
        .chain(runtime.statics_info.iter())
        .filter(|property| !declared || property.declaring_class == class)
        .map(|property| MemberKey {
            class: property.declaring_class,
            name: property.name.clone(),
            kind: MemberKind::Property,
        })
        .collect::<Vec<_>>();
    keys.sort_unstable_by(|left, right| {
        left.name
            .as_bytes()
            .cmp(right.name.as_bytes())
            .then_with(|| left.class.0.cmp(&right.class.0))
    });
    keys.dedup();
    reflect_member_keys(context, keys)
}

fn property(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    class: ClassId,
) -> Result<Value, Throw> {
    let name = context.vm.intern(arguments.bytes(0));
    let runtime = &context.vm.engine.tables.classes[class.0 as usize];
    let info = runtime
        .slot_names
        .get(&name)
        .and_then(|slot| runtime.slots.get(*slot as usize))
        .or_else(|| {
            runtime
                .static_names
                .get(&name)
                .and_then(|slot| runtime.statics_info.get(*slot as usize))
        });
    let Some(info) = info else {
        return Ok(Value::null());
    };
    member_reflection(
        context,
        MemberKey {
            class: info.declaring_class,
            name,
            kind: MemberKind::Property,
        },
    )
}

fn constants(
    context: &mut Context<'_, '_, '_>,
    class: ClassId,
    declared: bool,
) -> Result<Value, Throw> {
    let runtime = &context.vm.engine.tables.classes[class.0 as usize];
    let mut keys = runtime
        .constants()
        .filter(|(_, constant)| !declared || constant.declaring_class == class)
        .map(|(name, constant)| MemberKey {
            class: constant.declaring_class,
            name: name.clone(),
            kind: MemberKind::ClassConstant,
        })
        .collect::<Vec<_>>();
    keys.sort_unstable_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
    reflect_member_keys(context, keys)
}

fn constant(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    class: ClassId,
) -> Result<Value, Throw> {
    let name = context.vm.intern(arguments.bytes(0));
    let Some(entry) = context.vm.engine.tables.classes[class.0 as usize].constant(&name) else {
        return Ok(Value::null());
    };
    member_reflection(
        context,
        MemberKey {
            class: entry.declaring_class,
            name,
            kind: MemberKind::ClassConstant,
        },
    )
}

fn enum_cases(context: &mut Context<'_, '_, '_>, class: ClassId) -> Result<Value, Throw> {
    let names = context.vm.engine.tables.classes[class.0 as usize]
        .enum_cases
        .iter()
        .map(|case| case.name.clone())
        .collect::<Vec<_>>();
    let mut values = Vec::with_capacity(names.len());
    for name in names {
        values.push(member_reflection(
            context,
            MemberKey {
                class,
                name,
                kind: MemberKind::EnumCase,
            },
        )?);
    }
    Ok(context.vec(values))
}

fn enum_case(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    class: ClassId,
) -> Result<Value, Throw> {
    let name = context.vm.intern(arguments.bytes(0));
    if context.vm.engine.tables.classes[class.0 as usize]
        .enum_case(&name)
        .is_none()
    {
        return Ok(Value::null());
    }
    member_reflection(
        context,
        MemberKey {
            class,
            name,
            kind: MemberKind::EnumCase,
        },
    )
}

fn enum_backing_type(context: &mut Context<'_, '_, '_>, class: ClassId) -> Result<Value, Throw> {
    let name = context.vm.intern(b"value");
    let runtime = &context.vm.engine.tables.classes[class.0 as usize];
    let descriptor = runtime
        .slot_names
        .get(&name)
        .and_then(|slot| runtime.slots.get(*slot as usize))
        .and_then(|property| property.declared_type.clone());
    let Some(descriptor) = descriptor else {
        return Ok(Value::null());
    };
    objects::r#type(context, ReflectedType::new(descriptor))
}

fn is_instantiable(vm: &VirtualMachine<'_>, class: ClassId) -> bool {
    let runtime = &vm.engine.tables.classes[class.0 as usize];
    if runtime.kind != ClassLikeKind::Class || runtime.is_abstract {
        return false;
    }
    runtime
        .method(&vm.engine.tables.constructor_name)
        .is_none_or(|constructor| constructor.visibility == Visibility::Public)
}

fn callable_parameter(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    callable: &CallableKey,
    info: &support::CallableInfo,
) -> Result<Value, Throw> {
    let argument = arguments.get(0).expect("validated argument");
    let position = argument.as_int().map_or_else(
        || {
            let bytes = argument.as_string_bytes().expect("validated argument");
            info.parameters
                .iter()
                .position(|parameter| parameter.name.as_bytes() == bytes)
        },
        |position| usize::try_from(position).ok(),
    );
    let Some(position) = position.filter(|position| *position < info.parameters.len()) else {
        return Ok(Value::null());
    };
    parameter_reflection(context, callable, position)
}

fn callable_type(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    callable: &CallableKey,
    info: &support::CallableInfo,
) -> Result<Value, Throw> {
    let return_type = info.return_type.clone().unwrap_or(TypeDescriptor::Mixed);
    let descriptor = TypeDescriptor::Callable(Some(FunctionTypeDescriptor {
        parameters: info
            .parameters
            .iter()
            .map(|parameter| FunctionTypeParameterDescriptor {
                r#type: parameter
                    .declared_type
                    .clone()
                    .unwrap_or(TypeDescriptor::Mixed),
                optional: parameter.has_default,
            })
            .collect(),
        return_type: Box::new(return_type),
    }));
    let reflected = ReflectedType::owned(descriptor, GenericOwner::Callable(callable.clone()))
        .in_optional_class(info.declaring_class);
    let environment = environment_argument(context, &arguments, 0)?;
    let called = optional_reflected_type_argument(context, &arguments, 1)?;
    objects::r#type(
        context,
        types::resolve_type(context.vm, &reflected, &environment, called.as_ref()),
    )
}

fn type_parameters(
    context: &mut Context<'_, '_, '_>,
    owner: &GenericOwner,
) -> Result<Value, Throw> {
    let parameters = support::type_parameters(context.vm, owner).unwrap_or_default();
    let mut values = Vec::with_capacity(parameters.len());
    for position in 0..parameters.len() {
        values.push(objects::build(
            context,
            ReflectionData::TypeParameter(TypeParameterKey {
                owner: owner.clone(),
                position,
            }),
            Vec::new(),
        )?);
    }
    Ok(context.vec(values))
}

fn type_parameter(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    owner: GenericOwner,
) -> Result<Value, Throw> {
    let parameters = support::type_parameters(context.vm, &owner).unwrap_or_default();
    let argument = arguments.get(0).expect("validated argument");
    let position = argument.as_int().map_or_else(
        || {
            let bytes = argument.as_string_bytes().expect("validated argument");
            parameters
                .iter()
                .position(|parameter| parameter.name.as_bytes() == bytes)
        },
        |position| usize::try_from(position).ok(),
    );
    let Some(position) = position.filter(|position| *position < parameters.len()) else {
        return Ok(Value::null());
    };
    objects::build(
        context,
        ReflectionData::TypeParameter(TypeParameterKey { owner, position }),
        Vec::new(),
    )
}

fn prototypes(context: &mut Context<'_, '_, '_>, member: &MemberKey) -> Result<Value, Throw> {
    let mut bases = Vec::new();
    collect_bases(context.vm, member.class, &mut bases);
    let mut keys = Vec::new();
    for base in bases {
        let candidate = MemberKey {
            class: base,
            name: member.name.clone(),
            kind: member.kind,
        };
        if member_exists(context.vm, &candidate) {
            let declaring = effective_declaring_member(context.vm, &candidate);
            if !keys.contains(&declaring) && declaring != *member {
                keys.push(declaring);
            }
        }
    }
    reflect_member_keys(context, keys)
}

fn member_declared_type(
    context: &mut Context<'_, '_, '_>,
    member: &MemberKey,
) -> Result<Value, Throw> {
    let descriptor = match member.kind {
        MemberKind::Property => {
            let Some((_, _, property)) = property_info(context.vm, member) else {
                return Err(context.type_error("the reflected property is no longer loaded"));
            };
            property.declared_type.clone()
        }
        MemberKind::ClassConstant => context.vm.engine.tables.classes[member.class.0 as usize]
            .constant(&member.name)
            .and_then(|entry| entry.declared_type.clone()),
        _ => None,
    };
    let Some(descriptor) = descriptor else {
        return Ok(Value::null());
    };
    let class_name = context.vm.engine.tables.classes[member.class.0 as usize]
        .name
        .clone();
    objects::r#type(
        context,
        ReflectedType::owned(descriptor, GenericOwner::Symbol(class_name)).in_class(member.class),
    )
}

fn property_default(context: &mut Context<'_, '_, '_>, member: &MemberKey) -> Result<Value, Throw> {
    let Some((is_static, _, info)) = property_info(context.vm, member) else {
        return Err(context.type_error("the reflected property is no longer loaded"));
    };
    if is_static {
        let default =
            compiled_property(context.vm, member).and_then(|property| property.default.clone());
        let Some(default) = default else {
            return Ok(Value::null());
        };
        let unit = context.vm.engine.tables.classes[member.class.0 as usize]
            .attribute_unit
            .clone();
        return support::evaluate_initializer(context.vm, &default, unit.as_ref());
    }
    let default = info.default.clone();
    match default {
        Some(PropertyDefault::Value(value)) => Ok(value),
        Some(PropertyDefault::Pending {
            context: unit,
            class_position,
            property_position,
        }) => {
            let initializer = unit.unit.classes[class_position as usize].properties
                [property_position as usize]
                .default
                .clone();
            let Some(initializer) = initializer else {
                return Ok(Value::null());
            };
            support::evaluate_initializer(context.vm, &initializer, Some(&unit))
        }
        None => Ok(Value::null()),
    }
}

fn property_has_default(vm: &VirtualMachine<'_>, member: &MemberKey) -> bool {
    property_info(vm, member).is_some_and(|(is_static, _, info)| {
        if is_static {
            compiled_property(vm, member).is_some_and(|property| property.default.is_some())
        } else {
            info.default.is_some()
        }
    })
}

fn static_initialized(
    context: &mut Context<'_, '_, '_>,
    member: &MemberKey,
) -> Result<Value, Throw> {
    let Some((is_static, slot, _)) = property_info(context.vm, member) else {
        return Err(context.type_error("the reflected property is no longer loaded"));
    };
    if !is_static {
        return Ok(Value::bool(false));
    }
    let initialized = !context.vm.engine.tables.classes[member.class.0 as usize]
        .statics
        .borrow()[slot]
        .is_uninitialized();
    Ok(Value::bool(initialized))
}

fn static_value(context: &mut Context<'_, '_, '_>, member: &MemberKey) -> Result<Value, Throw> {
    let Some((is_static, slot, _)) = property_info(context.vm, member) else {
        return Err(context.type_error("the reflected property is no longer loaded"));
    };
    if !is_static {
        return Err(context.type_error("the reflected property is not static"));
    }
    let value = context.vm.engine.tables.classes[member.class.0 as usize]
        .statics
        .borrow()[slot]
        .clone();
    if value.is_uninitialized() {
        return Err(context.type_error("the reflected static property is not initialized"));
    }
    Ok(value)
}

fn class_constant_value(
    context: &mut Context<'_, '_, '_>,
    member: &MemberKey,
) -> Result<Value, Throw> {
    force_class_constant(context, member)
}

fn force_class_constant(
    context: &mut Context<'_, '_, '_>,
    member: &MemberKey,
) -> Result<Value, Throw> {
    context
        .vm
        .force_class_constant(member.class, member.name.clone())
        .map_err(|control| context.vm.control_to_throw(control))
}

fn promoted_property(
    context: &mut Context<'_, '_, '_>,
    callable: &CallableKey,
    parameter: &CompiledParameter,
) -> Result<Value, Throw> {
    let CallableKey::Method { class, name } = callable else {
        return Ok(Value::null());
    };
    if name != &context.vm.engine.tables.constructor_name {
        return Ok(Value::null());
    }
    let member = MemberKey {
        class: *class,
        name: parameter.name.clone(),
        kind: MemberKind::Property,
    };
    if !compiled_property(context.vm, &member).is_some_and(|property| property.is_promoted) {
        return Ok(Value::null());
    }
    member_reflection(context, member)
}

fn optional_declared_type(
    context: &mut Context<'_, '_, '_>,
    descriptor: Option<&TypeDescriptor>,
    callable: &CallableKey,
    class: Option<ClassId>,
) -> Result<Value, Throw> {
    let Some(descriptor) = descriptor else {
        return Ok(Value::null());
    };
    objects::r#type(
        context,
        ReflectedType::owned(descriptor.clone(), GenericOwner::Callable(callable.clone()))
            .in_optional_class(class),
    )
}

fn property_info<'a>(
    vm: &'a VirtualMachine<'_>,
    member: &MemberKey,
) -> Option<(bool, usize, &'a PropertyInfo)> {
    let runtime = &vm.engine.tables.classes[member.class.0 as usize];
    if let Some((slot, info)) = runtime
        .slots
        .iter()
        .enumerate()
        .find(|(_, info)| (info.declaring_class, &info.name) == (member.class, &member.name))
    {
        return Some((false, slot, info));
    }
    if let Some((slot, info)) = runtime
        .statics_info
        .iter()
        .enumerate()
        .find(|(_, info)| (info.declaring_class, &info.name) == (member.class, &member.name))
    {
        return Some((true, slot, info));
    }
    None
}

fn compiled_property<'a>(
    vm: &'a VirtualMachine<'_>,
    member: &MemberKey,
) -> Option<&'a CompiledProperty> {
    let runtime = &vm.engine.tables.classes[member.class.0 as usize];
    let unit = runtime.attribute_unit.as_ref()?;
    support::compiled_class(unit, &runtime.name)?
        .properties
        .iter()
        .find(|property| property.name == member.name)
}

fn method_entry(vm: &VirtualMachine<'_>, member: &MemberKey) -> Option<MethodEntry> {
    let runtime = &vm.engine.tables.classes[member.class.0 as usize];
    runtime
        .private_methods
        .get(&(member.class, member.name.clone()))
        .copied()
        .or_else(|| runtime.method(&member.name))
        .filter(|entry| entry.declaring_class == member.class)
}

fn member_visibility(vm: &VirtualMachine<'_>, member: &MemberKey) -> Option<Visibility> {
    match member.kind {
        MemberKind::Method => Some(method_entry(vm, member)?.visibility),
        MemberKind::Property => Some(property_info(vm, member)?.2.visibility),
        MemberKind::ClassConstant => vm.engine.tables.classes[member.class.0 as usize]
            .constant(&member.name)
            .map(|entry| entry.visibility),
        MemberKind::EnumCase => Some(Visibility::Public),
    }
}

fn member_exists(vm: &VirtualMachine<'_>, member: &MemberKey) -> bool {
    match member.kind {
        MemberKind::Method => vm.engine.tables.classes[member.class.0 as usize]
            .method(&member.name)
            .is_some(),
        MemberKind::Property => property_info(vm, member).is_some(),
        MemberKind::ClassConstant => vm.engine.tables.classes[member.class.0 as usize]
            .constant(&member.name)
            .is_some(),
        MemberKind::EnumCase => vm.engine.tables.classes[member.class.0 as usize]
            .enum_case(&member.name)
            .is_some(),
    }
}

fn effective_declaring_member(vm: &VirtualMachine<'_>, member: &MemberKey) -> MemberKey {
    let declaring = match member.kind {
        MemberKind::Method => vm.engine.tables.classes[member.class.0 as usize]
            .method(&member.name)
            .map(|entry| entry.declaring_class),
        MemberKind::Property => property_info(vm, member).map(|value| value.2.declaring_class),
        MemberKind::ClassConstant => vm.engine.tables.classes[member.class.0 as usize]
            .constant(&member.name)
            .map(|entry| entry.declaring_class),
        MemberKind::EnumCase => Some(member.class),
    }
    .unwrap_or(member.class);
    MemberKey {
        class: declaring,
        name: member.name.clone(),
        kind: member.kind,
    }
}

fn environment_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<Vec<(TypeParameterKey, ReflectedType)>, Throw> {
    let Some(value) = arguments
        .get(position)
        .filter(|value| !value.is_null() && !value.is_uninitialized())
    else {
        return Ok(Vec::new());
    };
    match objects::data(value) {
        Some(ReflectionData::TypeEnvironment(bindings)) => Ok(bindings),
        _ => Err(context.type_error("the reflection argument is not a type environment")),
    }
}

fn optional_reflected_type_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<Option<ReflectedType>, Throw> {
    let Some(value) = arguments
        .get(position)
        .filter(|value| !value.is_null() && !value.is_uninitialized())
    else {
        return Ok(None);
    };
    match objects::data(value) {
        Some(ReflectionData::Type(reflected)) => Ok(Some(reflected)),
        _ => Err(context.type_error("the reflection argument is not a reflected type")),
    }
}

fn own_parameter_references(vm: &VirtualMachine<'_>, owner: &GenericOwner) -> Vec<TypeDescriptor> {
    support::type_parameters(vm, owner)
        .unwrap_or_default()
        .into_iter()
        .map(|parameter| TypeDescriptor::Parameter(parameter.name))
        .collect()
}

fn non_empty_arguments(arguments: Vec<TypeDescriptor>) -> Option<Vec<TypeDescriptor>> {
    (!arguments.is_empty()).then_some(arguments)
}

fn parameter_reflection(
    context: &mut Context<'_, '_, '_>,
    callable: &CallableKey,
    position: usize,
) -> Result<Value, Throw> {
    objects::declaration(
        context,
        DeclarationKey::Parameter {
            callable: callable.clone(),
            position,
        },
    )
}

fn member_reflection(context: &mut Context<'_, '_, '_>, member: MemberKey) -> Result<Value, Throw> {
    objects::declaration(context, DeclarationKey::Member(member))
}

fn reflect_member_keys(
    context: &mut Context<'_, '_, '_>,
    keys: Vec<MemberKey>,
) -> Result<Value, Throw> {
    let mut values = Vec::with_capacity(keys.len());
    for key in keys {
        values.push(member_reflection(context, key)?);
    }
    Ok(context.vec(values))
}

fn callable_declaration(callable: &CallableKey) -> DeclarationKey {
    match callable {
        CallableKey::Function(name) => DeclarationKey::Symbol(name.clone()),
        CallableKey::Method { class, name } => DeclarationKey::Member(MemberKey {
            class: *class,
            name: name.clone(),
            kind: MemberKind::Method,
        }),
        CallableKey::Closure(function) => DeclarationKey::Closure(*function),
    }
}

fn generic_owner_declaration(owner: &GenericOwner) -> DeclarationKey {
    match owner {
        GenericOwner::Symbol(name) => DeclarationKey::Symbol(name.clone()),
        GenericOwner::Callable(callable) => callable_declaration(callable),
    }
}

fn collect_bases(vm: &VirtualMachine<'_>, class: ClassId, output: &mut Vec<ClassId>) {
    for base in &vm.engine.tables.classes[class.0 as usize].direct_bases {
        if !output.contains(&base.class) {
            output.push(base.class);
            collect_bases(vm, base.class, output);
        }
    }
}

fn class<'vm>(vm: &'vm VirtualMachine<'_>, index: u32) -> &'vm RuntimeClass {
    &vm.engine.tables.classes[index as usize]
}

const fn is_class_like(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
    )
}

fn short_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b'\\').next().unwrap_or(name)
}

fn namespace_name(name: &[u8]) -> &[u8] {
    name.iter()
        .rposition(|byte| *byte == b'\\')
        .map_or(b"".as_slice(), |position| &name[..position])
}

fn symbol_owner(name: &Atom, kind: SymbolKind) -> GenericOwner {
    if kind == SymbolKind::Function {
        GenericOwner::Callable(CallableKey::Function(name.clone()))
    } else {
        GenericOwner::Symbol(name.clone())
    }
}

fn symbol_kind(context: &mut Context<'_, '_, '_>, kind: SymbolKind) -> Result<Value, Throw> {
    let case = match kind {
        SymbolKind::Class => b"Class".as_slice(),
        SymbolKind::Interface => b"Interface".as_slice(),
        SymbolKind::Enum => b"Enum".as_slice(),
        SymbolKind::TypeAlias => b"TypeAlias".as_slice(),
        SymbolKind::Newtype => b"Newtype".as_slice(),
        SymbolKind::Function => b"Function".as_slice(),
        SymbolKind::Constant => b"Constant".as_slice(),
    };
    objects::enum_case(context, b"Whim\\Symbol\\SymbolKind", case)
}

fn visibility(context: &mut Context<'_, '_, '_>, visibility: Visibility) -> Result<Value, Throw> {
    let case = match visibility {
        Visibility::Public => b"Public".as_slice(),
        Visibility::Protected => b"Protected".as_slice(),
        Visibility::Private => b"Private".as_slice(),
    };
    objects::enum_case(context, b"Whim\\Reflection\\Member\\Visibility", case)
}

fn variance(context: &mut Context<'_, '_, '_>, variance: Variance) -> Result<Value, Throw> {
    let case = match variance {
        Variance::Invariant => b"Invariant".as_slice(),
        Variance::Covariant => b"Covariant".as_slice(),
        Variance::Contravariant => b"Contravariant".as_slice(),
    };
    objects::enum_case(context, b"Whim\\Reflection\\Generic\\Variance", case)
}

fn index_value(position: usize) -> Value {
    Value::int(i64::try_from(position).expect("reflection positions fit in i64"))
}

trait ReflectedTypeClass {
    fn in_optional_class(self, class: Option<ClassId>) -> Self;
}

impl ReflectedTypeClass for ReflectedType {
    fn in_optional_class(mut self, class: Option<ClassId>) -> Self {
        self.declaring_class = class;
        self
    }
}
