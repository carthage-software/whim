//! Reflected type operations.

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::bytecode::aliases::substitute;
use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledTypeParameter;
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
use crate::symbols::SymbolKind;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::object::ClassId;
use crate::value::object::TypeEnvironmentId;
use crate::vm::VirtualMachine;
use crate::vm::types::descriptor_same;

pub(crate) fn dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    match operation {
        Operation::TypeKind => type_kind(context, &reflected.descriptor),
        Operation::IsResolved => Ok(Value::bool(is_resolved(&reflected.descriptor))),
        Operation::TypeId => {
            require_resolved(context, reflected)?;
            Ok(objects::type_id(context, &reflected.descriptor))
        }
        Operation::ToString => Ok(context.string(
            context
                .vm
                .render_descriptor(&reflected.descriptor)
                .as_bytes(),
        )),
        Operation::Resolve => resolve(context, arguments, reflected),
        Operation::Accepts => accepts(context, arguments, reflected),
        Operation::Equals => equals(context, arguments, reflected),
        Operation::IsSubtypeOf => is_subtype_of(context, arguments, reflected),
        Operation::Value => literal_value(context, &reflected.descriptor),
        Operation::LowerBound => Ok(range_bound(&reflected.descriptor, true)),
        Operation::UpperBound => Ok(range_bound(&reflected.descriptor, false)),
        Operation::Declaration => declaration(context, reflected),
        Operation::TypeArguments => type_arguments(context, reflected),
        Operation::TypeEnvironment => type_environment(context, reflected),
        Operation::IsRecursiveReference => Ok(recursive_reference(&reflected.descriptor)),
        Operation::ClassLike => class_like(context, reflected),
        Operation::Specialization => specialization(context, arguments, reflected),
        Operation::BaseTypes => base_types(context, reflected),
        Operation::ClassType => member_class_type(context, reflected),
        Operation::Parameter => type_parameter(context, reflected),
        Operation::DeclaringType => static_declaring_type(context, reflected),
        Operation::Types => types(context, reflected),
        Operation::InnerType => inner_type(context, reflected),
        Operation::Parameters => function_parameters(context, reflected),
        Operation::ReturnType => function_return_type(context, reflected),
        Operation::KeyType => array_type(context, reflected, ArrayPart::Key),
        Operation::ValueType => array_type(context, reflected, ArrayPart::Value),
        Operation::RestType => rest_type(context, reflected),
        Operation::Entries => shape_entries(context, reflected),
        Operation::RestKeyType => array_type(context, reflected, ArrayPart::RestKey),
        Operation::RestValueType => array_type(context, reflected, ArrayPart::RestValue),
        Operation::ObjectType => object_type(context, reflected),
        _ => Err(context.type_error("the operation is not valid for this reflected type")),
    }
}

pub(crate) fn function_parameter_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    position: usize,
    reflected: &ReflectedType,
    optional: bool,
) -> Result<Value, Throw> {
    match operation {
        Operation::Position => Ok(index_value(position)),
        Operation::Type => objects::r#type(context, reflected.clone()),
        Operation::IsOptional => Ok(Value::bool(optional)),
        _ => {
            Err(context
                .type_error("the operation is not valid for this reflected function parameter"))
        }
    }
}

pub(crate) fn shape_entry_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    key: &ShapeKey,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    match operation {
        Operation::Key => Ok(match key {
            ShapeKey::Int(key) => Value::int(*key),
            ShapeKey::String(key) => Value::string(key.to_handle()),
        }),
        Operation::Type => objects::r#type(context, reflected.clone()),
        _ => {
            Err(context
                .type_error("the operation is not valid for this reflected dict-shape entry"))
        }
    }
}

