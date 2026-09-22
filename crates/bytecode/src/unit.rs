//! Everything the compiler produces for one source file.

use serde::Deserialize;
use serde::Serialize;
use serde_seeded::DeserializeSeeded;
use whim_span::Span;
use whim_value::Value;
use whim_value::atom::Atom;
use whim_value::heap::Heap;

use crate::chunk::Chunk;
use crate::chunk::descriptors::Literal;
use crate::chunk::descriptors::TypeDescriptor;
use crate::instruction::Instruction;

pub const STUB_ATTRIBUTE_NAME: &str = "Whim\\Marker\\Stub";
pub const STUB_ATTRIBUTE: &[u8] = b"Whim\\Marker\\Stub";
pub const CONSISTENT_CONSTRUCTOR_ATTRIBUTE: &[u8] = b"Whim\\Marker\\ConsistentConstructor";
pub const CONSISTENT_GENERICS_ATTRIBUTE: &[u8] = b"Whim\\Marker\\ConsistentGenerics";
pub const TRACK_CALLER_ATTRIBUTE: &[u8] = b"Whim\\Marker\\TrackCaller";
pub const TRACE_BOUNDARY_ATTRIBUTE: &[u8] = b"Whim\\Marker\\TraceBoundary";
pub const NEVER_INLINE_ATTRIBUTE: &[u8] = b"Whim\\Marker\\NeverInline";
pub const ALWAYS_INLINE_ATTRIBUTE: &[u8] = b"Whim\\Marker\\AlwaysInline";
pub const COLD_ATTRIBUTE: &[u8] = b"Whim\\Marker\\Cold";
pub const MUST_USE_ATTRIBUTE: &[u8] = b"Whim\\Marker\\MustUse";
pub const FRAMELESS_ATTRIBUTE: &[u8] = b"Whim\\Marker\\Frameless";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltInCallableAttributes {
    pub track_caller: bool,
    pub trace_boundary: bool,
    pub must_use: bool,
}

impl BuiltInCallableAttributes {
    #[must_use]
    pub fn for_whim_symbol(name: &str) -> Self {
        let track_caller = name == "Whim" || name.starts_with("Whim\\");
        let trace_boundary = track_caller
            && (name.starts_with("Whim\\_Private\\")
                || name.contains("\\_Private\\")
                || name.ends_with("\\_Private")
                || name == "Whim\\OS\\FileDescriptor");

        Self {
            track_caller,
            trace_boundary,
            must_use: false,
        }
    }

