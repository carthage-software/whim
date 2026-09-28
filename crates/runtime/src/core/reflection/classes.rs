//! Native reflection object declarations.

use whim_macros::whim_class;
use whim_macros::whim_methods;
use whim_value::Value;
use whim_value::heap::metadata::TeardownMode;
use whim_value::heap::metadata::TraceVisitor;
use whim_value::heap::queue::DropQueue;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::BuiltInChildren;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::core::reflection::Operation;
use crate::core::reflection::dispatch;
use crate::core::reflection::state::ReflectionState;
use crate::vm::VirtualMachine;

macro_rules! reflection_class {
    (
        $rust_name:ident = $whim_name:literal
        $(implements [$($interface:literal),* $(,)?])?
        $(with [$($group:ident),* $(,)?])?
        {
            $($method:ident : $signature:literal => $operation:ident;)*
        }
        $(calls { $($call:ident : $call_signature:literal => $call_operation:ident;)* })?
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($($interface),*)?];
            methods = [$($method : $signature => $operation;)*];
            calls = [$($($call : $call_signature => $call_operation;)*)?];
            groups = [$($($group),*)?];
        }
    };
}

macro_rules! reflection_class_methods {
    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($call:ident : $call_signature:literal => $call_operation:ident;)*];
        groups = [];
    ) => {
        #[whim_class($whim_name, final, traced)]
        $(#[whim_implements($interface)])*
        pub(crate) struct $rust_name(pub(crate) ReflectionState);

        impl $rust_name {
            pub(crate) fn new(_vm: &mut VirtualMachine<'_>) -> Result<Self, Throw> {
                Ok(Self(ReflectionState::default()))
            }
        }

        // SAFETY: the wrapper delegates its complete child set to its sole state.
        unsafe impl BuiltInChildren for $rust_name {
            fn enqueue_built_in_children(
                &mut self,
                queue: &DropQueue,
                mode: TeardownMode,
            ) {
                self.0.enqueue_children(queue, mode);
            }

            fn visit_built_in_children(&self, visitor: &mut TraceVisitor<'_>) {
                self.0.visit_children(visitor);
            }
        }

        #[whim_methods]
        impl $rust_name {
            #[whim_method("__construct(): void", visibility = "private")]
            const fn construct() {}

            $(
                #[whim_method($signature, must_use)]
                fn $method(
                    context: &mut Context<'_, '_, '_>,
                    arguments: Arguments<'_>,
                ) -> Result<Value, Throw> {
                    dispatch(context, arguments, Operation::$operation)
                }
            )*

            $(
                #[whim_method($call_signature)]
                fn $call(
                    context: &mut Context<'_, '_, '_>,
                    arguments: Arguments<'_>,
                ) -> Result<Value, Throw> {
                    dispatch(context, arguments, Operation::$call_operation)
                }
            )*
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [declaration $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_origin: "getOrigin(): Whim\\Reflection\\DeclarationOrigin" => Origin;
                get_location: "getLocation(): null|Whim\\Reflection\\SourceLocation" => Location;
                get_documentation: "getDocumentation(): null|string" => Documentation;
                get_attributes: "getAttributes<T: object = object>(): vec<Whim\\Reflection\\AttributeReflection<T>>" => Attributes;
                get_attributes_by_name: "getAttributesByName(string $class): vec<Whim\\Reflection\\AttributeReflection<object>>" => AttributesByName;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [generic $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_type_parameters: "getTypeParameters(): vec<Whim\\Reflection\\Generic\\TypeParameterReflection>" => TypeParameters;
                get_type_parameter: "getTypeParameter(int|string $parameter): null|Whim\\Reflection\\Generic\\TypeParameterReflection" => TypeParameter;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [symbol $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_name: "getName(): string" => Name;
                get_short_name: "getShortName(): string" => ShortName;
                get_namespace_name: "getNamespaceName(): string" => NamespaceName;
                get_kind: "getKind(): Whim\\Symbol\\SymbolKind" => SymbolKind;
                get_file: "getFile(): null|Whim\\Reflection\\FileReflection" => File;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [class_like $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
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
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [callable_name $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_name: "getName(): string" => Name;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [callable $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_parameters: "getParameters(): vec<Whim\\Reflection\\ParameterReflection>" => Parameters;
                get_where_constraints: "getWhereConstraints(): vec<Whim\\Reflection\\Generic\\WhereConstraintReflection>" => WhereConstraints;
                get_parameter: "getParameter(int|string $parameter): null|Whim\\Reflection\\ParameterReflection" => Parameter;
                get_required_parameter_count: "getRequiredParameterCount(): uint" => RequiredParameterCount;
                get_return_type: "getReturnType(): null|Whim\\Reflection\\Type\\TypeReflection" => ReturnType;
                get_callable_type: "getCallableType(null|Whim\\Reflection\\Generic\\TypeEnvironmentReflection $environment = null, null|Whim\\Reflection\\Type\\ClassTypeReflection $calledType = null): Whim\\Reflection\\Type\\FunctionTypeReflection" => CallableType;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [member $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_name: "getName(): string" => Name;
                get_declaring_type: "getDeclaringType(): Whim\\Reflection\\ClassLikeReflection" => DeclaringType;
                get_visibility: "getVisibility(): Whim\\Reflection\\Member\\Visibility" => Visibility;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [type_reflection $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_kind: "getKind(): Whim\\Reflection\\Type\\TypeKind" => TypeKind;
                is_resolved: "isResolved(): bool" => IsResolved;
                get_id: "getId(): Whim\\Type\\TypeId" => TypeId;
                to_string: "toString(): string" => ToString;
                resolve: "resolve(null|Whim\\Reflection\\Generic\\TypeEnvironmentReflection $environment = null, null|Whim\\Reflection\\Type\\ClassTypeReflection $calledType = null): Whim\\Reflection\\Type\\TypeReflection" => Resolve;
                accepts: "accepts(mixed $value): bool" => Accepts;
                equals: "equals(Whim\\Reflection\\Type\\TypeReflection $other): bool" => Equals;
                is_subtype_of: "isSubtypeOf(Whim\\Reflection\\Type\\TypeReflection $other): bool" => IsSubtypeOf;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };

    (
        @collect
        rust_name = $rust_name:ident;
        whim_name = $whim_name:literal;
        interfaces = [$($interface:literal),*];
        methods = [$($method:ident : $signature:literal => $operation:ident;)*];
        calls = [$($calls:tt)*];
        groups = [named_type $(, $rest:ident)*];
    ) => {
        reflection_class_methods! {
            @collect
            rust_name = $rust_name;
            whim_name = $whim_name;
            interfaces = [$($interface),*];
            methods = [
                $($method : $signature => $operation;)*
                get_declaration: "getDeclaration(): Whim\\Reflection\\SymbolReflection|Whim\\Reflection\\Member\\MethodReflection|Whim\\Reflection\\Member\\ClassConstantReflection|Whim\\Reflection\\Member\\EnumCaseReflection" => Declaration;
                get_type_arguments: "getTypeArguments(): vec<Whim\\Reflection\\Type\\TypeReflection>" => TypeArguments;
                get_type_environment: "getTypeEnvironment(): Whim\\Reflection\\Generic\\TypeEnvironmentReflection" => TypeEnvironment;
                is_recursive_reference: "isRecursiveReference(): bool" => IsRecursiveReference;
            ];
            calls = [$($calls)*];
            groups = [$($rest),*];
        }
    };
}

reflection_class! {
    SourceLocation = "Whim\\Reflection\\SourceLocation" {
        get_file: "getFile(): string" => File;
        get_start_offset: "getStartOffset(): (0..)" => StartOffset;
        get_end_offset: "getEndOffset(): (0..)" => EndOffset;
        get_start_line: "getStartLine(): (1..)" => StartLine;
        get_start_column: "getStartColumn(): (1..)" => StartColumn;
        get_end_line: "getEndLine(): (1..)" => EndLine;
        get_end_column: "getEndColumn(): (1..)" => EndColumn;
    }
}

reflection_class! {
    FileReflection = "Whim\\Reflection\\FileReflection"
    implements ["Whim\\Reflection\\DeclarationReflection"]
    with [declaration] {
        get_path: "getPath(): null|string" => Path;
        get_symbols: "getSymbols(null|Whim\\Symbol\\SymbolKind $kind = null): vec<Whim\\Reflection\\SymbolReflection>" => Symbols;
        has_top_level_code: "hasTopLevelCode(): bool" => HasTopLevelCode;
    }
}

reflection_class! {
    ClassReflection = "Whim\\Reflection\\ClassReflection"
    implements ["Whim\\Reflection\\ClassLikeReflection"]
    with [declaration, symbol, generic, class_like] {
        is_abstract: "isAbstract(): bool" => IsAbstract;
        is_final: "isFinal(): bool" => IsFinal;
        is_readonly: "isReadonly(): bool" => IsReadonly;
        is_instantiable: "isInstantiable(): bool" => IsInstantiable;
        is_cloneable: "isCloneable(): bool" => IsCloneable;
        get_attribute_definition: "getAttributeDefinition(): null|Whim\\Reflection\\Attribute\\DefinitionReflection" => AttributeDefinition;
        get_parent_type: "getParentType(): null|Whim\\Reflection\\Type\\ClassTypeReflection" => ParentType;
        get_direct_interface_types: "getDirectInterfaceTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => DirectInterfaceTypes;
        get_interface_types: "getInterfaceTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => InterfaceTypes;
        get_constructor: "getConstructor(): null|Whim\\Reflection\\Member\\MethodReflection" => Constructor;
        get_destructor: "getDestructor(): null|Whim\\Reflection\\Member\\MethodReflection" => Destructor;
    }
    calls {
        instantiate: "instantiate(vec<Whim\\Reflection\\Type\\TypeReflection> $typeArguments = vec[], vec<mixed> $arguments = vec[]): object" => Instantiate;
    }
}

reflection_class! {
    InterfaceReflection = "Whim\\Reflection\\InterfaceReflection"
    implements ["Whim\\Reflection\\ClassLikeReflection"]
    with [declaration, symbol, generic, class_like] {
        get_direct_parent_types: "getDirectParentTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => DirectParentTypes;
        get_parent_types: "getParentTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => ParentTypes;
    }
}

reflection_class! {
    EnumReflection = "Whim\\Reflection\\EnumReflection"
    implements ["Whim\\Reflection\\ClassLikeReflection"]
    with [declaration, symbol, generic, class_like] {
        get_backing_type: "getBackingType(): null|Whim\\Reflection\\Type\\TypeReflection" => BackingType;
        get_cases: "getCases(): vec<Whim\\Reflection\\Member\\EnumCaseReflection>" => Cases;
        get_case: "getCase(string $name): null|Whim\\Reflection\\Member\\EnumCaseReflection" => Case;
        get_direct_interface_types: "getDirectInterfaceTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => DirectInterfaceTypes;
        get_interface_types: "getInterfaceTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => InterfaceTypes;
    }
}

reflection_class! {
    TypeAliasReflection = "Whim\\Reflection\\TypeAliasReflection"
    implements [
        "Whim\\Reflection\\SymbolReflection",
        "Whim\\Reflection\\Generic\\GenericDeclarationReflection",
    ]
    with [declaration, symbol, generic] {
        get_aliased_type: "getAliasedType(): Whim\\Reflection\\Type\\TypeReflection" => AliasedType;
        get_type: "getType(): Whim\\Reflection\\Type\\SymbolTypeReflection" => Type;
    }
}

reflection_class! {
    NewtypeReflection = "Whim\\Reflection\\NewtypeReflection"
    implements [
        "Whim\\Reflection\\SymbolReflection",
        "Whim\\Reflection\\Generic\\GenericDeclarationReflection",
    ]
    with [declaration, symbol, generic] {
        get_backing_type: "getBackingType(): Whim\\Reflection\\Type\\TypeReflection" => BackingType;
        get_type: "getType(): Whim\\Reflection\\Type\\SymbolTypeReflection" => Type;
    }
}

reflection_class! {
    FunctionReflection = "Whim\\Reflection\\FunctionReflection"
    implements [
        "Whim\\Reflection\\SymbolReflection",
        "Whim\\Reflection\\FunctionLikeReflection",
        "Whim\\Reflection\\InvokableReflection",
    ]
    with [declaration, symbol, generic, callable] {
        get_type: "getType(): Whim\\Reflection\\Type\\SymbolTypeReflection" => Type;
    }
    calls {
        invoke: "invoke(vec<Whim\\Reflection\\Type\\TypeReflection> $typeArguments = vec[], vec<mixed> $arguments = vec[]): mixed" => Invoke;
    }
}

reflection_class! {
    ConstantReflection = "Whim\\Reflection\\ConstantReflection"
    implements ["Whim\\Reflection\\SymbolReflection"]
    with [declaration, symbol] {
        get_value_type: "getValueType(): Whim\\Reflection\\Type\\TypeReflection" => ValueType;
        get_value: "getValue(): mixed" => Value;
        get_type: "getType(): Whim\\Reflection\\Type\\SymbolTypeReflection" => Type;
    }
}

reflection_class! {
    MethodReflection = "Whim\\Reflection\\Member\\MethodReflection"
    implements [
        "Whim\\Reflection\\Member\\MemberReflection",
        "Whim\\Reflection\\FunctionLikeReflection",
    ]
    with [declaration, member, generic, callable] {
        is_static: "isStatic(): bool" => IsStatic;
        is_abstract: "isAbstract(): bool" => IsAbstract;
        is_final: "isFinal(): bool" => IsFinal;
        is_constructor: "isConstructor(): bool" => IsConstructor;
        is_destructor: "isDestructor(): bool" => IsDestructor;
        get_prototypes: "getPrototypes(): vec<Whim\\Reflection\\Member\\MethodReflection>" => Prototypes;
        get_type: "getType(): Whim\\Reflection\\Type\\MemberTypeReflection" => Type;
    }
    calls {
        invoke: "invoke(classname<object>|object $target, vec<Whim\\Reflection\\Type\\TypeReflection> $typeArguments = vec[], vec<mixed> $arguments = vec[]): mixed" => Invoke;
    }
}

reflection_class! {
    PropertyReflection = "Whim\\Reflection\\Member\\PropertyReflection"
    implements ["Whim\\Reflection\\Member\\MemberReflection"]
    with [declaration, member] {
        is_static: "isStatic(): bool" => IsStatic;
        is_readonly: "isReadonly(): bool" => IsReadonly;
        is_promoted: "isPromoted(): bool" => IsPromoted;
        get_declared_type: "getDeclaredType(): null|Whim\\Reflection\\Type\\TypeReflection" => DeclaredType;
        get_prototypes: "getPrototypes(): vec<Whim\\Reflection\\Member\\PropertyReflection>" => Prototypes;
        has_default_value: "hasDefaultValue(): bool" => HasDefaultValue;
        get_default_value: "getDefaultValue(): mixed" => DefaultValue;
        is_static_initialized: "isStaticInitialized(): bool" => IsStaticInitialized;
        get_static_value: "getStaticValue(): mixed" => StaticValue;
    }
}

reflection_class! {
    ClassConstantReflection = "Whim\\Reflection\\Member\\ClassConstantReflection"
    implements ["Whim\\Reflection\\Member\\MemberReflection"]
    with [declaration, member] {
        get_declared_type: "getDeclaredType(): null|Whim\\Reflection\\Type\\TypeReflection" => DeclaredType;
        get_value_type: "getValueType(): Whim\\Reflection\\Type\\TypeReflection" => ValueType;
        get_value: "getValue(): mixed" => Value;
        get_prototypes: "getPrototypes(): vec<Whim\\Reflection\\Member\\ClassConstantReflection>" => Prototypes;
        get_type: "getType(): Whim\\Reflection\\Type\\MemberTypeReflection" => Type;
    }
}

reflection_class! {
    EnumCaseReflection = "Whim\\Reflection\\Member\\EnumCaseReflection"
    implements ["Whim\\Reflection\\Member\\MemberReflection"]
    with [declaration, member] {
        get_enum: "getEnum(): Whim\\Reflection\\EnumReflection" => Enum;
        get_backing_value: "getBackingValue(): null|int|uint|string" => BackingValue;
        get_value: "getValue(): object" => Value;
        get_type: "getType(): Whim\\Reflection\\Type\\MemberTypeReflection" => Type;
    }
}

reflection_class! {
    ParameterReflection = "Whim\\Reflection\\ParameterReflection"
    implements ["Whim\\Reflection\\DeclarationReflection"]
    with [declaration] {
        get_name: "getName(): string" => Name;
        get_position: "getPosition(): (0..)" => Position;
        get_declaring_function_like: "getDeclaringFunctionLike(): Whim\\Reflection\\FunctionLikeReflection" => DeclaringFunctionLike;
        get_declared_type: "getDeclaredType(): null|Whim\\Reflection\\Type\\TypeReflection" => DeclaredType;
        get_type: "getType(null|Whim\\Reflection\\Generic\\TypeEnvironmentReflection $environment = null): null|Whim\\Reflection\\Type\\TypeReflection" => Type;
        is_optional: "isOptional(): bool" => IsOptional;
        has_default_value: "hasDefaultValue(): bool" => HasDefaultValue;
        get_default_value: "getDefaultValue(): mixed" => DefaultValue;
        is_sensitive: "isSensitive(): bool" => IsSensitive;
        get_promoted_property: "getPromotedProperty(): null|Whim\\Reflection\\Member\\PropertyReflection" => PromotedProperty;
    }
}

reflection_class! {
    ClosureReflection = "Whim\\Reflection\\ClosureReflection"
    implements ["Whim\\Reflection\\FunctionLikeReflection"]
    with [declaration, generic, callable_name, callable] {
        get_captures: "getCaptures(): vec<Whim\\Reflection\\CaptureReflection>" => Captures;
    }
}

reflection_class! {
    CaptureReflection = "Whim\\Reflection\\CaptureReflection" {
        get_name: "getName(): string" => Name;
        get_position: "getPosition(): (0..)" => Position;
        is_receiver: "isReceiver(): bool" => IsReceiver;
        get_location: "getLocation(): null|Whim\\Reflection\\SourceLocation" => Location;
    }
}

reflection_class! {
    CallableReflection = "Whim\\Reflection\\CallableReflection"
    implements ["Whim\\Reflection\\InvokableReflection"] {
        is_closure: "isClosure(): bool" => IsClosure;
        is_first_class_callable: "isFirstClassCallable(): bool" => IsFirstClassCallable;
        is_partial_function_application: "isPartialFunctionApplication(): bool" => IsPartialFunctionApplication;
        get_declaration: "getDeclaration(): Whim\\Reflection\\FunctionLikeReflection" => Declaration;
        get_type: "getType(): Whim\\Reflection\\Type\\FunctionTypeReflection" => Type;
        get_type_environment: "getTypeEnvironment(): Whim\\Reflection\\Generic\\TypeEnvironmentReflection" => TypeEnvironment;
        get_bound_object: "getBoundObject(): null|object" => BoundObject;
        get_scope_class: "getScopeClass(): null|Whim\\Reflection\\ClassLikeReflection" => ScopeClass;
        get_called_type: "getCalledType(): null|Whim\\Reflection\\Type\\ClassTypeReflection" => CalledType;
        get_captures: "getCaptures(): vec<Whim\\Reflection\\CaptureValueReflection>" => Captures;
        get_bound_arguments: "getBoundArguments(): vec<Whim\\Reflection\\BoundArgumentReflection>" => BoundArguments;
    }
    calls {
        invoke: "invoke(vec<Whim\\Reflection\\Type\\TypeReflection> $typeArguments = vec[], vec<mixed> $arguments = vec[]): mixed" => Invoke;
    }
}

reflection_class! {
    CaptureValueReflection = "Whim\\Reflection\\CaptureValueReflection" {
        get_capture: "getCapture(): Whim\\Reflection\\CaptureReflection" => Capture;
        get_type: "getType(): Whim\\Reflection\\Type\\TypeReflection" => Type;
        get_value: "getValue(): mixed" => Value;
    }
}

reflection_class! {
    BoundArgumentReflection = "Whim\\Reflection\\BoundArgumentReflection" {
        get_parameter: "getParameter(): Whim\\Reflection\\ParameterReflection" => Parameter;
        get_type: "getType(): Whim\\Reflection\\Type\\TypeReflection" => Type;
        get_value: "getValue(): mixed" => Value;
    }
}

reflection_class! {
    TypeParameterReflection = "Whim\\Reflection\\Generic\\TypeParameterReflection" {
        get_name: "getName(): string" => Name;
        get_position: "getPosition(): (0..)" => Position;
        get_declaring_declaration: "getDeclaringDeclaration(): Whim\\Reflection\\Generic\\GenericDeclarationReflection" => DeclaringDeclaration;
        get_variance: "getVariance(): Whim\\Reflection\\Generic\\Variance" => Variance;
        get_bounds: "getBounds(): vec<Whim\\Reflection\\Type\\TypeReflection>" => Bounds;
        get_default: "getDefault(): null|Whim\\Reflection\\Type\\TypeReflection" => Default;
        get_type: "getType(): Whim\\Reflection\\Type\\TypeParameterTypeReflection" => Type;
        get_location: "getLocation(): null|Whim\\Reflection\\SourceLocation" => Location;
    }
}

reflection_class! {
    WhereConstraintReflection = "Whim\\Reflection\\Generic\\WhereConstraintReflection" {
        get_parameter: "getParameter(): Whim\\Reflection\\Generic\\TypeParameterReflection" => Parameter;
        get_bound: "getBound(): Whim\\Reflection\\Type\\TypeReflection" => Bound;
        get_declaring_function_like: "getDeclaringFunctionLike(): Whim\\Reflection\\FunctionLikeReflection" => DeclaringFunctionLike;
        get_position: "getPosition(): (0..)" => Position;
        get_location: "getLocation(): null|Whim\\Reflection\\SourceLocation" => Location;
    }
}

reflection_class! {
    TypeBindingReflection = "Whim\\Reflection\\Generic\\TypeBindingReflection" {
        get_parameter: "getParameter(): Whim\\Reflection\\Generic\\TypeParameterReflection" => Parameter;
        get_argument: "getArgument(): Whim\\Reflection\\Type\\TypeReflection" => Argument;
    }
}

reflection_class! {
    TypeEnvironmentReflection = "Whim\\Reflection\\Generic\\TypeEnvironmentReflection" {
        get_bindings: "getBindings(): vec<Whim\\Reflection\\Generic\\TypeBindingReflection>" => Bindings;
        get: "get(Whim\\Reflection\\Generic\\TypeParameterReflection $parameter): null|Whim\\Reflection\\Type\\TypeReflection" => Binding;
    }
}

#[whim_class("Whim\\Reflection\\AttributeReflection<out T: object>", final, traced)]
pub(crate) struct AttributeReflection(pub(crate) ReflectionState);

impl AttributeReflection {
    #[expect(clippy::unnecessary_wraps, reason = "built-in constructor contract")]
    pub(crate) fn new(_vm: &mut VirtualMachine<'_>) -> Result<Self, Throw> {
        Ok(Self(ReflectionState::default()))
    }
}

// SAFETY: the wrapper delegates its complete child set to its sole state.
unsafe impl BuiltInChildren for AttributeReflection {
    fn enqueue_built_in_children(&mut self, queue: &DropQueue, mode: TeardownMode) {
        self.0.enqueue_children(queue, mode);
    }

    fn visit_built_in_children(&self, visitor: &mut TraceVisitor<'_>) {
        self.0.visit_children(visitor);
    }
}

#[whim_methods(generics = "<out T: object>")]
impl AttributeReflection {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("getClass(): Whim\\Reflection\\ClassReflection", must_use)]
    fn get_class(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        dispatch(context, arguments, Operation::Class)
    }

    #[whim_method("getTarget(): Whim\\Reflection\\DeclarationReflection", must_use)]
    fn get_target(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        dispatch(context, arguments, Operation::Target)
    }

    #[whim_method("getArguments(): vec<mixed>", must_use)]
    fn get_arguments(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        dispatch(context, arguments, Operation::Arguments)
    }

    #[whim_method("getNamedArguments(): dict<string, mixed>", must_use)]
    fn get_named_arguments(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        dispatch(context, arguments, Operation::NamedArguments)
    }

    #[whim_method("getLocation(): null|Whim\\Reflection\\SourceLocation", must_use)]
    fn get_location(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        dispatch(context, arguments, Operation::Location)
    }

    #[whim_method("newInstance(): T", must_use)]
    fn new_instance(
        context: &mut Context<'_, '_, '_>,
        arguments: Arguments<'_>,
    ) -> Result<Value, Throw> {
        dispatch(context, arguments, Operation::NewInstance)
    }
}

reflection_class! {
    AttributeDefinitionReflection = "Whim\\Reflection\\Attribute\\DefinitionReflection" {
        get_class: "getClass(): Whim\\Reflection\\ClassReflection" => Class;
        get_targets: "getTargets(): vec<Whim\\Reflection\\Attribute\\Target>" => Targets;
        is_repeatable: "isRepeatable(): bool" => IsRepeatable;
    }
}

reflection_class! {
    ObjectReflection = "Whim\\Reflection\\ObjectReflection" {
        get_class: "getClass(): Whim\\Reflection\\ClassReflection|Whim\\Reflection\\EnumReflection" => Class;
        get_enum_case: "getEnumCase(): null|Whim\\Reflection\\Member\\EnumCaseReflection" => EnumCase;
        get_type: "getType(): Whim\\Reflection\\Type\\ClassTypeReflection" => Type;
        get_type_environment: "getTypeEnvironment(): Whim\\Reflection\\Generic\\TypeEnvironmentReflection" => TypeEnvironment;
        get_type_argument: "getTypeArgument(Whim\\Reflection\\Generic\\TypeParameterReflection $parameter): null|Whim\\Reflection\\Type\\TypeReflection" => TypeArgument;
        is_instance_of: "isInstanceOf(Whim\\Reflection\\Type\\ClassTypeReflection $type): bool" => IsInstanceOf;
        get_specialization: "getSpecialization(Whim\\Reflection\\ClassLikeReflection $declaration): null|Whim\\Reflection\\Type\\ClassTypeReflection" => Specialization;
        get_properties: "getProperties(): vec<Whim\\Reflection\\PropertyValueReflection>" => PropertyValues;
        get_property: "getProperty(Whim\\Reflection\\Member\\PropertyReflection $property): null|Whim\\Reflection\\PropertyValueReflection" => PropertyValue;
    }
}

reflection_class! {
    PropertyValueReflection = "Whim\\Reflection\\PropertyValueReflection" {
        get_property: "getProperty(): Whim\\Reflection\\Member\\PropertyReflection" => Property;
        is_initialized: "isInitialized(): bool" => IsInitialized;
        get_declared_type: "getDeclaredType(): null|Whim\\Reflection\\Type\\TypeReflection" => DeclaredType;
        get_value_type: "getValueType(): null|Whim\\Reflection\\Type\\TypeReflection" => ValueType;
        get_value: "getValue(): mixed" => Value;
    }
}

reflection_class! {
    NewtypeValueReflection = "Whim\\Reflection\\NewtypeValueReflection" {
        get_declaration: "getDeclaration(): Whim\\Reflection\\NewtypeReflection" => Declaration;
        get_type: "getType(): Whim\\Reflection\\Type\\SymbolTypeReflection" => Type;
        get_type_environment: "getTypeEnvironment(): Whim\\Reflection\\Generic\\TypeEnvironmentReflection" => TypeEnvironment;
        get_backing_value: "getBackingValue(): mixed" => BackingValue;
    }
}

reflection_class! {
    PrimitiveTypeReflection = "Whim\\Reflection\\Type\\PrimitiveTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {}
}

reflection_class! {
    LiteralTypeReflection = "Whim\\Reflection\\Type\\LiteralTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_value: "getValue(): null|bool|int|uint|float|string" => Value;
    }
}

reflection_class! {
    IntegerRangeTypeReflection = "Whim\\Reflection\\Type\\IntegerRangeTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_lower_bound: "getLowerBound(): null|int|uint" => LowerBound;
        get_upper_bound: "getUpperBound(): null|int|uint" => UpperBound;
    }
}

reflection_class! {
    StringLengthTypeReflection = "Whim\\Reflection\\Type\\StringLengthTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_minimum_length: "getMinimumLength(): uint" => MinimumLength;
        get_maximum_length: "getMaximumLength(): null|uint" => MaximumLength;
    }
}

reflection_class! {
    ClassTypeReflection = "Whim\\Reflection\\Type\\ClassTypeReflection"
    implements ["Whim\\Reflection\\Type\\NamedTypeReflection"]
    with [type_reflection, named_type] {
        get_class_like: "getClassLike(): Whim\\Reflection\\ClassLikeReflection" => ClassLike;
        get_specialization: "getSpecialization(Whim\\Reflection\\ClassLikeReflection $declaration): null|Whim\\Reflection\\Type\\ClassTypeReflection" => Specialization;
        get_base_types: "getBaseTypes(): vec<Whim\\Reflection\\Type\\ClassTypeReflection>" => BaseTypes;
    }
}

reflection_class! {
    SymbolTypeReflection = "Whim\\Reflection\\Type\\SymbolTypeReflection"
    implements ["Whim\\Reflection\\Type\\NamedTypeReflection"]
    with [type_reflection, named_type] {}
}

reflection_class! {
    MemberTypeReflection = "Whim\\Reflection\\Type\\MemberTypeReflection"
    implements ["Whim\\Reflection\\Type\\NamedTypeReflection"]
    with [type_reflection, named_type] {
        get_class_type: "getClassType(): Whim\\Reflection\\Type\\ClassTypeReflection" => ClassType;
    }
}

reflection_class! {
    TypeParameterTypeReflection = "Whim\\Reflection\\Type\\TypeParameterTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_parameter: "getParameter(): Whim\\Reflection\\Generic\\TypeParameterReflection" => Parameter;
    }
}

reflection_class! {
    StaticTypeReflection = "Whim\\Reflection\\Type\\StaticTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_declaring_type: "getDeclaringType(): Whim\\Reflection\\ClassLikeReflection" => DeclaringType;
    }
}

reflection_class! {
    UnionTypeReflection = "Whim\\Reflection\\Type\\UnionTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_types: "getTypes(): vec<Whim\\Reflection\\Type\\TypeReflection>" => Types;
    }
}

reflection_class! {
    IntersectionTypeReflection = "Whim\\Reflection\\Type\\IntersectionTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_types: "getTypes(): vec<Whim\\Reflection\\Type\\TypeReflection>" => Types;
    }
}

reflection_class! {
    NegatedTypeReflection = "Whim\\Reflection\\Type\\NegatedTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_type: "getType(): Whim\\Reflection\\Type\\TypeReflection" => InnerType;
    }
}