pub(crate) fn environment_dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
    bindings: &[(TypeParameterKey, ReflectedType)],
) -> Result<Value, Throw> {
    match operation {
        Operation::Bindings => {
            let mut values = Vec::with_capacity(bindings.len());
            for (parameter, argument) in bindings {
                values.push(objects::build(
                    context,
                    ReflectionData::TypeBinding {
                        parameter: parameter.clone(),
                        argument: argument.clone(),
                    },
                    Vec::new(),
                )?);
            }
            Ok(context.vec(values))
        }
        Operation::Binding => {
            let wanted = reflected_parameter_argument(context, &arguments, 0)?;
            let Some((_, argument)) = bindings.iter().find(|(parameter, _)| parameter == &wanted)
            else {
                return Ok(Value::null());
            };
            objects::r#type(context, argument.clone())
        }
        _ => {
            Err(context
                .type_error("the operation is not valid for this reflected type environment"))
        }
    }
}

pub(crate) fn binding_dispatch(
    context: &mut Context<'_, '_, '_>,
    operation: Operation,
    parameter: &TypeParameterKey,
    argument: &ReflectedType,
) -> Result<Value, Throw> {
    match operation {
        Operation::Parameter => objects::build(
            context,
            ReflectionData::TypeParameter(parameter.clone()),
            Vec::new(),
        ),
        Operation::Argument => objects::r#type(context, argument.clone()),
        _ => Err(context.type_error("the operation is not valid for this reflected type binding")),
    }
}

fn type_kind(
    context: &mut Context<'_, '_, '_>,
    descriptor: &TypeDescriptor,
) -> Result<Value, Throw> {
    let case = match descriptor {
        TypeDescriptor::Mixed => "Mixed",
        TypeDescriptor::Never => "Never",
        TypeDescriptor::Void => "Void",
        TypeDescriptor::Null => "Null",
        TypeDescriptor::Bool => "Bool",
        TypeDescriptor::Int => "Int",
        TypeDescriptor::Float => "Float",
        TypeDescriptor::String => "String",
        TypeDescriptor::Object => "Object",
        TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_) => "Literal",
        TypeDescriptor::IntRange { .. } => "IntegerRange",
        TypeDescriptor::Named { .. } => "Named",
        TypeDescriptor::Member { .. } => "Member",
        TypeDescriptor::Parameter(_) => "TypeParameter",
        TypeDescriptor::StaticClass => "Static",
        TypeDescriptor::Union(_) => "Union",
        TypeDescriptor::Intersection(_) => "Intersection",
        TypeDescriptor::Negated(_) => "Negated",
        TypeDescriptor::Callable(_) => "Function",
        TypeDescriptor::Array(_) => "Array",
        TypeDescriptor::Vector(_) => "Vec",
        TypeDescriptor::VectorShape { .. } => "VecShape",
        TypeDescriptor::Dictionary(_) => "Dict",
        TypeDescriptor::DictionaryShape { .. } => "DictShape",
        TypeDescriptor::Classname(_) => "Classname",
        TypeDescriptor::Tuple(_) | TypeDescriptor::TupleRest { .. } | TypeDescriptor::TupleAny => {
            "Tuple"
        }
        TypeDescriptor::Wildcard => "Wildcard",
    };
    objects::enum_case(
        context,
        b"Whim\\Reflection\\Type\\TypeKind",
        case.as_bytes(),
    )
}

fn resolve(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let bindings = optional_environment_argument(context, &arguments, 0)?;
    let called = optional_type_argument(context, &arguments, 1)?;
    let resolved = resolve_type(context.vm, reflected, &bindings, called.as_ref());
    objects::r#type(context, resolved)
}

fn accepts(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    require_resolved(context, reflected)?;
    let value = arguments.get(0).expect("validated argument");
    let accepts = context
        .vm
        .check_descriptor(
            &reflected.descriptor,
            value,
            reflected.declaring_class,
            TypeEnvironmentId::default(),
            0,
        )
        .map_err(|control| context.vm.control_to_throw(control))?;
    Ok(Value::bool(accepts))
}

fn equals(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let other = reflected_type_argument(context, &arguments, 0)?;
    Ok(Value::bool(descriptor_same(
        &reflected.descriptor,
        &other.descriptor,
    )))
}

