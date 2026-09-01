//! Construction and checked projection of reflection objects.

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::core::reflection::classes;
use crate::core::reflection::model::DeclarationKey;
use crate::core::reflection::model::MemberKind;
use crate::core::reflection::model::ReflectedType;
use crate::core::reflection::model::ReflectionData;
use crate::core::type_;
use crate::symbols::SymbolKind;
use crate::unreachable_invariant;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::object::TypeEnvironmentId;

pub(crate) fn build(
    context: &mut Context<'_, '_, '_>,
    data: ReflectionData,
    values: Vec<Value>,
) -> Result<Value, Throw> {
    let class_name = class_name(context, &data)?;
    let name = context.vm.intern(class_name.as_bytes());
    let Some(class) = context.vm.resolve_class_symbol(&name) else {
        return Err(context.type_error("the reflection implementation class is not registered"));
    };
    let value = if let ReflectionData::Attribute { declaration, .. } = &data {
        let argument = TypeDescriptor::Named {
            name: declaration.class.clone(),
            arguments: None,
            recursive: false,
        };
        context.vm.build_built_in_instance_typed(
            class,
            &[argument],
            TypeEnvironmentId::default(),
        )?
    } else {
        context.vm.build_built_in_instance(class)?
    };
    let Some(state) = classes::state(&value) else {
        // SAFETY: every reflection implementation carries reflection state.
        unsafe { unreachable_invariant("a reflection object carries reflection state") }
    };
    state.initialize(data, values);
    Ok(value)
}

pub(crate) fn data(value: &Value) -> Option<ReflectionData> {
    classes::state(value)?.data.borrow().clone()
}

pub(crate) fn symbol(context: &mut Context<'_, '_, '_>, name: Atom) -> Result<Value, Throw> {
    build(context, ReflectionData::Symbol(name), Vec::new())
}

pub(crate) fn declaration(
    context: &mut Context<'_, '_, '_>,
    declaration: DeclarationKey,
) -> Result<Value, Throw> {
    match declaration {
        DeclarationKey::Symbol(name) => symbol(context, name),
        DeclarationKey::Member(member) => {
            build(context, ReflectionData::Member(member), Vec::new())
        }
        DeclarationKey::Parameter { callable, position } => build(
            context,
            ReflectionData::Parameter { callable, position },
            Vec::new(),
        ),
        DeclarationKey::Closure(function) => {
            build(context, ReflectionData::Closure(function), Vec::new())
        }
    }
}