reflection_class! {
    FunctionTypeReflection = "Whim\\Reflection\\Type\\FunctionTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_parameters: "getParameters(): vec<Whim\\Reflection\\Type\\FunctionTypeParameterReflection>" => Parameters;
        get_return_type: "getReturnType(): null|Whim\\Reflection\\Type\\TypeReflection" => ReturnType;
    }
}

reflection_class! {
    FunctionTypeParameterReflection = "Whim\\Reflection\\Type\\FunctionTypeParameterReflection" {
        get_position: "getPosition(): (0..)" => Position;
        get_type: "getType(): Whim\\Reflection\\Type\\TypeReflection" => Type;
        is_optional: "isOptional(): bool" => IsOptional;
    }
}

reflection_class! {
    ArrayTypeReflection = "Whim\\Reflection\\Type\\ArrayTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_key_type: "getKeyType(): null|Whim\\Reflection\\Type\\TypeReflection" => KeyType;
        get_value_type: "getValueType(): null|Whim\\Reflection\\Type\\TypeReflection" => ValueType;
    }
}

reflection_class! {
    VecTypeReflection = "Whim\\Reflection\\Type\\VecTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_value_type: "getValueType(): null|Whim\\Reflection\\Type\\TypeReflection" => ValueType;
    }
}

reflection_class! {
    VecShapeTypeReflection = "Whim\\Reflection\\Type\\VecShapeTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_types: "getTypes(): vec<Whim\\Reflection\\Type\\TypeReflection>" => Types;
        get_rest_type: "getRestType(): null|Whim\\Reflection\\Type\\TypeReflection" => RestType;
    }
}