fn is_subtype_of(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    require_resolved(context, reflected)?;
    let other = reflected_type_argument(context, &arguments, 0)?;
    require_resolved(context, &other)?;
    let result = context
        .vm
        .descriptor_is_subtype(
            &reflected.descriptor,
            &other.descriptor,
            TypeEnvironmentId::default(),
            0,
        )
        .map_err(|control| context.vm.control_to_throw(control))?;
    Ok(Value::bool(result))
}

fn literal_value(
    context: &mut Context<'_, '_, '_>,
    descriptor: &TypeDescriptor,
) -> Result<Value, Throw> {
    Ok(match descriptor {
        TypeDescriptor::TrueLiteral => Value::bool(true),
        TypeDescriptor::FalseLiteral => Value::bool(false),
        TypeDescriptor::IntLiteral(value) => Value::int(*value),
        TypeDescriptor::FloatLiteral(value) => Value::float(*value),
        TypeDescriptor::StringLiteral(value) => Value::string(value.to_handle()),
        _ => return Err(context.type_error("the reflected type is not a literal type")),
    })
}

fn range_bound(descriptor: &TypeDescriptor, lower: bool) -> Value {
    let TypeDescriptor::IntRange { min, max } = descriptor else {
        return Value::null();
    };
    if lower { *min } else { *max }.map_or_else(Value::null, Value::int)
}

fn declaration(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    match &reflected.descriptor {
        TypeDescriptor::Named { name, .. } => {
            objects::declaration(context, DeclarationKey::Symbol(name.clone()))
        }
        TypeDescriptor::Member { class, member, .. } => {
            let Some(class) = class_id(context.vm, class) else {
                return Err(context.type_error("the reflected class is not loaded"));
            };
            let runtime = &context.vm.engine.tables.classes[class.0 as usize];
            let kind = if runtime.method(member).is_some() {
                MemberKind::Method
            } else if runtime.constant(member).is_some() {
                MemberKind::ClassConstant
            } else if runtime.enum_case(member).is_some() {
                MemberKind::EnumCase
            } else {
                return Err(context.type_error("the reflected member type is not loaded"));
            };
            objects::declaration(
                context,
                DeclarationKey::Member(MemberKey {
                    class,
                    name: member.clone(),
                    kind,
                }),
            )
        }
        _ => Err(context.type_error("the reflected type does not name a declaration")),
    }
}

fn type_arguments(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let arguments = match &reflected.descriptor {
        TypeDescriptor::Named { arguments, .. } => arguments.as_deref().unwrap_or_default(),
        TypeDescriptor::Member {
            member_arguments, ..
        } => member_arguments.as_deref().unwrap_or_default(),
        _ => &[],
    };
    reflect_descriptors(context, arguments, reflected)
}

fn type_environment(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let bindings = named_environment(context.vm, reflected);
    objects::build(
        context,
        ReflectionData::TypeEnvironment(bindings),
        Vec::new(),
    )
}

const fn recursive_reference(descriptor: &TypeDescriptor) -> Value {
    Value::bool(matches!(
        descriptor,
        TypeDescriptor::Named {
            recursive: true,
            ..
        }
    ))
}

fn class_like(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::Named { name, .. } = &reflected.descriptor else {
        return Err(context.type_error("the reflected type is not a class-like type"));
    };
    let Some(entry) = context.vm.engine.tables.symbols.get(name) else {
        return Err(context.type_error("the reflected class-like type is not loaded"));
    };
    if !matches!(
        entry.kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
    ) {
        return Err(context.type_error("the reflected type does not name a class-like"));
    }
    objects::symbol(context, name.clone())
}

fn specialization(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let declaration = reflected_symbol_argument(context, &arguments, 0)?;
    let Some(entry) = context.vm.engine.tables.symbols.get(&declaration) else {
        return Ok(Value::null());
    };
    if !matches!(
        entry.kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
    ) {
        return Ok(Value::null());
    }
    let Some(specialized) = class_specialization(context.vm, reflected, ClassId(entry.index))
    else {
        return Ok(Value::null());
    };
    objects::r#type(context, specialized)
}

