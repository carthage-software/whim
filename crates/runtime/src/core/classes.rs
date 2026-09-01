//! Engine-owned class identifiers and handlers used by the Rust-backed core.

#![expect(
    clippy::string_lit_as_bytes,
    reason = "the class macro derives byte constants and docs from one string literal"
)]

use crate::builtin::Context;
use crate::builtin::spec::CoreDeclarations;
use crate::builtin::spec::ParameterSpec;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::classes::BuiltInMethodBody;
use crate::unreachable_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;
use crate::value::object::ClassId;

macro_rules! define_core_classes {
    (
        groups {
            $(
                $(#[$group_attribute:meta])*
                $group:ident {
                    $($field:ident => $constant:ident = $name:literal,)*
                }
            )*
        }
        required {
            $($required_constant:ident = $required_name:literal,)*
        }
    ) => {
        /// Fully qualified names of classes the engine requires from its core.
        pub(crate) mod names {
            $(
                $(pub(crate) const $constant: &[u8] = $name.as_bytes();)*
            )*
            $(pub(crate) const $required_constant: &[u8] = $required_name.as_bytes();)*
        }

        $(
            $(#[$group_attribute])*
            #[derive(Debug, Clone, Copy)]
            pub(crate) struct $group {
                $(
                    #[doc = concat!("`", $name, "`.")]
                    pub(crate) $field: ClassId,
                )*
            }

            impl $group {
                pub(crate) fn resolve(core: &CoreDeclarations) -> Self {
                    Self {
                        $($field: core_class(core, names::$constant),)*
                    }
                }
            }
        )*

        pub(crate) fn validate_required_classes(core: &CoreDeclarations) {
            $(core_class(core, names::$required_constant);)*
        }
    };
}

define_core_classes! {
    groups {
        WellKnown {
            throwable => THROWABLE = "Whim\\Unwind\\Throwable",
            error => ERROR = "Whim\\Unwind\\Error",
            parser_error => PARSER_ERROR = "Whim\\Unwind\\ParserError",
            compiler_error => COMPILER_ERROR = "Whim\\Unwind\\CompilerError",
            linker_error => LINKER_ERROR = "Whim\\Unwind\\LinkerError",
            type_error => TYPE_ERROR = "Whim\\Unwind\\TypeError",
            incompatible_operands_error => INCOMPATIBLE_OPERANDS_ERROR = "Whim\\Unwind\\IncompatibleOperandsError",
            arithmetic_error => ARITHMETIC_ERROR = "Whim\\Unwind\\ArithmeticError",
            division_by_zero_error => DIVISION_BY_ZERO_ERROR = "Whim\\Unwind\\DivisionByZeroError",
            overflow_error => OVERFLOW_ERROR = "Whim\\Unwind\\OverflowError",
            underflow_error => UNDERFLOW_ERROR = "Whim\\Unwind\\UnderflowError",
            argument_count_error => ARGUMENT_COUNT_ERROR = "Whim\\Unwind\\ArgumentCountError",
            out_of_bounds_error => OUT_OF_BOUNDS_ERROR = "Whim\\Unwind\\OutOfBoundsError",
            unhandled_match_error => UNHANDLED_MATCH_ERROR = "Whim\\Unwind\\UnhandledMatchError",
            assertion_error => ASSERTION_ERROR = "Whim\\Unwind\\AssertionError",
            undefined_variable_error => UNDEFINED_VARIABLE_ERROR = "Whim\\Unwind\\UndefinedVariableError",
            uninitialized_property_error => UNINITIALIZED_PROPERTY_ERROR = "Whim\\Unwind\\UninitializedPropertyError",
            readonly_error => READONLY_ERROR = "Whim\\Unwind\\ReadonlyError",
            leaked_resource_error => LEAKED_RESOURCE_ERROR = "Whim\\Unwind\\LeakedResourceError",
            discarded_result_error => DISCARDED_RESULT_ERROR = "Whim\\Unwind\\DiscardedResultError",
            visibility_error => VISIBILITY_ERROR = "Whim\\Unwind\\VisibilityError",
            instantiation_error => INSTANTIATION_ERROR = "Whim\\Unwind\\InstantiationError",
            undefined_symbol_error => UNDEFINED_SYMBOL_ERROR = "Whim\\Unwind\\UndefinedSymbolError",
            require_error => REQUIRE_ERROR = "Whim\\Unwind\\RequireError",
            stack_overflow_error => STACK_OVERFLOW_ERROR = "Whim\\Unwind\\StackOverflowError",
            coroutine_error => COROUTINE_ERROR = "Whim\\Unwind\\CoroutineError",
            trace_frame => TRACE_FRAME = "Whim\\Unwind\\TraceFrame",
        }
        IterateClasses {
            iterator => ITERATOR = "Whim\\Iterate\\Iterator",
            to_iterator => TO_ITERATOR = "Whim\\Iterate\\ToIterator",
        }
        EnumClasses {
            unit => UNIT_ENUM = "Whim\\Enum\\UnitEnum",
            backed => BACKED_ENUM = "Whim\\Enum\\BackedEnum",
        }
        WhimClasses {
            sensitive_parameter_value => SENSITIVE_PARAMETER_VALUE = "Whim\\Marker\\SensitiveParameterValue",
        }
    }
    required {
        ATTRIBUTE = "Whim\\Attribute\\Attribute",
        DEPRECATED = "Whim\\Marker\\Deprecated",
        SENSITIVE_PARAMETER = "Whim\\Marker\\SensitiveParameter",
        WEAK = "Whim\\Reference\\Weak",
        WEAK_MAP = "Whim\\Reference\\WeakMap",
    }
}

fn core_class(core: &CoreDeclarations, name: &[u8]) -> ClassId {
    let index = core
        .interfaces
        .iter()
        .map(|spec| spec.name)
        .chain(core.classes.iter().map(|spec| spec.name))
        .chain(core.enums.iter().map(|spec| spec.name))
        .position(|candidate| candidate.as_bytes() == name);
    let Some(index) = index else {
        panic!(
            "the generated core does not declare {}",
            String::from_utf8_lossy(name)
        )
    };
    // SAFETY: the surrounding invariant proves this result is successful.
    let index = unsafe {
        unwrap_result_invariant(
            u32::try_from(index),
            "the core class table fits in the class identifier range",
        )
    };
    ClassId(index)
}

pub(crate) const ERROR_SLOT_MESSAGE: usize = 0;
pub(crate) const ERROR_SLOT_CODE: usize = 1;
pub(crate) const ERROR_SLOT_TRACE: usize = 4;
pub(crate) const ERROR_SLOT_PREVIOUS: usize = 5;

pub(crate) const TRACE_FRAME_SLOT_FUNCTION: usize = 0;
pub(crate) const TRACE_FRAME_SLOT_FILE: usize = 1;
pub(crate) const TRACE_FRAME_SLOT_LINE: usize = 2;
pub(crate) const TRACE_FRAME_SLOT_ARGUMENTS: usize = 3;

/// The placeholder body of an abstract interface method; the abstractness
/// check throws before any dispatch could reach it.
pub(crate) fn abstract_method_body<'call>(
    _context: &mut Context<'call, '_, '_>,
    _window: &'call [Value],
) -> Result<Value, Throw> {
    // SAFETY: the surrounding invariant makes this path unreachable.
    unsafe { unreachable_invariant("abstract methods are never invoked") }
}

static ENUM_BACKING_PARAMETER: [ParameterSpec; 1] = [ParameterSpec {
    name: "value",
    type_spec: TypeSpec::Union(&[TypeSpec::Int, TypeSpec::String]),
    optional: false,
    default: None,
    sensitive: false,
}];

static ENUM_STATIC_TYPE: TypeSpec = TypeSpec::Static;

pub(crate) fn enum_cases_body() -> BuiltInMethodBody {
    BuiltInMethodBody {
        handler: enum_cases_handler,
        type_parameters: &[],
        parameters: &[],
        return_spec: TypeSpec::VectorOf(&ENUM_STATIC_TYPE),
        signature: "fn(): vec<static>",
        attributes: BuiltInCallableAttributes::for_whim_symbol("Whim\\Enum"),
    }
}

pub(crate) fn enum_from_body() -> BuiltInMethodBody {
    BuiltInMethodBody {
        handler: enum_from_handler,
        type_parameters: &[],
        parameters: &ENUM_BACKING_PARAMETER,
        return_spec: TypeSpec::Static,
        signature: "fn(int|string): static",
        attributes: BuiltInCallableAttributes::for_whim_symbol("Whim\\Enum"),
    }
}

pub(crate) fn enum_try_from_body() -> BuiltInMethodBody {
    BuiltInMethodBody {
        handler: enum_try_from_handler,
        type_parameters: &[],
        parameters: &ENUM_BACKING_PARAMETER,
        return_spec: TypeSpec::Optional(&ENUM_STATIC_TYPE),
        signature: "fn(int|string): null|static",
        attributes: BuiltInCallableAttributes::for_whim_symbol("Whim\\Enum"),
    }
}

fn enum_cases_handler<'call>(
    context: &mut Context<'call, '_, '_>,
    _window: &'call [Value],
) -> Result<Value, Throw> {
    let Some(class) = context.called_class() else {
        let error = context.vm.intern(b"Whim\\Unwind\\TypeError");
        return Err(context
            .vm
            .throw(error, "cases() has no called enum class", 0));
    };

    Ok(context.enum_cases(class))
}

