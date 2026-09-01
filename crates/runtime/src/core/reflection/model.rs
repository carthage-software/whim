//! Stable identities carried by native reflection objects.

use std::rc::Rc;

use whim_span::Span;

use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledAttribute;
use crate::symbols::UnitContext;
use crate::value::atom::Atom;
use crate::value::function::FuncId;
use crate::value::newtype::NewtypeValueId;
use crate::value::object::ClassId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CallableKey {
    Function(Atom),
    Method { class: ClassId, name: Atom },
    Closure(FuncId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MemberKind {
    Method,
    Property,
    ClassConstant,
    EnumCase,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MemberKey {
    pub(crate) class: ClassId,
    pub(crate) name: Atom,
    pub(crate) kind: MemberKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum DeclarationKey {
    Symbol(Atom),
    Member(MemberKey),
    Parameter {
        callable: CallableKey,
        position: usize,
    },
    Closure(FuncId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum GenericOwner {
    Symbol(Atom),
    Callable(CallableKey),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TypeParameterKey {
    pub(crate) owner: GenericOwner,
    pub(crate) position: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ReflectedType {
    pub(crate) descriptor: TypeDescriptor,
    pub(crate) owner: Option<GenericOwner>,
    pub(crate) declaring_class: Option<ClassId>,
}

impl ReflectedType {
    pub(crate) const fn new(descriptor: TypeDescriptor) -> Self {
        Self {
            descriptor,
            owner: None,
            declaring_class: None,
        }
    }

    pub(crate) const fn owned(descriptor: TypeDescriptor, owner: GenericOwner) -> Self {
        Self {
            descriptor,
            owner: Some(owner),
            declaring_class: None,
        }
    }

    pub(crate) const fn in_class(mut self, class: ClassId) -> Self {
        self.declaring_class = Some(class);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SourceLocationData {
    pub(crate) file: Atom,
    pub(crate) start_offset: u32,
    pub(crate) end_offset: u32,
    pub(crate) start_line: u32,
    pub(crate) start_column: u32,
    pub(crate) end_line: u32,
    pub(crate) end_column: u32,
}

#[derive(Clone)]
pub(crate) enum ReflectionData {
    SourceLocation(SourceLocationData),
    Symbol(Atom),
    Member(MemberKey),
    Parameter {
        callable: CallableKey,
        position: usize,
    },
    Closure(FuncId),
    Capture {
        function: FuncId,
        position: usize,
    },
    CallableValue,
    CaptureValue {
        function: FuncId,
        position: usize,
    },
    BoundArgument {
        callable: CallableKey,
        parameter: usize,
    },
    TypeParameter(TypeParameterKey),
    TypeBinding {
        parameter: TypeParameterKey,
        argument: ReflectedType,
    },
    TypeEnvironment(Vec<(TypeParameterKey, ReflectedType)>),
    Attribute {
        target: DeclarationKey,
        declaration: CompiledAttribute,
        unit: Option<Rc<UnitContext>>,
    },
    AttributeDefinition(ClassId),
    Object,
    PropertyValue {
        property: MemberKey,
        slot: usize,
    },
    NewtypeValue(NewtypeValueId),
    Type(ReflectedType),
    FunctionTypeParameter {
        position: usize,
        r#type: ReflectedType,
        optional: bool,
    },
    DictShapeEntry {
        key: ShapeKey,
        r#type: ReflectedType,
    },
}

#[derive(Clone)]
pub(crate) struct DeclarationMetadata {
    pub(crate) unit: Option<Rc<UnitContext>>,
    pub(crate) span: Option<Span>,
    pub(crate) attributes: Vec<CompiledAttribute>,
}