reflection_class! {
    DictTypeReflection = "Whim\\Reflection\\Type\\DictTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_key_type: "getKeyType(): null|Whim\\Reflection\\Type\\TypeReflection" => KeyType;
        get_value_type: "getValueType(): null|Whim\\Reflection\\Type\\TypeReflection" => ValueType;
    }
}

reflection_class! {
    ObjectShapeTypeReflection = "Whim\\Reflection\\Type\\ObjectShapeTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_properties: "getProperties(): vec<Whim\\Reflection\\Type\\ObjectShapePropertyReflection>" => Properties;
        is_open: "isOpen(): bool" => IsOpen;
    }
}

reflection_class! {
    ObjectShapePropertyReflection = "Whim\\Reflection\\Type\\ObjectShapePropertyReflection" {
        get_name: "getName(): string" => Name;
        get_type: "getType(): Whim\\Reflection\\Type\\TypeReflection" => Type;
    }
}

reflection_class! {
    DictShapeTypeReflection = "Whim\\Reflection\\Type\\DictShapeTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_entries: "getEntries(): vec<Whim\\Reflection\\Type\\DictShapeEntryReflection>" => Entries;
        get_rest_key_type: "getRestKeyType(): null|Whim\\Reflection\\Type\\TypeReflection" => RestKeyType;
        get_rest_value_type: "getRestValueType(): null|Whim\\Reflection\\Type\\TypeReflection" => RestValueType;
    }
}