fn base_types(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let Some((class, _)) = named_class(context.vm, reflected) else {
        return Err(context.type_error("the reflected type is not a loaded class-like type"));
    };
    let mut classes = Vec::new();
    collect_base_ids(context.vm, class, &mut classes);
    let mut reflected_bases = Vec::with_capacity(classes.len());
    for base in classes {
        if let Some(specialized) = class_specialization(context.vm, reflected, base) {
            reflected_bases.push(objects::r#type(context, specialized)?);
        }
    }
    Ok(context.vec(reflected_bases))
}

fn member_class_type(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::Member {
        class,
        class_arguments,
        ..
    } = &reflected.descriptor
    else {
        return Err(context.type_error("the reflected type is not a member type"));
    };
    objects::r#type(
        context,
        ReflectedType {
            descriptor: TypeDescriptor::Named {
                name: class.clone(),
                arguments: class_arguments.clone(),
                recursive: false,
            },
            owner: reflected.owner.clone(),
            declaring_class: reflected.declaring_class,
        },
    )
}

fn type_parameter(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::Parameter(name) = &reflected.descriptor else {
        return Err(context.type_error("the reflected type is not a type-parameter reference"));
    };
    let Some(owner) = reflected.owner.as_ref() else {
        return Err(context.type_error("the reflected type parameter has no declaration owner"));
    };
    let Some(parameters) = support::type_parameters(context.vm, owner) else {
        return Err(context.type_error("the reflected type parameter owner is not loaded"));
    };
    let Some(position) = parameters
        .iter()
        .position(|parameter| parameter.name == *name)
    else {
        return Err(context.type_error("the reflected type parameter is not declared"));
    };
    objects::build(
        context,
        ReflectionData::TypeParameter(TypeParameterKey {
            owner: owner.clone(),
            position,
        }),
        Vec::new(),
    )
}

fn static_declaring_type(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    if !matches!(reflected.descriptor, TypeDescriptor::StaticClass) {
        return Err(context.type_error("the reflected type is not `static`"));
    }
    let Some(class) = reflected.declaring_class else {
        return Err(context.type_error("the reflected `static` type has no declaring class"));
    };
    let name = context.vm.engine.tables.classes[class.0 as usize]
        .name
        .clone();
    objects::symbol(context, name)
}

fn types(context: &mut Context<'_, '_, '_>, reflected: &ReflectedType) -> Result<Value, Throw> {
    let types = match &reflected.descriptor {
        TypeDescriptor::Union(types)
        | TypeDescriptor::Intersection(types)
        | TypeDescriptor::Tuple(types) => types,
        TypeDescriptor::VectorShape { elements, .. }
        | TypeDescriptor::TupleRest { elements, .. } => elements,
        _ => return Err(context.type_error("the reflected type has no type list")),
    };
    reflect_descriptors(context, types, reflected)
}

fn inner_type(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::Negated(inner) = &reflected.descriptor else {
        return Err(context.type_error("the reflected type is not negated"));
    };
    reflect_child(context, inner, reflected)
}

fn function_parameters(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::Callable(signature) = &reflected.descriptor else {
        return Err(context.type_error("the reflected type is not callable"));
    };
    let Some(signature) = signature else {
        return Ok(context.vec(Vec::new()));
    };
    let mut parameters = Vec::with_capacity(signature.parameters.len());
    for (position, parameter) in signature.parameters.iter().enumerate() {
        parameters.push(objects::build(
            context,
            ReflectionData::FunctionTypeParameter {
                position,
                r#type: child_type(&parameter.r#type, reflected),
                optional: parameter.optional,
            },
            Vec::new(),
        )?);
    }
    Ok(context.vec(parameters))
}

fn function_return_type(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::Callable(signature) = &reflected.descriptor else {
        return Err(context.type_error("the reflected type is not callable"));
    };
    let Some(signature) = signature else {
        return Ok(Value::null());
    };
    reflect_child(context, &signature.return_type, reflected)
}

