//! Reflection over live objects, callables, properties, and newtype layers.

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::bytecode::aliases::substitute;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::ClassLikeKind;
use crate::core::reflection::Operation;
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
use crate::engine::builtins::BuiltInCallable;
use crate::symbols::FunctionLocator;
use crate::symbols::SymbolKind;
use crate::value::Value;
use crate::value::function::CallTarget;
use crate::value::function::FuncId;
use crate::value::function::FunctionObject;
use crate::value::function::PresetArg;
use crate::value::heap::handle::ManagedRef;
use crate::value::newtype::NewtypeValueId;
use crate::value::object::ClassId;
use crate::value::object::InstanceObject;
use crate::value::object::TypeEnvironmentId;
use crate::vm::VirtualMachine;

pub(crate) fn object_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    values: &[Value],
) -> Result<Value, Throw> {
    let object = reflected_object(context, values)?;
    match operation {
        Operation::Class => {
            let name = context.vm.engine.tables.classes[object.class().0 as usize]
                .name
                .clone();
            objects::symbol(context, name)
        }
        Operation::EnumCase => object_enum_case(context, &object),
        Operation::Type => objects::r#type(context, object_type(context.vm, &object)),
        Operation::TypeEnvironment => {
            let reflected = object_type(context.vm, &object);
            type_environment(
                context,
                types::complete_class_environment(context.vm, &reflected),
            )
        }
        Operation::TypeArgument => {
            let parameter = type_parameter_argument(context, &arguments, 0)?;
            let reflected = object_type(context.vm, &object);
            let bindings = types::complete_class_environment(context.vm, &reflected);
            let Some((_, argument)) = bindings
                .into_iter()
                .find(|(candidate, _)| candidate == &parameter)
            else {
                return Ok(Value::null());
            };
            objects::r#type(context, argument)
        }
        Operation::IsInstanceOf => {
            let reflected = type_argument(context, &arguments, 0)?;
            let value = values
                .first()
                .expect("an object reflection retains its object");
            let accepted = context
                .vm
                .check_descriptor(
                    &reflected.descriptor,
                    value,
                    reflected.declaring_class,
                    TypeEnvironmentId::default(),
                    0,
                )
                .map_err(|control| context.vm.control_to_throw(control))?;
            Ok(Value::bool(accepted))
        }
        Operation::Specialization => {
            let target = class_like_argument(context, &arguments, 0)?;
            let reflected = object_type(context.vm, &object);
            let Some(specialized) = types::class_specialization(context.vm, &reflected, target)
            else {
                return Ok(Value::null());
            };
            objects::r#type(context, specialized)
        }
        Operation::PropertyValues => object_properties(context, &object),
        Operation::PropertyValue => object_property(context, arguments, &object),
        _ => Err(context.type_error("the operation is not valid for this reflected object")),
    }
}

pub(crate) fn property_value_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    property: &MemberKey,
    slot: usize,
    values: &[Value],
) -> Result<Value, Throw> {
    let object = reflected_object(context, values)?;
    let Some(info) = context.vm.engine.tables.classes[object.class().0 as usize]
        .slots
        .get(slot)
        .cloned()
    else {
        return Err(context.type_error("the reflected property slot is no longer present"));
    };
    if (info.declaring_class, &info.name) != (property.class, &property.name) {
        return Err(
            context.type_error("the reflected property slot no longer matches its declaration")
        );
    }
    match operation {
        Operation::Property => {
            objects::declaration(context, DeclarationKey::Member(property.clone()))
        }
        Operation::IsInitialized => Ok(Value::bool(!object.slot_is_uninitialized(slot))),
        Operation::DeclaredType => {
            let Some(descriptor) = info.declared_type else {
                return Ok(Value::null());
            };
            let class_name = context.vm.engine.tables.classes[info.declaring_class.0 as usize]
                .name
                .clone();
            let reflected = ReflectedType::owned(descriptor, GenericOwner::Symbol(class_name))
                .in_class(info.declaring_class);
            let object_type = object_type(context.vm, &object);
            let bindings = types::complete_class_environment(context.vm, &object_type);
            objects::r#type(
                context,
                types::resolve_type(context.vm, &reflected, &bindings, None),
            )
        }
        Operation::ValueType => {
            if object.slot_is_uninitialized(slot) {
                return Ok(Value::null());
            }
            let value = object.read_slot(slot);
            let descriptor = context.vm.runtime_type_descriptor(&value, 0);
            objects::r#type(context, ReflectedType::new(descriptor))
        }
        Operation::Value => {
            if object.slot_is_uninitialized(slot) {
                let control = context.vm.uninitialized_property_slot_error(&object, slot);
                return Err(context.vm.control_to_throw(control));
            }
            Ok(object.read_slot(slot))
        }
        _ => {
            Err(context.type_error("the operation is not valid for this reflected property value"))
        }
    }
}

