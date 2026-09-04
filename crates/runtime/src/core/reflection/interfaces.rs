//! Shared public reflection contracts.

use whim_macros::whim_interface;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::reflection::Operation;
use crate::core::reflection::dispatch;
use crate::value::Value;

macro_rules! reflection_interface {
    (
        $(#[$attribute:meta])*
        $rust_name:ident = $whim_name:literal {
            $($method:ident : $signature:literal => $operation:ident;)*
        }
    ) => {
        #[whim_interface($whim_name)]
        $(#[$attribute])*
        trait $rust_name {
            $(
                #[whim_method($signature, must_use)]
                fn $method(
                    context: &mut Context<'_, '_, '_>,
                    arguments: Arguments<'_>,
                ) -> Result<Value, Throw> {
                    dispatch(context, arguments, Operation::$operation)
                }
            )*
        }
    };
}

reflection_interface! {
    #[whim_permits(
        "Whim\\Reflection\\Symbol\\SymbolReflection",
        "Whim\\Reflection\\Symbol\\ClassLikeReflection",
        "Whim\\Reflection\\Generic\\GenericDeclarationReflection",
        "Whim\\Reflection\\Member\\MemberReflection",
        "Whim\\Reflection\\Callable\\CallableReflection",
        "Whim\\Reflection\\Callable\\ParameterReflection",
        "Whim\\Reflection\\Symbol\\ClassReflection",
        "Whim\\Reflection\\Symbol\\InterfaceReflection",
        "Whim\\Reflection\\Symbol\\EnumReflection",
        "Whim\\Reflection\\Symbol\\TypeAliasReflection",
        "Whim\\Reflection\\Symbol\\NewtypeReflection",
        "Whim\\Reflection\\Symbol\\FunctionReflection",
        "Whim\\Reflection\\Symbol\\ConstantReflection",
        "Whim\\Reflection\\Member\\MethodReflection",
        "Whim\\Reflection\\Member\\PropertyReflection",
        "Whim\\Reflection\\Member\\ClassConstantReflection",
        "Whim\\Reflection\\Member\\EnumCaseReflection",
        "Whim\\Reflection\\Callable\\ClosureReflection"
    )]
    DeclarationReflection = "Whim\\Reflection\\DeclarationReflection" {
        get_origin: "getOrigin(): Whim\\Reflection\\DeclarationOrigin" => Origin;
        get_location: "getLocation(): null|Whim\\Reflection\\SourceLocation" => Location;
        get_documentation: "getDocumentation(): null|string" => Documentation;
        get_attributes: "getAttributes<T: object = object>(): vec<Whim\\Reflection\\AttributeReflection<T>>" => Attributes;
        get_attributes_by_name: "getAttributesByName(string $class): vec<Whim\\Reflection\\AttributeReflection<object>>" => AttributesByName;
    }
}

reflection_interface! {
    #[whim_extends("Whim\\Reflection\\DeclarationReflection")]
    #[whim_permits(
        "Whim\\Reflection\\Symbol\\ClassLikeReflection",
        "Whim\\Reflection\\Callable\\CallableReflection",
        "Whim\\Reflection\\Symbol\\ClassReflection",
        "Whim\\Reflection\\Symbol\\InterfaceReflection",
        "Whim\\Reflection\\Symbol\\EnumReflection",
        "Whim\\Reflection\\Symbol\\TypeAliasReflection",
        "Whim\\Reflection\\Symbol\\NewtypeReflection",
        "Whim\\Reflection\\Symbol\\FunctionReflection",
        "Whim\\Reflection\\Member\\MethodReflection",
        "Whim\\Reflection\\Callable\\ClosureReflection"
    )]
    GenericDeclarationReflection = "Whim\\Reflection\\Generic\\GenericDeclarationReflection" {
        get_type_parameters: "getTypeParameters(): vec<Whim\\Reflection\\Generic\\TypeParameterReflection>" => TypeParameters;
        get_type_parameter: "getTypeParameter(int|string $parameter): null|Whim\\Reflection\\Generic\\TypeParameterReflection" => TypeParameter;
    }
}