#[derive(Clone, Copy)]
enum ArrayPart {
    Key,
    Value,
    RestKey,
    RestValue,
}

fn array_type(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
    part: ArrayPart,
) -> Result<Value, Throw> {
    let descriptor = match part {
        ArrayPart::Key => match &reflected.descriptor {
            TypeDescriptor::Array(Some((key, _))) | TypeDescriptor::Dictionary(Some((key, _))) => {
                Some(key.as_ref())
            }
            TypeDescriptor::Array(None) | TypeDescriptor::Dictionary(None) => None,
            _ => return Err(context.type_error("the reflected array has no key type")),
        },
        ArrayPart::Value => match &reflected.descriptor {
            TypeDescriptor::Array(Some((_, value)))
            | TypeDescriptor::Dictionary(Some((_, value)))
            | TypeDescriptor::Vector(Some(value)) => Some(value.as_ref()),
            TypeDescriptor::Array(None)
            | TypeDescriptor::Dictionary(None)
            | TypeDescriptor::Vector(None) => None,
            _ => return Err(context.type_error("the reflected array has no value type")),
        },
        ArrayPart::RestKey => match &reflected.descriptor {
            TypeDescriptor::DictionaryShape {
                rest: Some((key, _)),
                ..
            } => Some(key.as_ref()),
            TypeDescriptor::DictionaryShape { rest: None, .. } => None,
            _ => return Err(context.type_error("the reflected dict shape has no rest key type")),
        },
        ArrayPart::RestValue => match &reflected.descriptor {
            TypeDescriptor::DictionaryShape {
                rest: Some((_, value)),
                ..
            } => Some(value.as_ref()),
            TypeDescriptor::DictionaryShape { rest: None, .. } => None,
            _ => {
                return Err(context.type_error("the reflected dict shape has no rest value type"));
            }
        },
    };
    let Some(descriptor) = descriptor else {
        return Ok(Value::null());
    };
    reflect_child(context, descriptor, reflected)
}

fn rest_type(context: &mut Context<'_, '_, '_>, reflected: &ReflectedType) -> Result<Value, Throw> {
    let rest = match &reflected.descriptor {
        TypeDescriptor::VectorShape { rest, .. } => rest.as_deref(),
        TypeDescriptor::TupleRest { rest, .. } => Some(rest.as_ref()),
        TypeDescriptor::Tuple(_) | TypeDescriptor::TupleAny => None,
        _ => return Err(context.type_error("the reflected type has no repeated tail")),
    };
    let Some(rest) = rest else {
        return Ok(Value::null());
    };
    reflect_child(context, rest, reflected)
}

fn shape_entries(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::DictionaryShape { entries, .. } = &reflected.descriptor else {
        return Err(context.type_error("the reflected type is not a dict shape"));
    };
    let mut values = Vec::with_capacity(entries.len());
    for (key, descriptor) in entries {
        values.push(objects::build(
            context,
            ReflectionData::DictShapeEntry {
                key: key.clone(),
                r#type: child_type(descriptor, reflected),
            },
            Vec::new(),
        )?);
    }
    Ok(context.vec(values))
}