pub(crate) fn r#type(
    context: &mut Context<'_, '_, '_>,
    r#type: ReflectedType,
) -> Result<Value, Throw> {
    build(context, ReflectionData::Type(r#type), Vec::new())
}

pub(crate) fn enum_case(
    context: &mut Context<'_, '_, '_>,
    enum_name: &[u8],
    case_name: &[u8],
) -> Result<Value, Throw> {
    let enum_name = context.vm.intern(enum_name);
    let Some(class) = context.vm.resolve_class_symbol(&enum_name) else {
        return Err(context.type_error("the reflection enum is not registered"));
    };
    let case_name = context.vm.intern(case_name);
    let Some(value) = context.vm.enum_case_value(class, case_name) else {
        return Err(context.type_error("the reflection enum case is not registered"));
    };
    Ok(value)
}

pub(crate) fn type_id(context: &mut Context<'_, '_, '_>, descriptor: &TypeDescriptor) -> Value {
    let identifier = context.vm.intern_type_descriptor_ref(descriptor);
    type_::type_id_value(context, identifier)
}

fn class_name(
    context: &mut Context<'_, '_, '_>,
    data: &ReflectionData,
) -> Result<&'static str, Throw> {
    Ok(match data {
        ReflectionData::SourceLocation(_) => "Whim\\Reflection\\SourceLocation",
        ReflectionData::Symbol(name) => {
            let Some(entry) = context.vm.engine.tables.symbols.get(name) else {
                return Err(context.type_error("the reflected symbol is no longer loaded"));
            };
            match entry.kind {
                SymbolKind::Class => "Whim\\Reflection\\Symbol\\ClassReflection",
                SymbolKind::Interface => "Whim\\Reflection\\Symbol\\InterfaceReflection",
                SymbolKind::Enum => "Whim\\Reflection\\Symbol\\EnumReflection",
                SymbolKind::TypeAlias => "Whim\\Reflection\\Symbol\\TypeAliasReflection",
                SymbolKind::Newtype => "Whim\\Reflection\\Symbol\\NewtypeReflection",
                SymbolKind::Function => "Whim\\Reflection\\Symbol\\FunctionReflection",
                SymbolKind::Constant => "Whim\\Reflection\\Symbol\\ConstantReflection",
            }
        }
        ReflectionData::Member(member) => match member.kind {
            MemberKind::Method => "Whim\\Reflection\\Member\\MethodReflection",
            MemberKind::Property => "Whim\\Reflection\\Member\\PropertyReflection",
            MemberKind::ClassConstant => "Whim\\Reflection\\Member\\ClassConstantReflection",
            MemberKind::EnumCase => "Whim\\Reflection\\Member\\EnumCaseReflection",
        },
        ReflectionData::Parameter { .. } => "Whim\\Reflection\\Callable\\ParameterReflection",
        ReflectionData::Closure(_) => "Whim\\Reflection\\Callable\\ClosureReflection",
        ReflectionData::Capture { .. } => "Whim\\Reflection\\Callable\\CaptureReflection",
        ReflectionData::CallableValue => "Whim\\Reflection\\Callable\\CallableValueReflection",
        ReflectionData::CaptureValue { .. } => "Whim\\Reflection\\Callable\\CaptureValueReflection",
        ReflectionData::BoundArgument { .. } => {
            "Whim\\Reflection\\Callable\\BoundArgumentReflection"
        }
        ReflectionData::TypeParameter(_) => "Whim\\Reflection\\Generic\\TypeParameterReflection",
        ReflectionData::TypeBinding { .. } => "Whim\\Reflection\\Generic\\TypeBindingReflection",
        ReflectionData::TypeEnvironment(_) => {
            "Whim\\Reflection\\Generic\\TypeEnvironmentReflection"
        }
        ReflectionData::Attribute { .. } => "Whim\\Reflection\\AttributeReflection",
        ReflectionData::AttributeDefinition(_) => {
            "Whim\\Reflection\\Attribute\\DefinitionReflection"
        }
        ReflectionData::Object => "Whim\\Reflection\\ObjectReflection",
        ReflectionData::PropertyValue { .. } => "Whim\\Reflection\\PropertyValueReflection",
        ReflectionData::NewtypeValue(_) => "Whim\\Reflection\\NewtypeValueReflection",
        ReflectionData::Type(r#type) => type_class_name(context, &r#type.descriptor),
        ReflectionData::FunctionTypeParameter { .. } => {
            "Whim\\Reflection\\Type\\FunctionTypeParameterReflection"
        }
        ReflectionData::DictShapeEntry { .. } => "Whim\\Reflection\\Type\\DictShapeEntryReflection",
    })
}

fn type_class_name(context: &Context<'_, '_, '_>, descriptor: &TypeDescriptor) -> &'static str {
    match descriptor {
        TypeDescriptor::Wildcard => "Whim\\Reflection\\Type\\WildcardTypeReflection",
        TypeDescriptor::Mixed
        | TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::Object => "Whim\\Reflection\\Type\\PrimitiveTypeReflection",
        TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_) => "Whim\\Reflection\\Type\\LiteralTypeReflection",
        TypeDescriptor::IntRange { .. } => "Whim\\Reflection\\Type\\IntegerRangeTypeReflection",
        TypeDescriptor::Named { name, .. } => context.vm.engine.tables.symbols.get(name).map_or(
            "Whim\\Reflection\\Type\\SymbolTypeReflection",
            |entry| {
                if matches!(
                    entry.kind,
                    SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
                ) {
                    "Whim\\Reflection\\Type\\ClassTypeReflection"
                } else {
                    "Whim\\Reflection\\Type\\SymbolTypeReflection"
                }
            },
        ),
        TypeDescriptor::Member { .. } => "Whim\\Reflection\\Type\\MemberTypeReflection",
        TypeDescriptor::Parameter(_) => "Whim\\Reflection\\Type\\TypeParameterTypeReflection",
        TypeDescriptor::StaticClass => "Whim\\Reflection\\Type\\StaticTypeReflection",
        TypeDescriptor::Array(_) => "Whim\\Reflection\\Type\\ArrayTypeReflection",
        TypeDescriptor::Vector(_) => "Whim\\Reflection\\Type\\VecTypeReflection",
        TypeDescriptor::VectorShape { .. } => "Whim\\Reflection\\Type\\VecShapeTypeReflection",
        TypeDescriptor::Dictionary(_) => "Whim\\Reflection\\Type\\DictTypeReflection",
        TypeDescriptor::DictionaryShape { .. } => "Whim\\Reflection\\Type\\DictShapeTypeReflection",
        TypeDescriptor::Callable(_) => "Whim\\Reflection\\Type\\FunctionTypeReflection",
        TypeDescriptor::Classname(_) => "Whim\\Reflection\\Type\\ClassnameTypeReflection",
        TypeDescriptor::Tuple(_) | TypeDescriptor::TupleRest { .. } | TypeDescriptor::TupleAny => {
            "Whim\\Reflection\\Type\\TupleTypeReflection"
        }
        TypeDescriptor::Union(_) => "Whim\\Reflection\\Type\\UnionTypeReflection",
        TypeDescriptor::Intersection(_) => "Whim\\Reflection\\Type\\IntersectionTypeReflection",
        TypeDescriptor::Negated(_) => "Whim\\Reflection\\Type\\NegatedTypeReflection",
    }
}
