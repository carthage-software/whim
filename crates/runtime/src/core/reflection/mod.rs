//! Native, read-only reflection over the engine's loaded declarations and
//! values.

mod attributes;
mod classes;
mod declarations;
mod enums;
pub(crate) mod functions;
mod interfaces;
mod metadata;
mod model;
mod objects;
mod state;
mod support;
mod types;
mod values;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::value::Value;

pub(crate) use classes::*;
pub(crate) use enums::*;
pub(crate) use interfaces::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Accepts,
    AliasedType,
    Argument,
    Arguments,
    AttributeDefinition,
    Attributes,
    AttributesByName,
    BackingType,
    BackingValue,
    BaseTypes,
    Binding,
    Bindings,
    BoundArguments,
    BoundObject,
    Bounds,
    CallableKind,
    CallableType,
    CalledType,
    Capture,
    Captures,
    Case,
    Cases,
    Class,
    ClassLike,
    ClassType,
    Constant,
    Constants,
    Constructor,
    Declaration,
    DeclaredConstants,
    DeclaredMethods,
    DeclaredProperties,
    DeclaredType,
    DeclaringCallable,
    DeclaringDeclaration,
    DeclaringType,
    Default,
    DefaultValue,
    Destructor,
    DirectBaseTypes,
    DirectInterfaceTypes,
    DirectParentTypes,
    Documentation,
    EndColumn,
    EndLine,
    EndOffset,
    Entries,
    Enum,
    EnumCase,
    Equals,
    File,
    HasDefaultValue,
    InnerType,
    InterfaceTypes,
    IsAbstract,
    IsCloneable,
    IsConstructor,
    IsDestructor,
    IsFinal,
    IsInitialized,
    IsInstanceOf,
    IsInstantiable,
    IsOptional,
    IsPromoted,
    IsReadonly,
    IsReceiver,
    IsRecursiveReference,
    IsRepeatable,
    IsResolved,
    IsSensitive,
    IsShort,
    IsStatic,
    IsStaticInitialized,
    IsSubtypeOf,
    Key,
    KeyType,
    Location,
    LowerBound,
    MaximumLength,
    Method,
    Methods,
    MinimumLength,
    Name,
    NamedArguments,
    NamespaceName,
    NewInstance,
    ObjectType,
    Origin,
    Parameter,
    Parameters,
    ParentType,
    ParentTypes,
    PermittedSubtypeNames,
    Position,
    PromotedProperty,
    Properties,
    Property,
    PropertyValue,
    PropertyValues,
    Prototypes,
    RequiredParameterCount,
    Resolve,
    RestKeyType,
    RestType,
    RestValueType,
    ReturnType,
    ScopeClass,
    ShortName,
    Specialization,
    StartColumn,
    StartLine,
    StartOffset,
    StaticValue,
    SymbolKind,
    Target,
    Targets,
    ToString,
    Type,
    TypeArgument,
    TypeArguments,
    TypeEnvironment,
    TypeId,
    TypeKind,
    TypeParameter,
    TypeParameters,
    Types,
    UpperBound,
    Value,
    ValueType,
    Variance,
    Visibility,
}

pub(crate) fn dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
) -> Result<Value, Throw> {
    state::dispatch(context, arguments, operation)
}

#[cfg(test)]
mod tests {
    use crate::engine::Engine;
    use crate::engine::EngineConfiguration;

    #[test]
    fn sealed_reflection_interfaces_permit_their_public_leaves() {
        let engine = Engine::new(EngineConfiguration::default());
        let callable = engine
            .heap
            .intern(b"Whim\\Reflection\\Callable\\CallableReflection");
        let method = engine
            .heap
            .intern(b"Whim\\Reflection\\Member\\MethodReflection");
        let entry = engine.tables.symbols[&callable];
        let permitted = engine.tables.classes[entry.index as usize]
            .sealed_to
            .as_ref()
            .expect("the callable reflection interface is sealed");

        assert!(
            permitted.contains(&method),
            "CallableReflection does not permit MethodReflection: {permitted:?}",
        );

        let named = engine
            .heap
            .intern(b"Whim\\Reflection\\Type\\NamedTypeReflection");
        let class_type = engine
            .heap
            .intern(b"Whim\\Reflection\\Type\\ClassTypeReflection");
        let entry = engine.tables.symbols[&named];
        let permitted = engine.tables.classes[entry.index as usize]
            .sealed_to
            .as_ref()
            .expect("the named type reflection interface is sealed");
        assert!(
            permitted.contains(&class_type),
            "NamedTypeReflection does not permit ClassTypeReflection: {permitted:?}",
        );
    }
}