fn object_type(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<Value, Throw> {
    let TypeDescriptor::Classname(inner) = &reflected.descriptor else {
        return Err(context.type_error("the reflected type is not a classname type"));
    };
    reflect_child(context, inner, reflected)
}

pub(crate) fn named_environment(
    vm: &VirtualMachine<'_>,
    reflected: &ReflectedType,
) -> Vec<(TypeParameterKey, ReflectedType)> {
    let (name, arguments) = match &reflected.descriptor {
        TypeDescriptor::Named {
            name, arguments, ..
        } => (name, arguments.as_deref().unwrap_or_default()),
        TypeDescriptor::Member {
            class,
            class_arguments,
            member,
            member_arguments,
        } => {
            let class_owner = GenericOwner::Symbol(class.clone());
            let mut bindings = declaration_bindings(
                vm,
                &class_owner,
                class_arguments.as_deref().unwrap_or_default(),
                reflected,
            );
            if let Some(class_id) = vm.resolve_class_symbol(class)
                && let Some(entry) = vm.engine.tables.classes[class_id.0 as usize].method(member)
            {
                let member_owner = GenericOwner::Callable(CallableKey::Method {
                    class: entry.declaring_class,
                    name: member.clone(),
                });
                bindings.extend(declaration_bindings(
                    vm,
                    &member_owner,
                    member_arguments.as_deref().unwrap_or_default(),
                    reflected,
                ));
            }
            return bindings;
        }
        _ => return Vec::new(),
    };
    let owner = GenericOwner::Symbol(name.clone());
    declaration_bindings(vm, &owner, arguments, reflected)
}

pub(crate) fn complete_class_environment(
    vm: &VirtualMachine<'_>,
    reflected: &ReflectedType,
) -> Vec<(TypeParameterKey, ReflectedType)> {
    let Some((class, _)) = named_class(vm, reflected) else {
        return Vec::new();
    };
    let mut classes = vec![class];
    collect_base_ids(vm, class, &mut classes);
    let mut bindings = Vec::new();
    for class in classes {
        let Some(specialization) = class_specialization(vm, reflected, class) else {
            continue;
        };
        bindings.extend(named_environment(vm, &specialization));
    }
    bindings
}

fn declaration_bindings(
    vm: &VirtualMachine<'_>,
    owner: &GenericOwner,
    arguments: &[TypeDescriptor],
    reflected: &ReflectedType,
) -> Vec<(TypeParameterKey, ReflectedType)> {
    let Some(parameters) = support::type_parameters(vm, owner) else {
        return Vec::new();
    };
    parameters
        .into_iter()
        .enumerate()
        .filter_map(|(position, parameter)| {
            let descriptor = arguments.get(position).cloned().or(parameter.default)?;
            Some((
                TypeParameterKey {
                    owner: owner.clone(),
                    position,
                },
                child_type(&descriptor, reflected),
            ))
        })
        .collect()
}

pub(crate) fn class_specialization(
    vm: &VirtualMachine<'_>,
    reflected: &ReflectedType,
    target: ClassId,
) -> Option<ReflectedType> {
    let (source, arguments) = named_class(vm, reflected)?;
    if source == target {
        return Some(reflected.clone());
    }
    let source_class = &vm.engine.tables.classes[source.0 as usize];
    let specialization = source_class.base_specializations.get(&target)?;
    let parameters = &source_class.type_parameters;
    let arguments = complete_arguments(parameters, arguments);
    let bindings = parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, argument)| (parameter.name.clone(), argument))
        .collect::<Vec<_>>();
    let specialized: Vec<_> = specialization
        .iter()
        .map(|descriptor| substitute(descriptor, &bindings, 0))
        .collect();
    let target_name = vm.engine.tables.classes[target.0 as usize].name.clone();
    Some(ReflectedType {
        descriptor: TypeDescriptor::Named {
            name: target_name,
            arguments: (!specialized.is_empty()).then_some(specialized),
            recursive: false,
        },
        owner: reflected.owner.clone(),
        declaring_class: Some(target),
    })
}

fn complete_arguments(
    parameters: &[CompiledTypeParameter],
    written: &[TypeDescriptor],
) -> Vec<TypeDescriptor> {
    let mut bindings = Vec::new();
    let mut arguments = Vec::with_capacity(parameters.len());
    for (position, parameter) in parameters.iter().enumerate() {
        let argument = written.get(position).cloned().or_else(|| {
            parameter
                .default
                .as_ref()
                .map(|default| substitute(default, &bindings, 0))
        });
        let Some(argument) = argument else {
            break;
        };
        bindings.push((parameter.name.clone(), argument.clone()));
        arguments.push(argument);
    }
    arguments
}