reflection_interface! {
    #[whim_extends("Whim\\Reflection\\DeclarationReflection")]
    #[whim_permits(
        "Whim\\Reflection\\Symbol\\ClassLikeReflection",
        "Whim\\Reflection\\Symbol\\ClassReflection",
        "Whim\\Reflection\\Symbol\\InterfaceReflection",
        "Whim\\Reflection\\Symbol\\EnumReflection",
        "Whim\\Reflection\\Symbol\\TypeAliasReflection",
        "Whim\\Reflection\\Symbol\\NewtypeReflection",
        "Whim\\Reflection\\Symbol\\FunctionReflection",
        "Whim\\Reflection\\Symbol\\ConstantReflection"
    )]
    SymbolReflection = "Whim\\Reflection\\Symbol\\SymbolReflection" {
        get_name: "getName(): string" => Name;
        get_short_name: "getShortName(): string" => ShortName;
        get_namespace_name: "getNamespaceName(): string" => NamespaceName;
        get_kind: "getKind(): Whim\\Symbol\\SymbolKind" => SymbolKind;
    }
}

reflection_interface! {
    #[whim_extends("Whim\\Reflection\\Symbol\\SymbolReflection")]
    #[whim_extends("Whim\\Reflection\\Generic\\GenericDeclarationReflection")]
    #[whim_permits(
        "Whim\\Reflection\\Symbol\\ClassReflection",
        "Whim\\Reflection\\Symbol\\InterfaceReflection",
        "Whim\\Reflection\\Symbol\\EnumReflection"
    )]
    ClassLikeReflection = "Whim\\Reflection\\Symbol\\ClassLikeReflection" {
        get_type: "getType(): Whim\\Reflection\\Type\\ClassTypeReflection" => Type;
        get_direct_base_types: "getDirectBaseTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => DirectBaseTypes;
        get_base_types: "getBaseTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => BaseTypes;
        get_permitted_subtype_names: "getPermittedSubtypeNames(): null|vec<string>" => PermittedSubtypeNames;
        get_declared_methods: "getDeclaredMethods(): vec<Whim\\Reflection\\Member\\MethodReflection>" => DeclaredMethods;
        get_methods: "getMethods(): vec<Whim\\Reflection\\Member\\MethodReflection>" => Methods;
        get_method: "getMethod(string $name): null|Whim\\Reflection\\Member\\MethodReflection" => Method;
        get_declared_properties: "getDeclaredProperties(): vec<Whim\\Reflection\\Member\\PropertyReflection>" => DeclaredProperties;
        get_properties: "getProperties(): vec<Whim\\Reflection\\Member\\PropertyReflection>" => Properties;
        get_property: "getProperty(string $name): null|Whim\\Reflection\\Member\\PropertyReflection" => Property;
        get_declared_constants: "getDeclaredConstants(): vec<Whim\\Reflection\\Member\\ClassConstantReflection>" => DeclaredConstants;
        get_constants: "getConstants(): vec<Whim\\Reflection\\Member\\ClassConstantReflection>" => Constants;
        get_constant: "getConstant(string $name): null|Whim\\Reflection\\Member\\ClassConstantReflection" => Constant;
    }
}

reflection_interface! {
    #[whim_extends("Whim\\Reflection\\Generic\\GenericDeclarationReflection")]
    #[whim_permits(
        "Whim\\Reflection\\Symbol\\FunctionReflection",
        "Whim\\Reflection\\Member\\MethodReflection",
        "Whim\\Reflection\\Callable\\ClosureReflection"
    )]
    CallableReflection = "Whim\\Reflection\\Callable\\CallableReflection" {
        get_name: "getName(): string" => Name;
        get_parameters: "getParameters(): vec<Whim\\Reflection\\Callable\\ParameterReflection>" => Parameters;
        get_parameter: "getParameter(int|string $parameter): null|Whim\\Reflection\\Callable\\ParameterReflection" => Parameter;
        get_required_parameter_count: "getRequiredParameterCount(): (0..)" => RequiredParameterCount;
        get_return_type: "getReturnType(): null|Whim\\Reflection\\Type\\TypeReflection" => ReturnType;
        get_callable_type: "getCallableType(null|Whim\\Reflection\\Generic\\TypeEnvironmentReflection $environment = null, null|Whim\\Reflection\\Type\\ClassTypeReflection $calledType = null): Whim\\Reflection\\Type\\FunctionTypeReflection" => CallableType;
    }
}

