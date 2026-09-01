//! The declarations generated for Rust-backed core symbols.

use std::ptr::NonNull;

use crate::builtin::Context;
use crate::builtin::throw::Throw;
use crate::bytecode::unit::BuiltInCallableMarkers;
use crate::bytecode::unit::EnumBacking;
use crate::bytecode::unit::Variance;
use crate::bytecode::unit::Visibility;
use crate::value::Value;
use crate::value::object::BuiltInHooks;
use crate::vm::VirtualMachine;

pub(crate) type BuiltInHandler =
    for<'call> fn(&mut Context<'call, '_, '_>, &'call [Value]) -> Result<Value, Throw>;

/// A macro-generated direct entry point for an exact built-in function call.
pub(crate) type BuiltInDirectHandler =
    fn(&mut VirtualMachine<'_>, &[Value]) -> Result<Value, Throw>;

pub(crate) type BuiltInInitializer = fn(&mut VirtualMachine<'_>, NonNull<()>) -> Result<(), Throw>;

#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    dead_code,
    reason = "the declaration macros support every legal built-in boundary type"
)]
pub(crate) enum TypeSpec {
    /// An existential slot inside a composite type. It accepts every value
    /// without inspecting it and is distinct from variance-sensitive `mixed`.
    Wildcard,
    Mixed,
    /// A function that completes without a value. Valid only as a return.
    Void,
    /// No value. A parameter of this type can never be passed and a return of
    /// this type can only unwind or terminate execution.
    Never,
    Null,
    Bool,
    Int,
    IntRange(Option<i64>, Option<i64>),
    Float,
    String,
    StringLiteral(&'static [u8]),
    Array,
    Vec,
    Dict,
    Tuple,
    Function,
    Object,
    Static,
    Instance(&'static str),
    Parameter(&'static str),
    GenericInstance(&'static str, &'static [Self]),
    /// A tuple whose arity and element types are fixed.
    TupleOf(&'static [Self]),
    /// A tuple with fixed leading positions and a homogeneous trailing type.
    TupleRest(&'static [Self], &'static Self),
    Optional(&'static Self),
    VectorOf(&'static Self),
    ArrayOf(&'static Self, &'static Self),
    DictionaryOf(&'static Self, &'static Self),
    CallableOf(&'static [CallableParameterSpec], &'static Self),
    Classname(&'static Self),
    Union(&'static [Self]),
    Intersection(&'static [Self]),
    Negated(&'static Self),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CallableParameterSpec {
    pub type_spec: TypeSpec,
    /// Whether callers may omit this parameter.
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TypeParameterSpec {
    pub name: &'static str,
    pub variance: Variance,
    pub bounds: &'static [TypeSpec],
    pub default: Option<TypeSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BaseSpec {
    /// The fully qualified class or interface name.
    pub name: &'static str,
    /// Complete type arguments, or `None` for a non-generic base.
    pub arguments: Option<&'static [TypeSpec]>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParameterSpec {
    /// The parameter name, without the `$`.
    pub name: &'static str,
    pub type_spec: TypeSpec,
    /// Whether the parameter may be omitted at the call site.
    pub optional: bool,
    /// The literal supplied when the parameter is omitted.
    pub default: Option<ParameterDefaultSpec>,
    /// Whether the parameter is sensitive and must be redacted from traces.
    pub sensitive: bool,
}

#[derive(Debug, Clone, Copy)]
#[expect(
    dead_code,
    reason = "the signature macros support every scalar parameter default"
)]
pub(crate) enum ParameterDefaultSpec {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(&'static [u8]),
}

#[derive(Clone, Copy)]
pub(crate) struct FunctionSpec {
    /// The fully qualified name, such as `Whim\Env\get_arguments`.
    pub name: &'static str,
    /// The language-level generic binders.
    pub type_parameters: &'static [TypeParameterSpec],
    /// The parameters, in declaration order.
    pub parameters: &'static [ParameterSpec],
    pub return_spec: TypeSpec,
    pub handler: BuiltInHandler,
    /// The direct exact-call entry point emitted by the function macro.
    pub direct_handler: Option<BuiltInDirectHandler>,
    /// The rendered `fn(...)` type used by diagnostics and callable checks.
    pub signature: &'static str,
}

pub(crate) struct FunctionDeclaration {
    pub callable: FunctionSpec,
    pub markers: BuiltInCallableMarkers,
}

#[derive(Debug, Clone, Copy)]
#[expect(
    dead_code,
    reason = "the constant macro supports every scalar constant type"
)]
pub(crate) enum ConstantValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ConstantSpec {
    /// The fully qualified name.
    pub name: &'static str,
    pub value: ConstantValue,
}

#[derive(Clone, Copy)]
pub(crate) struct MethodSpec {
    pub name: &'static str,
    pub visibility: Visibility,
    /// Whether the method is `static`; static handlers receive
    /// [`crate::value::Value::null()`] as the receiver.
    pub is_static: bool,
    /// The method's language-level generic binders.
    pub type_parameters: &'static [TypeParameterSpec],
    /// The parameters, in declaration order.
    pub parameters: &'static [ParameterSpec],
    pub return_spec: TypeSpec,
    pub handler: BuiltInHandler,
    pub markers: BuiltInCallableMarkers,
    /// The rendered `fn(...)` type of the method.
    pub signature: &'static str,
}

/// One built-in property declaration: built-in classes declare their slot
/// layout; defaults are assigned by constructors.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PropertySpec {
    /// The property name, without the `$`.
    pub name: &'static str,
    pub visibility: Visibility,
    pub is_readonly: bool,
    /// The runtime-enforced declared type, when present.
    pub type_spec: Option<TypeSpec>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ClassConstantSpec {
    pub name: &'static str,
    pub visibility: Visibility,
    pub type_spec: TypeSpec,
    pub value: ConstantValue,
}

pub(crate) struct ClassSpec {
    /// The fully qualified name.
    pub name: &'static str,
    /// The language-level generic binders.
    pub type_parameters: &'static [TypeParameterSpec],
    /// The parent class, when one is extended.
    pub parent: Option<BaseSpec>,
    pub interfaces: &'static [BaseSpec],
    pub is_final: bool,
    pub is_abstract: bool,
    pub is_readonly: bool,
    /// The permitted direct children, when the class is sealed.
    pub sealed_to: Option<&'static [&'static str]>,
    pub constants: &'static [ClassConstantSpec],
    /// The properties, in slot order.
    pub properties: &'static [PropertySpec],
    pub methods: Box<[MethodSpec]>,
    /// The layout, teardown, and tracing operations for inline built-in state.
    pub built_in_hooks: Option<&'static BuiltInHooks>,
    /// Initializes this built-in class's Rust representation, present exactly
    /// when the built-in hooks describe field-carrying inline state.
    pub built_in_initializer: Option<BuiltInInitializer>,
    /// The attribute target flags when the class may itself be applied as an
    /// attribute, as `#[Whim\Attribute\Attribute(flags)]` would grant a class
    /// compiled from source; `None` for an ordinary class.
    pub attribute_flags: Option<i64>,
}

#[derive(Clone, Copy)]
pub(crate) struct InterfaceMethodSpec {
    pub name: &'static str,
    pub is_static: bool,
    /// The method's language-level generic binders.
    pub type_parameters: &'static [TypeParameterSpec],
    /// The parameters, in declaration order.
    pub parameters: &'static [ParameterSpec],
    pub return_spec: TypeSpec,
    /// The rendered `fn(...)` type of the method.
    pub signature: &'static str,
    /// The default body, when the interface provides one.
    pub default_handler: Option<BuiltInHandler>,
    pub markers: BuiltInCallableMarkers,
}

pub(crate) struct InterfaceSpec {
    /// The fully qualified name.
    pub name: &'static str,
    /// The language-level generic binders.
    pub type_parameters: &'static [TypeParameterSpec],
    pub extends: &'static [BaseSpec],
    /// The permitted direct implementors, when the interface is sealed.
    pub sealed_to: Option<&'static [&'static str]>,
    pub constants: &'static [ClassConstantSpec],
    /// The public instance properties implementations must provide.
    pub properties: &'static [PropertySpec],
    /// The methods, abstract or default.
    pub methods: Box<[InterfaceMethodSpec]>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EnumCaseSpec {
    pub name: &'static str,
    /// The backing value; present exactly when the enum is backed.
    pub value: Option<ConstantValue>,
}

pub(crate) struct EnumSpec {
    /// The fully qualified name.
    pub name: &'static str,
    pub interfaces: &'static [BaseSpec],
    /// How the cases are backed, or `None` for a pure enum.
    pub backing: Option<EnumBacking>,
    /// The cases, in declaration order.
    pub cases: &'static [EnumCaseSpec],
    pub constants: &'static [ClassConstantSpec],
    pub methods: Box<[MethodSpec]>,
}

pub(crate) struct NewtypeSpec {
    /// The fully qualified name.
    pub name: &'static str,
    pub type_parameters: &'static [TypeParameterSpec],
    pub backing: TypeSpec,
}

pub(crate) struct CoreDeclarations {
    pub functions: Box<[FunctionDeclaration]>,
    pub classes: Box<[ClassSpec]>,
    pub interfaces: Box<[InterfaceSpec]>,
    pub enums: Box<[EnumSpec]>,
    pub newtypes: Box<[NewtypeSpec]>,
    pub constants: &'static [ConstantSpec],
}