pub(crate) fn callable_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    values: &[Value],
) -> Result<Value, Throw> {
    let function = reflected_callable(context, values)?;
    let callable = callable_key(context.vm, &function)
        .ok_or_else(|| context.type_error("the reflected callable has no declaration"))?;
    let info = support::callable_info(context.vm, &callable).ok_or_else(|| {
        context.type_error("the reflected callable declaration is no longer loaded")
    })?;
    match operation {
        Operation::CallableKind => callable_kind(context, &function, &callable, &info),
        Operation::Declaration => objects::declaration(context, callable_declaration(&callable)),
        Operation::Type => {
            let value = values
                .first()
                .expect("a callable reflection retains its callable");
            let descriptor = context.vm.runtime_type_descriptor(value, 0);
            objects::r#type(context, ReflectedType::new(descriptor))
        }
        Operation::TypeEnvironment => {
            let bindings = callable_environment(context.vm, &function, &callable, &info);
            type_environment(context, bindings)
        }
        Operation::BoundObject => Ok(function
            .this()
            .map_or_else(Value::null, |object| Value::object(object.clone()))),
        Operation::ScopeClass => {
            let Some(class) = function.scope() else {
                return Ok(Value::null());
            };
            let name = context.vm.engine.tables.classes[class.0 as usize]
                .name
                .clone();
            objects::symbol(context, name)
        }
        Operation::CalledType => called_type(context, &function, &callable, &info),
        Operation::Captures => capture_values(context, &function, &callable, &info),
        Operation::BoundArguments => bound_arguments(context, &function, &callable, &info),
        _ => {
            Err(context.type_error("the operation is not valid for this reflected callable value"))
        }
    }
}

pub(crate) fn capture_value_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    function: FuncId,
    position: usize,
    values: &[Value],
) -> Result<Value, Throw> {
    let value = reflected_value(context, values, "capture")?;
    match operation {
        Operation::Capture => objects::build(
            context,
            ReflectionData::Capture { function, position },
            Vec::new(),
        ),
        Operation::Type => {
            let descriptor = context.vm.runtime_type_descriptor(&value, 0);
            objects::r#type(context, ReflectedType::new(descriptor))
        }
        Operation::Value => Ok(value),
        _ => Err(context.type_error("the operation is not valid for this reflected capture value")),
    }
}

pub(crate) fn bound_argument_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    callable: &CallableKey,
    parameter: usize,
    values: &[Value],
) -> Result<Value, Throw> {
    let value = reflected_value(context, values, "bound argument")?;
    match operation {
        Operation::Parameter => objects::declaration(
            context,
            DeclarationKey::Parameter {
                callable: callable.clone(),
                position: parameter,
            },
        ),
        Operation::Type => {
            let descriptor = context.vm.runtime_type_descriptor(&value, 0);
            objects::r#type(context, ReflectedType::new(descriptor))
        }
        Operation::Value => Ok(value),
        _ => {
            Err(context.type_error("the operation is not valid for this reflected bound argument"))
        }
    }
}

pub(crate) fn newtype_value_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    identifier: NewtypeValueId,
    values: &[Value],
) -> Result<Value, Throw> {
    let value = reflected_value(context, values, "newtype value")?;
    if value.newtype_id() != Some(identifier) {
        return Err(context.type_error("the reflected newtype layer no longer matches its value"));
    }
    let tagged = context.vm.engine.tables.newtype_value(identifier);
    let declaration = &context.vm.engine.tables.newtypes[tagged.declaration.0 as usize];
    match operation {
        Operation::Declaration => objects::symbol(context, declaration.name.clone()),
        Operation::Type => {
            let descriptor = context.vm.runtime_type_descriptor(&value, 0);
            objects::r#type(context, ReflectedType::new(descriptor))
        }
        Operation::TypeEnvironment => {
            let descriptor = context.vm.runtime_type_descriptor(&value, 0);
            let reflected = ReflectedType::new(descriptor);
            type_environment(context, types::named_environment(context.vm, &reflected))
        }
        Operation::BackingValue => Ok(value.clone_with_newtype(tagged.parent)),
        _ => Err(context.type_error("the operation is not valid for this reflected newtype value")),
    }
}