reflection_interface! {
    #[whim_extends("Whim\\Reflection\\DeclarationReflection")]
    #[whim_permits(
        "Whim\\Reflection\\Member\\MethodReflection",
        "Whim\\Reflection\\Member\\PropertyReflection",
        "Whim\\Reflection\\Member\\ClassConstantReflection",
        "Whim\\Reflection\\Member\\EnumCaseReflection"
    )]
    MemberReflection = "Whim\\Reflection\\Member\\MemberReflection" {
        get_name: "getName(): string" => Name;
        get_declaring_type: "getDeclaringType(): Whim\\Reflection\\Symbol\\ClassLikeReflection" => DeclaringType;
        get_visibility: "getVisibility(): Whim\\Reflection\\Member\\Visibility" => Visibility;
    }
}

reflection_interface! {
    #[whim_extends("Whim\\Convert\\ToString")]
    #[whim_permits(
        "Whim\\Reflection\\Type\\PrimitiveTypeReflection",
        "Whim\\Reflection\\Type\\LiteralTypeReflection",
        "Whim\\Reflection\\Type\\IntegerRangeTypeReflection",
        "Whim\\Reflection\\Type\\StringLengthTypeReflection",
        "Whim\\Reflection\\Type\\NamedTypeReflection",
        "Whim\\Reflection\\Type\\ClassTypeReflection",
        "Whim\\Reflection\\Type\\SymbolTypeReflection",
        "Whim\\Reflection\\Type\\MemberTypeReflection",
        "Whim\\Reflection\\Type\\TypeParameterTypeReflection",
        "Whim\\Reflection\\Type\\StaticTypeReflection",
        "Whim\\Reflection\\Type\\UnionTypeReflection",
        "Whim\\Reflection\\Type\\IntersectionTypeReflection",
        "Whim\\Reflection\\Type\\NegatedTypeReflection",
        "Whim\\Reflection\\Type\\FunctionTypeReflection",
        "Whim\\Reflection\\Type\\ArrayTypeReflection",
        "Whim\\Reflection\\Type\\VecTypeReflection",
        "Whim\\Reflection\\Type\\VecShapeTypeReflection",
        "Whim\\Reflection\\Type\\DictTypeReflection",
        "Whim\\Reflection\\Type\\DictShapeTypeReflection",
        "Whim\\Reflection\\Type\\ClassnameTypeReflection",
        "Whim\\Reflection\\Type\\TupleTypeReflection",
        "Whim\\Reflection\\Type\\WildcardTypeReflection"
    )]
    TypeReflection = "Whim\\Reflection\\Type\\TypeReflection" {
        get_kind: "getKind(): Whim\\Reflection\\Type\\TypeKind" => TypeKind;
        is_resolved: "isResolved(): bool" => IsResolved;
        get_id: "getId(): Whim\\Type\\TypeId" => TypeId;
        to_string: "toString(): string" => ToString;
        resolve: "resolve(null|Whim\\Reflection\\Generic\\TypeEnvironmentReflection $environment = null, null|Whim\\Reflection\\Type\\ClassTypeReflection $calledType = null): Whim\\Reflection\\Type\\TypeReflection" => Resolve;
        accepts: "accepts(mixed $value): bool" => Accepts;
        equals: "equals(Whim\\Reflection\\Type\\TypeReflection $other): bool" => Equals;
        is_subtype_of: "isSubtypeOf(Whim\\Reflection\\Type\\TypeReflection $other): bool" => IsSubtypeOf;
    }
}

reflection_interface! {
    #[whim_extends("Whim\\Reflection\\Type\\TypeReflection")]
    #[whim_permits(
        "Whim\\Reflection\\Type\\ClassTypeReflection",
        "Whim\\Reflection\\Type\\SymbolTypeReflection",
        "Whim\\Reflection\\Type\\MemberTypeReflection"
    )]
    NamedTypeReflection = "Whim\\Reflection\\Type\\NamedTypeReflection" {
        get_declaration: "getDeclaration(): Whim\\Reflection\\Symbol\\SymbolReflection|Whim\\Reflection\\Member\\MethodReflection|Whim\\Reflection\\Member\\ClassConstantReflection|Whim\\Reflection\\Member\\EnumCaseReflection" => Declaration;
        get_type_arguments: "getTypeArguments(): vec<Whim\\Reflection\\Type\\TypeReflection>" => TypeArguments;
        get_type_environment: "getTypeEnvironment(): Whim\\Reflection\\Generic\\TypeEnvironmentReflection" => TypeEnvironment;
        is_recursive_reference: "isRecursiveReference(): bool" => IsRecursiveReference;
    }
}