fn enum_from_handler<'call>(
    context: &mut Context<'call, '_, '_>,
    window: &'call [Value],
) -> Result<Value, Throw> {
    let Some(class) = context.called_class() else {
        let error = context.vm.intern(b"Whim\\Unwind\\TypeError");
        return Err(context
            .vm
            .throw(error, "from() has no called enum class", 0));
    };

    let Some(value) = window.get(1) else {
        let error = context.vm.intern(b"Whim\\Unwind\\ArgumentCountError");
        return Err(context
            .vm
            .throw(error, "from() expects one backing value", 0));
    };

    let Some(case) = context.enum_case_from_backing(class, value) else {
        let error = context.vm.intern(b"Whim\\Unwind\\ValueError");
        return Err(context
            .vm
            .throw(error, "no enum case has the supplied backing value", 0));
    };

    Ok(case)
}

fn enum_try_from_handler<'call>(
    context: &mut Context<'call, '_, '_>,
    window: &'call [Value],
) -> Result<Value, Throw> {
    let Some(class) = context.called_class() else {
        let error = context.vm.intern(b"Whim\\Unwind\\TypeError");
        return Err(context
            .vm
            .throw(error, "tryFrom() has no called enum class", 0));
    };

    let Some(value) = window.get(1) else {
        let error = context.vm.intern(b"Whim\\Unwind\\ArgumentCountError");
        return Err(context
            .vm
            .throw(error, "tryFrom() expects one backing value", 0));
    };

    Ok(context
        .enum_case_from_backing(class, value)
        .unwrap_or_else(Value::null))
}