fn reflected_object(
    context: &mut Context<'_, '_, '_>,
    values: &[Value],
) -> Result<ManagedRef<InstanceObject>, Throw> {
    values
        .first()
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| context.type_error("the reflected object value is no longer available"))
}

fn reflected_callable(
    context: &mut Context<'_, '_, '_>,
    values: &[Value],
) -> Result<ManagedRef<FunctionObject>, Throw> {
    values
        .first()
        .and_then(Value::as_function)
        .cloned()
        .ok_or_else(|| context.type_error("the reflected callable value is no longer available"))
}

fn reflected_value(
    context: &mut Context<'_, '_, '_>,
    values: &[Value],
    kind: &str,
) -> Result<Value, Throw> {
    values
        .first()
        .cloned()
        .ok_or_else(|| context.type_error(&format!("the reflected {kind} is no longer available")))
}

fn object_type(vm: &VirtualMachine<'_>, object: &ManagedRef<InstanceObject>) -> ReflectedType {
    ReflectedType::new(vm.runtime_type_descriptor(&Value::object(object.clone()), 0))
        .in_class(object.class())
}

fn object_enum_case(
    context: &mut Context<'_, '_, '_>,
    object: &ManagedRef<InstanceObject>,
) -> Result<Value, Throw> {
    let runtime = &context.vm.engine.tables.classes[object.class().0 as usize];
    if runtime.kind != ClassLikeKind::Enum {
        return Ok(Value::null());
    }
    let name = runtime
        .case_instances
        .borrow()
        .iter()
        .find_map(|(name, value)| {
            value
                .as_object()
                .is_some_and(|candidate| candidate.ptr_eq(object))
                .then(|| name.clone())
        });
    let Some(name) = name else {
        return Ok(Value::null());
    };
    objects::declaration(
        context,
        DeclarationKey::Member(MemberKey {
            class: object.class(),
            name,
            kind: MemberKind::EnumCase,
        }),
    )
}

fn object_properties(
    context: &mut Context<'_, '_, '_>,
    object: &ManagedRef<InstanceObject>,
) -> Result<Value, Throw> {
    let properties = context.vm.engine.tables.classes[object.class().0 as usize]
        .slots
        .iter()
        .enumerate()
        .map(|(slot, property)| {
            (
                slot,
                MemberKey {
                    class: property.declaring_class,
                    name: property.name.clone(),
                    kind: MemberKind::Property,
                },
            )
        })
        .collect::<Vec<_>>();
    let mut reflected = Vec::with_capacity(properties.len());
    for (slot, property) in properties {
        reflected.push(objects::build(
            context,
            ReflectionData::PropertyValue { property, slot },
            vec![Value::object(object.clone())],
        )?);
    }
    Ok(context.vec(reflected))
}

fn object_property(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    object: &ManagedRef<InstanceObject>,
) -> Result<Value, Throw> {
    let Some(value) = arguments.get(0) else {
        return Err(context.type_error("the reflected property argument is missing"));
    };
    let Some(ReflectionData::Member(property)) = objects::data(value) else {
        return Err(context.type_error("the reflection argument is not a reflected property"));
    };
    if property.kind != MemberKind::Property {
        return Err(context.type_error("the reflection argument is not a reflected property"));
    }
    let slot = context.vm.engine.tables.classes[object.class().0 as usize]
        .slots
        .iter()
        .position(|candidate| {
            (candidate.declaring_class, &candidate.name) == (property.class, &property.name)
        });
    let Some(slot) = slot else {
        return Ok(Value::null());
    };
    objects::build(
        context,
        ReflectionData::PropertyValue { property, slot },
        vec![Value::object(object.clone())],
    )
}