fn named_class<'a>(
    vm: &'a VirtualMachine<'_>,
    reflected: &'a ReflectedType,
) -> Option<(ClassId, &'a [TypeDescriptor])> {
    let TypeDescriptor::Named {
        name, arguments, ..
    } = &reflected.descriptor
    else {
        return None;
    };
    let entry = vm.engine.tables.symbols.get(name)?;
    if !matches!(
        entry.kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
    ) {
        return None;
    }
    Some((
        ClassId(entry.index),
        arguments.as_deref().unwrap_or_default(),
    ))
}

fn collect_base_ids(vm: &VirtualMachine<'_>, class: ClassId, output: &mut Vec<ClassId>) {
    for base in &vm.engine.tables.classes[class.0 as usize].direct_bases {
        if !output.contains(&base.class) {
            output.push(base.class);
            collect_base_ids(vm, base.class, output);
        }
    }
}

pub(crate) fn resolve_type(
    vm: &VirtualMachine<'_>,
    reflected: &ReflectedType,
    bindings: &[(TypeParameterKey, ReflectedType)],
    called: Option<&ReflectedType>,
) -> ReflectedType {
    fn resolve_descriptor(
        vm: &VirtualMachine<'_>,
        descriptor: &TypeDescriptor,
        owner: Option<&GenericOwner>,
        bindings: &[(TypeParameterKey, ReflectedType)],
        called: Option<&ReflectedType>,
    ) -> TypeDescriptor {
        match descriptor {
            TypeDescriptor::Parameter(name) => owner
                .and_then(|owner| {
                    bindings.iter().find_map(|(parameter, argument)| {
                        if &parameter.owner != owner {
                            return None;
                        }
                        let parameters = support::type_parameters(vm, &parameter.owner)?;
                        (parameters.get(parameter.position)?.name == *name)
                            .then(|| argument.descriptor.clone())
                    })
                })
                .unwrap_or_else(|| descriptor.clone()),
            TypeDescriptor::StaticClass => called.map_or(TypeDescriptor::StaticClass, |called| {
                called.descriptor.clone()
            }),
            TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => {
                let class_owner = GenericOwner::Symbol(class.clone());
                let member_owner = vm.resolve_class_symbol(class).and_then(|class| {
                    vm.engine.tables.classes[class.0 as usize]
                        .method(member)
                        .map(|entry| {
                            GenericOwner::Callable(CallableKey::Method {
                                class: entry.declaring_class,
                                name: member.clone(),
                            })
                        })
                });
                TypeDescriptor::Member {
                    class: class.clone(),
                    class_arguments: class_arguments.as_ref().map(|arguments| {
                        arguments
                            .iter()
                            .map(|argument| {
                                resolve_descriptor(
                                    vm,
                                    argument,
                                    Some(&class_owner),
                                    bindings,
                                    called,
                                )
                            })
                            .collect()
                    }),
                    member: member.clone(),
                    member_arguments: member_arguments.as_ref().map(|arguments| {
                        arguments
                            .iter()
                            .map(|argument| {
                                resolve_descriptor(
                                    vm,
                                    argument,
                                    member_owner.as_ref().or(owner),
                                    bindings,
                                    called,
                                )
                            })
                            .collect()
                    }),
                }
            }
            _ => descriptor
                .map_children(|child| resolve_descriptor(vm, child, owner, bindings, called)),
        }
    }

    ReflectedType {
        descriptor: resolve_descriptor(
            vm,
            &reflected.descriptor,
            reflected.owner.as_ref(),
            bindings,
            called,
        ),
        owner: reflected.owner.clone(),
        declaring_class: reflected.declaring_class,
    }
}