    #[must_use]
    pub fn resolve(name: &str, markers: BuiltInCallableMarkers) -> Self {
        let defaults = Self::for_whim_symbol(name);
        Self {
            track_caller: markers.track_caller.unwrap_or(defaults.track_caller),
            trace_boundary: markers.trace_boundary.unwrap_or(defaults.trace_boundary),
            must_use: markers.must_use,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuiltInCallableMarkers {
    pub track_caller: Option<bool>,
    pub trace_boundary: Option<bool>,
    pub must_use: bool,
}

#[must_use]
pub fn has_attribute(attributes: &[CompiledAttribute], name: &[u8]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.class.as_bytes() == name)
}

#[must_use]
pub fn is_never_inline(attributes: &[CompiledAttribute]) -> bool {
    has_attribute(attributes, NEVER_INLINE_ATTRIBUTE)
        || has_attribute(attributes, COLD_ATTRIBUTE)
        || has_attribute(attributes, TRACK_CALLER_ATTRIBUTE)
        || has_attribute(attributes, FRAMELESS_ATTRIBUTE)
}

#[must_use]
pub fn must_use_note(attributes: &[CompiledAttribute]) -> Option<Option<Atom>> {
    let attribute = attributes
        .iter()
        .find(|attribute| attribute.class.as_bytes() == MUST_USE_ATTRIBUTE)?;
    let initializer = attribute
        .named_arguments
        .iter()
        .find(|(name, _)| name.as_bytes() == b"note")
        .map(|(_, initializer)| initializer)
        .or_else(|| attribute.arguments.first());
    Some(match initializer {
        Some(ConstantInitializer::Literal(Literal::String(note))) => Some(note.clone()),
        _ => None,
    })
}

#[must_use]
pub fn is_always_inline(attributes: &[CompiledAttribute]) -> bool {
    has_attribute(attributes, ALWAYS_INLINE_ATTRIBUTE)
}

#[must_use]
pub fn is_external(attributes: &[CompiledAttribute]) -> bool {
    has_attribute(attributes, STUB_ATTRIBUTE)
}

/// Returns the literal produced by a callable whose complete body can execute
/// without an activation record.
#[must_use]
pub fn frameless_literal(function: &CompiledFunction) -> Option<Literal> {
    if !has_attribute(&function.attributes, FRAMELESS_ATTRIBUTE) {
        return None;
    }

    literal_return(function)
}

#[must_use]
pub fn literal_value(literal: &Literal) -> Value {
    match literal {
        Literal::Null => Value::null(),
        Literal::Bool(value) => Value::bool(*value),
        Literal::Int(value) => Value::int(*value),
        Literal::Float(value) => Value::float(*value),
        Literal::String(atom) => Value::string(atom.to_handle()),
    }
}

/// Returns the literal produced by a body consisting only of that literal's
/// load and return.
#[must_use]
pub fn literal_return(function: &CompiledFunction) -> Option<Literal> {
    let code = &function.chunk.code;
    let return_index = code.iter().position(|instruction| {
        matches!(
            instruction,
            Instruction::Return { .. }
                | Instruction::ReturnUnchecked { .. }
                | Instruction::ReturnReferenceUnchecked { .. }
                | Instruction::ReturnPairUnchecked { .. }
                | Instruction::ReturnScalarUnchecked { .. }
                | Instruction::ReturnIntUnchecked { .. }
                | Instruction::ReturnNull
                | Instruction::ReturnNullUnchecked
        )
    })?;

    if code[return_index + 1..].iter().any(|instruction| {
        !matches!(
            instruction,
            Instruction::ReturnNull | Instruction::ReturnNullUnchecked
        )
    }) {
        return None;
    }

    match code[return_index] {
        Instruction::ReturnNull | Instruction::ReturnNullUnchecked if return_index == 0 => {
            Some(Literal::Null)
        }
        Instruction::ReturnIntUnchecked { immediate } if return_index == 0 => {
            Some(Literal::Int(i64::from(immediate.value())))
        }
        Instruction::Return { source }
        | Instruction::ReturnUnchecked { source }
        | Instruction::ReturnReferenceUnchecked { source }
        | Instruction::ReturnScalarUnchecked { source }
            if return_index == 1 =>
        {
            match code[0] {
                Instruction::LoadNull { destination } if destination == source => {
                    Some(Literal::Null)
                }
                Instruction::LoadTrue { destination } if destination == source => {
                    Some(Literal::Bool(true))
                }
                Instruction::LoadFalse { destination } if destination == source => {
                    Some(Literal::Bool(false))
                }
                Instruction::LoadInt {
                    destination,
                    immediate,
                } if destination == source => Some(Literal::Int(i64::from(immediate.value()))),
                Instruction::LoadConstant {
                    destination,
                    constant,
                } if destination == source => function
                    .chunk
                    .constants
                    .get(usize::from(constant.index()))
                    .cloned(),
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Protected,
    Private,
}

impl Visibility {
    #[must_use]
    pub const fn is_at_least(&self, other: Self) -> bool {
        self.rank() >= other.rank()
    }

    /// How open a visibility is; an override may move up this order, never down.
    #[must_use]
    pub const fn rank(&self) -> u8 {
        match self {
            Self::Public => 2,
            Self::Protected => 1,
            Self::Private => 0,
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Protected => "protected",
            Self::Private => "private",
        }
    }
}

/// Where a declared instance property lands in its inherited slot layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotPlacement {
    Inherited(u32),
    Appended,
}

/// Applies the instance property layout rule.
#[must_use]
pub fn slot_placement(
    inherited: Option<(u32, Visibility)>,
    visibility: Visibility,
) -> SlotPlacement {
    match inherited {
        Some((slot, inherited_visibility))
            if visibility != Visibility::Private && inherited_visibility != Visibility::Private =>
        {
            SlotPlacement::Inherited(slot)
        }
        _ => SlotPlacement::Appended,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassLikeKind {
    Class,
    Interface,
    Enum,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledBaseReference {
    pub name: Atom,
    pub type_arguments: Option<Vec<TypeDescriptor>>,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Variance {
    Invariant,
    Covariant,
    Contravariant,
}

impl Variance {
    #[must_use]
    pub const fn nested_polarity(&self, polarity: i8) -> i8 {
        match self {
            Self::Invariant => 0,
            Self::Covariant => polarity,
            Self::Contravariant => -polarity,
        }
    }
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledTypeParameter {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    #[seeded(with(serde_seeded::unseeded))]
    pub variance: Variance,
    pub bounds: Vec<TypeDescriptor>,
    pub default: Option<TypeDescriptor>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub enum ConstantInitializer {
    Literal(Literal),
    Thunk(Box<Chunk>),
}

/// Default values compile into the function chunk's prologue; `has_default`
/// only drives arity checking.
#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledParameter {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    #[seeded(with(serde_seeded::unseeded))]
    pub has_default: bool,
    pub default: Option<ConstantInitializer>,
    pub declared_type: Option<TypeDescriptor>,
    #[seeded(with(serde_seeded::unseeded))]
    pub sensitive: bool,
    pub attributes: Vec<CompiledAttribute>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledFunction {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    pub signature: Atom,
    pub type_parameters: Vec<CompiledTypeParameter>,
    pub parameters: Vec<CompiledParameter>,
    pub return_type: Option<TypeDescriptor>,
    pub attributes: Vec<CompiledAttribute>,
    #[seeded(with(serde_seeded::unseeded))]
    pub captures_this: bool,
    pub capture_names: Vec<Atom>,
    pub capture_types: Vec<Option<TypeDescriptor>>,
    pub chunk: Chunk,
}

impl CompiledFunction {
    #[must_use]
    pub fn incoming_register_count(&self, has_receiver: bool) -> u16 {
        let captures = self
            .capture_names
            .len()
            .saturating_sub(usize::from(self.captures_this));
        let count = usize::from(has_receiver)
            .saturating_add(self.parameters.len())
            .saturating_add(captures);
        u16::try_from(count).unwrap_or(u16::MAX)
    }
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledAttribute {
    pub class: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    pub arguments: Vec<ConstantInitializer>,
    #[seeded(with(crate::decode::pairs))]
    pub named_arguments: Vec<(Atom, ConstantInitializer)>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledClassConstant {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    #[seeded(with(serde_seeded::unseeded))]
    pub visibility: Visibility,
    pub initializer: ConstantInitializer,
    pub declared_type: Option<TypeDescriptor>,
    pub attributes: Vec<CompiledAttribute>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledProperty {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    #[seeded(with(serde_seeded::unseeded))]
    pub visibility: Visibility,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_static: bool,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_readonly: bool,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_promoted: bool,
    pub default: Option<ConstantInitializer>,
    pub declared_type: Option<TypeDescriptor>,
    pub attributes: Vec<CompiledAttribute>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledMethod {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub visibility: Visibility,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_static: bool,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_abstract: bool,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_final: bool,
    pub where_constraints: Vec<CompiledWhereConstraint>,
    pub function: CompiledFunction,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledWhereConstraint {
    pub parameter: Atom,
    pub bound: TypeDescriptor,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
}

/// `int` or `string`, the only two the language allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnumBacking {
    Int,
    String,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledEnumCase {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    pub value: Option<ConstantInitializer>,
    pub attributes: Vec<CompiledAttribute>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledClassLike {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    #[seeded(with(serde_seeded::unseeded))]
    pub kind: ClassLikeKind,
    pub type_parameters: Vec<CompiledTypeParameter>,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_abstract: bool,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_final: bool,
    #[seeded(with(serde_seeded::unseeded))]
    pub is_readonly: bool,
    pub parent: Option<CompiledBaseReference>,
    pub interfaces: Vec<CompiledBaseReference>,
    pub constants: Vec<CompiledClassConstant>,
    pub properties: Vec<CompiledProperty>,
    pub methods: Vec<CompiledMethod>,
    pub cases: Vec<CompiledEnumCase>,
    #[seeded(with(serde_seeded::unseeded))]
    pub enum_backing: Option<EnumBacking>,
    pub attributes: Vec<CompiledAttribute>,
    pub sealed_to: Option<Vec<Atom>>,
}

impl CompiledClassLike {
    #[must_use]
    pub fn clone_with_methods(&self, methods: Vec<CompiledMethod>) -> Self {
        Self {
            name: self.name.clone(),
            span: self.span,
            kind: self.kind,
            type_parameters: self.type_parameters.clone(),
            is_abstract: self.is_abstract,
            is_final: self.is_final,
            is_readonly: self.is_readonly,
            parent: self.parent.clone(),
            interfaces: self.interfaces.clone(),
            constants: self.constants.clone(),
            properties: self.properties.clone(),
            methods,
            cases: self.cases.clone(),
            enum_backing: self.enum_backing,
            attributes: self.attributes.clone(),
            sealed_to: self.sealed_to.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledConstant {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    pub attributes: Vec<CompiledAttribute>,
    pub initializer: ConstantInitializer,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledTypeAlias {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    pub attributes: Vec<CompiledAttribute>,
    pub type_parameters: Vec<CompiledTypeParameter>,
    pub descriptor: TypeDescriptor,
    pub rendered: Atom,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledNewtype {
    pub name: Atom,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    pub attributes: Vec<CompiledAttribute>,
    pub type_parameters: Vec<CompiledTypeParameter>,
    pub backing: TypeDescriptor,
}

/// The declaration facts of an installed built-in function that whole-world
/// optimization may use without treating the built-in body as bytecode.
#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledBuiltInFunction {
    pub name: Atom,
    pub type_parameters: Vec<CompiledTypeParameter>,
    pub parameters: Vec<CompiledParameter>,
    pub return_type: TypeDescriptor,
    #[seeded(with(serde_seeded::unseeded))]
    pub attributes: BuiltInCallableAttributes,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledFile {
    pub path: Option<Atom>,
    #[seeded(with(serde_seeded::unseeded))]
    pub span: Span,
    pub has_top_level_code: bool,
    pub attributes: Vec<CompiledAttribute>,
}

#[derive(Debug, Clone, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct CompiledUnit {
    pub path: Atom,
    pub files: Vec<CompiledFile>,
    pub main: Chunk,
    pub functions: Vec<CompiledFunction>,
    pub classes: Vec<CompiledClassLike>,
    pub constants: Vec<CompiledConstant>,
    pub type_aliases: Vec<CompiledTypeAlias>,
    pub newtypes: Vec<CompiledNewtype>,
}