fn callable_key(
    vm: &VirtualMachine<'_>,
    function: &ManagedRef<FunctionObject>,
) -> Option<CallableKey> {
    match function.target() {
        CallTarget::User(identifier) => {
            let runtime = vm.engine.tables.functions.get(identifier.0 as usize)?;
            match runtime.locator {
                FunctionLocator::TopLevel(_)
                    if runtime.name.as_bytes().starts_with(b"{closure") =>
                {
                    Some(CallableKey::Closure(identifier))
                }
                FunctionLocator::TopLevel(_) => Some(CallableKey::Function(runtime.name.clone())),
                FunctionLocator::Method { class, method } => {
                    let declaration = runtime.unit.unit.classes.get(class as usize)?;
                    let method = declaration.methods.get(method as usize)?;
                    Some(CallableKey::Method {
                        class: runtime.declaring_class?,
                        name: method.name.clone(),
                    })
                }
            }
        }
        CallTarget::BuiltIn(identifier) => {
            if let Some(((class, name), _)) = vm
                .engine
                .tables
                .built_in_method_ids
                .iter()
                .find(|(_, candidate)| **candidate == identifier)
            {
                return Some(CallableKey::Method {
                    class: *class,
                    name: name.clone(),
                });
            }
            match vm
                .engine
                .tables
                .built_in_functions
                .get(identifier.0 as usize)?
            {
                BuiltInCallable::Function(spec) => {
                    let name = vm.heap().intern(spec.name.as_bytes());
                    vm.engine
                        .tables
                        .symbols
                        .get(&name)
                        .is_some_and(|entry| entry.kind == SymbolKind::Function)
                        .then_some(CallableKey::Function(name))
                }
                BuiltInCallable::Method { .. } => None,
            }
        }
    }
}

fn callable_kind(
    context: &mut Context<'_, '_, '_>,
    function: &ManagedRef<FunctionObject>,
    callable: &CallableKey,
    info: &support::CallableInfo,
) -> Result<Value, Throw> {
    let partial = function
        .presets()
        .iter()
        .enumerate()
        .any(|(position, preset)| match preset {
            PresetArg::Hole(order) => u32::try_from(position).ok() != Some(*order),
            PresetArg::Given(_) => true,
        });
    let name = if partial {
        b"Partial".as_slice()
    } else {
        match callable {
            CallableKey::Function(_) => b"Function".as_slice(),
            CallableKey::Method { .. } if function.this().is_some() => b"InstanceMethod".as_slice(),
            CallableKey::Method { .. } => b"StaticMethod".as_slice(),
            CallableKey::Closure(_) if info.is_short_closure => b"ShortClosure".as_slice(),
            CallableKey::Closure(_) => b"Closure".as_slice(),
        }
    };
    objects::enum_case(context, b"Whim\\Reflection\\Callable\\CallableKind", name)
}

fn callable_environment(
    vm: &VirtualMachine<'_>,
    function: &ManagedRef<FunctionObject>,
    callable: &CallableKey,
    info: &support::CallableInfo,
) -> Vec<(TypeParameterKey, ReflectedType)> {
    let mut bindings = function.this().map_or_else(
        || {
            function
                .called()
                .or(info.declaring_class)
                .or_else(|| function.scope())
                .map_or_else(Vec::new, |class| {
                    let environment =
                        callable_outer_environment(vm, function, info.type_parameters.len());
                    let reflected = class_type_from_environment(vm, class, environment);
                    types::complete_class_environment(vm, &reflected)
                })
        },
        |object| {
            let reflected = object_type(vm, object);
            types::complete_class_environment(vm, &reflected)
        },
    );
    if function.type_arguments_bound() {
        for (position, parameter) in info.type_parameters.iter().enumerate() {
            let Some(argument) = vm
                .type_environment_binding(function.type_environment(), &parameter.name)
                .cloned()
            else {
                continue;
            };
            bindings.push((
                TypeParameterKey {
                    owner: GenericOwner::Callable(callable.clone()),
                    position,
                },
                ReflectedType::new(argument),
            ));
        }
    }
    bindings
}

fn called_type(
    context: &mut Context<'_, '_, '_>,
    function: &ManagedRef<FunctionObject>,
    _callable: &CallableKey,
    info: &support::CallableInfo,
) -> Result<Value, Throw> {
    if let Some(object) = function.this() {
        return objects::r#type(context, object_type(context.vm, object));
    }
    let Some(class) = function.called() else {
        return Ok(Value::null());
    };
    let environment = callable_outer_environment(context.vm, function, info.type_parameters.len());
    objects::r#type(
        context,
        class_type_from_environment(context.vm, class, environment),
    )
}