pub(crate) fn is_resolved(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Parameter(_) | TypeDescriptor::StaticClass => false,
        TypeDescriptor::Named { arguments, .. } => arguments
            .as_ref()
            .is_none_or(|arguments| arguments.iter().all(is_resolved)),
        TypeDescriptor::Member {
            class_arguments,
            member_arguments,
            ..
        } => {
            class_arguments
                .as_ref()
                .is_none_or(|arguments| arguments.iter().all(is_resolved))
                && member_arguments
                    .as_ref()
                    .is_none_or(|arguments| arguments.iter().all(is_resolved))
        }
        TypeDescriptor::Array(arguments) | TypeDescriptor::Dictionary(arguments) => arguments
            .as_ref()
            .is_none_or(|(key, value)| is_resolved(key) && is_resolved(value)),
        TypeDescriptor::Vector(value) => value.as_deref().is_none_or(is_resolved),
        TypeDescriptor::VectorShape { elements, rest } => {
            elements.iter().all(is_resolved) && rest.as_deref().is_none_or(is_resolved)
        }
        TypeDescriptor::DictionaryShape { entries, rest } => {
            entries.iter().all(|(_, value)| is_resolved(value))
                && rest
                    .as_ref()
                    .is_none_or(|(key, value)| is_resolved(key) && is_resolved(value))
        }
        TypeDescriptor::Callable(signature) => signature.as_ref().is_none_or(|signature| {
            signature
                .parameters
                .iter()
                .all(|parameter| is_resolved(&parameter.r#type))
                && is_resolved(&signature.return_type)
        }),
        TypeDescriptor::Classname(inner) | TypeDescriptor::Negated(inner) => is_resolved(inner),
        TypeDescriptor::Tuple(types)
        | TypeDescriptor::Union(types)
        | TypeDescriptor::Intersection(types) => types.iter().all(is_resolved),
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.iter().all(is_resolved) && is_resolved(rest)
        }
        _ => true,
    }
}

fn require_resolved(
    context: &mut Context<'_, '_, '_>,
    reflected: &ReflectedType,
) -> Result<(), Throw> {
    if is_resolved(&reflected.descriptor) {
        Ok(())
    } else {
        Err(context.type_error("the reflected type is not resolved"))
    }
}

fn optional_environment_argument(
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

fn optional_type_argument(
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

fn reflected_type_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<ReflectedType, Throw> {
    optional_type_argument(context, arguments, position)?
        .ok_or_else(|| context.type_error("the reflected type argument is missing"))
}

fn reflected_parameter_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<TypeParameterKey, Throw> {
    let Some(value) = arguments.get(position) else {
        return Err(context.type_error("the reflected type parameter is missing"));
    };
    match objects::data(value) {
        Some(ReflectionData::TypeParameter(parameter)) => Ok(parameter),
        _ => Err(context.type_error("the reflection argument is not a reflected type parameter")),
    }
}

fn reflected_symbol_argument(
    context: &mut Context<'_, '_, '_>,
    arguments: &Arguments<'_>,
    position: usize,
) -> Result<Atom, Throw> {
    let Some(value) = arguments.get(position) else {
        return Err(context.type_error("the reflected symbol argument is missing"));
    };
    match objects::data(value) {
        Some(ReflectionData::Symbol(name)) => Ok(name),
        _ => Err(context.type_error("the reflection argument is not a reflected symbol")),
    }
}

fn reflect_descriptors(
    context: &mut Context<'_, '_, '_>,
    descriptors: &[TypeDescriptor],
    parent: &ReflectedType,
) -> Result<Value, Throw> {
    let mut values = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        values.push(objects::r#type(context, child_type(descriptor, parent))?);
    }
    Ok(context.vec(values))
}

fn reflect_child(
    context: &mut Context<'_, '_, '_>,
    descriptor: &TypeDescriptor,
    parent: &ReflectedType,
) -> Result<Value, Throw> {
    objects::r#type(context, child_type(descriptor, parent))
}

pub(crate) fn child_type(descriptor: &TypeDescriptor, parent: &ReflectedType) -> ReflectedType {
    ReflectedType {
        descriptor: descriptor.clone(),
        owner: parent.owner.clone(),
        declaring_class: parent.declaring_class,
    }
}

fn class_id(vm: &VirtualMachine<'_>, name: &Atom) -> Option<ClassId> {
    vm.resolve_class_symbol(name)
}

fn index_value(position: usize) -> Value {
    Value::int(i64::try_from(position).expect("reflection positions fit in i64"))
}