reflection_class! {
    DictShapeEntryReflection = "Whim\\Reflection\\Type\\DictShapeEntryReflection" {
        get_key: "getKey(): bool|int|uint|string" => Key;
        get_type: "getType(): Whim\\Reflection\\Type\\TypeReflection" => Type;
    }
}

reflection_class! {
    ClassnameTypeReflection = "Whim\\Reflection\\Type\\ClassnameTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_object_type: "getObjectType(): Whim\\Reflection\\Type\\TypeReflection" => ObjectType;
    }
}

reflection_class! {
    TupleTypeReflection = "Whim\\Reflection\\Type\\TupleTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {
        get_types: "getTypes(): vec<Whim\\Reflection\\Type\\TypeReflection>" => Types;
        get_rest_type: "getRestType(): null|Whim\\Reflection\\Type\\TypeReflection" => RestType;
    }
}

reflection_class! {
    WildcardTypeReflection = "Whim\\Reflection\\Type\\WildcardTypeReflection"
    implements ["Whim\\Reflection\\Type\\TypeReflection"]
    with [type_reflection] {}
}

pub(crate) fn state(value: &Value) -> Option<&ReflectionState> {
    macro_rules! find {
        ($($class:ty),* $(,)?) => {
            $(
                if let Some(object) = state_ref::<$class>(value) {
                    return Some(&object.0);
                }
            )*
        };
    }

    find!(
        SourceLocation,
        FileReflection,
        ClassReflection,
        InterfaceReflection,
        EnumReflection,
        TypeAliasReflection,
        NewtypeReflection,
        FunctionReflection,
        ConstantReflection,
        MethodReflection,
        PropertyReflection,
        ClassConstantReflection,
        EnumCaseReflection,
        ParameterReflection,
        ClosureReflection,
        CaptureReflection,
        CallableReflection,
        CaptureValueReflection,
        BoundArgumentReflection,
        TypeParameterReflection,
        WhereConstraintReflection,
        TypeBindingReflection,
        TypeEnvironmentReflection,
        AttributeReflection,
        AttributeDefinitionReflection,
        ObjectReflection,
        PropertyValueReflection,
        NewtypeValueReflection,
        PrimitiveTypeReflection,
        LiteralTypeReflection,
        IntegerRangeTypeReflection,
        StringLengthTypeReflection,
        ClassTypeReflection,
        SymbolTypeReflection,
        MemberTypeReflection,
        TypeParameterTypeReflection,
        StaticTypeReflection,
        UnionTypeReflection,
        IntersectionTypeReflection,
        NegatedTypeReflection,
        FunctionTypeReflection,
        FunctionTypeParameterReflection,
        ArrayTypeReflection,
        VecTypeReflection,
        VecShapeTypeReflection,
        DictTypeReflection,
        ObjectShapeTypeReflection,
        ObjectShapePropertyReflection,
        DictShapeTypeReflection,
        DictShapeEntryReflection,
        ClassnameTypeReflection,
        TupleTypeReflection,
        WildcardTypeReflection,
    );

    None
}