fn capture_values(
    context: &mut Context<'_, '_, '_>,
    function: &ManagedRef<FunctionObject>,
    callable: &CallableKey,
    info: &support::CallableInfo,
) -> Result<Value, Throw> {
    let CallableKey::Closure(identifier) = callable else {
        return Ok(context.vec(Vec::new()));
    };
    let count = function.captures().len().min(info.capture_names.len());
    let mut values = Vec::with_capacity(count);
    for (position, value) in function.captures().iter().take(count).enumerate() {
        values.push(objects::build(
            context,
            ReflectionData::CaptureValue {
                function: *identifier,
                position,
            },
            vec![value.clone()],
        )?);
    }
    Ok(context.vec(values))
}

fn bound_arguments(
    context: &mut Context<'_, '_, '_>,
    function: &ManagedRef<FunctionObject>,
    callable: &CallableKey,
    info: &support::CallableInfo,
) -> Result<Value, Throw> {
    let mut values = Vec::new();
    for (position, preset) in function.presets().iter().enumerate() {
        let PresetArg::Given(value) = preset else {
            continue;
        };
        if value.is_uninitialized() || position >= info.parameters.len() {
            continue;
        }
        values.push(objects::build(
            context,
            ReflectionData::BoundArgument {
                callable: callable.clone(),
                parameter: position,
            },
            vec![value.clone()],
        )?);
    }
    Ok(context.vec(values))
}

fn callable_outer_environment(
    vm: &VirtualMachine<'_>,
    function: &ManagedRef<FunctionObject>,
    parameter_count: usize,
) -> TypeEnvironmentId {
    if !function.type_arguments_bound() {
        return function.type_environment();
    }
    let mut environment = function.type_environment();
    for _ in 0..parameter_count {
        let Some(parent) = vm.engine.tables.type_environments[environment.0 as usize].parent else {
            break;
        };
        environment = parent;
    }
    environment
}

fn class_type_from_environment(
    vm: &VirtualMachine<'_>,
    class: ClassId,
    environment: TypeEnvironmentId,
) -> ReflectedType {
    let runtime = &vm.engine.tables.classes[class.0 as usize];
    let arguments = (!runtime.type_parameters.is_empty()).then(|| {
        let mut bindings = Vec::with_capacity(runtime.type_parameters.len());
        let mut arguments = Vec::with_capacity(runtime.type_parameters.len());
        for parameter in runtime.type_parameters.iter() {
            let argument = vm
                .type_environment_binding(environment, &parameter.name)
                .cloned()
                .or_else(|| {
                    parameter
                        .default
                        .as_ref()
                        .map(|default| substitute(default, &bindings, 0))
                })
                .unwrap_or(TypeDescriptor::Mixed);
            bindings.push((parameter.name.clone(), argument.clone()));
            arguments.push(argument);
        }
        arguments
    });
    ReflectedType::new(TypeDescriptor::Named {
        name: runtime.name.clone(),
        arguments,
        recursive: false,
    })
    .in_class(class)
}

fn type_environment(
    context: &mut Context<'_, '_, '_>,
    bindings: Vec<(TypeParameterKey, ReflectedType)>,
) -> Result<Value, Throw> {
    objects::build(
        context,
        ReflectionData::TypeEnvironment(bindings),
        Vec::new(),
    )
}

fn type_parameter_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<TypeParameterKey, Throw> {
    let Some(value) = arguments.get(position) else {
        return Err(context.type_error("the reflected type parameter argument is missing"));
    };
    match objects::data(value) {
        Some(ReflectionData::TypeParameter(parameter)) => Ok(parameter),
        _ => Err(context.type_error("the reflection argument is not a reflected type parameter")),
    }
}

fn type_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<ReflectedType, Throw> {
    let Some(value) = arguments.get(position) else {
        return Err(context.type_error("the reflected type argument is missing"));
    };
    match objects::data(value) {
        Some(ReflectionData::Type(reflected)) => Ok(reflected),
        _ => Err(context.type_error("the reflection argument is not a reflected type")),
    }
}

fn class_like_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<ClassId, Throw> {
    let Some(value) = arguments.get(position) else {
        return Err(context.type_error("the reflected class-like argument is missing"));
    };
    let Some(ReflectionData::Symbol(name)) = objects::data(value) else {
        return Err(context.type_error("the reflection argument is not a reflected class-like"));
    };
    let Some(entry) = context.vm.engine.tables.symbols.get(&name) else {
        return Err(context.type_error("the reflected class-like is no longer loaded"));
    };
    if !matches!(
        entry.kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
    ) {
        return Err(context.type_error("the reflection argument is not a reflected class-like"));
    }
    Ok(ClassId(entry.index))
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
