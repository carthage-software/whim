#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "class metadata is shared across the runtime"
)]

use std::cell::RefCell;
use std::rc::Rc;

use hashbrown::HashMap;
use imbl::HashMap as PersistentHashMap;
use imbl::HashSet as PersistentHashSet;
use imbl::Vector as PersistentVector;

use crate::builtin::spec::BuiltInHandler;
use crate::builtin::spec::BuiltInInitializer;
use crate::builtin::spec::ParameterSpec;
use crate::builtin::spec::TypeParameterSpec;
use crate::builtin::spec::TypeSpec;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::chunk::descriptors::check_trivial_descriptor;
use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::Visibility;
use crate::symbols::UnitContext;
use crate::u32_index;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::function::FuncId;
use crate::value::object::BuiltInHooks;
use crate::value::object::ClassId;

#[derive(Clone, Copy)]
pub(crate) enum MethodBodyKind {
    Bytecode(FuncId),
    BuiltIn(BuiltInMethodBody),
}

#[derive(Clone, Copy)]
pub(crate) struct BuiltInMethodBody {
    pub handler: BuiltInHandler,
    pub type_parameters: &'static [TypeParameterSpec],
    pub parameters: &'static [ParameterSpec],
    pub return_spec: TypeSpec,
    pub signature: &'static str,
    pub attributes: BuiltInCallableAttributes,
}

#[derive(Clone, Copy)]
pub(crate) struct MethodEntry {
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    pub declaring_class: ClassId,
    pub body: MethodBodyKind,
}

#[derive(Clone)]
pub(crate) struct ClassConstantEntry {
    pub value: ClassConstantValue,
    pub declared_type: Option<TypeDescriptor>,
    pub visibility: Visibility,
    pub declaring_class: ClassId,
}

#[derive(Clone)]
pub(crate) enum ClassConstantValue {
    Evaluated(Value),
    Evaluating,
    Pending {
        context: Rc<UnitContext>,
        class_position: u32,
        constant_position: u32,
    },
}

#[derive(Clone)]
pub(crate) struct EnumCaseDeclaration {
    pub name: Atom,
    pub backing: Option<Value>,
}

#[derive(Clone)]
pub(crate) enum ClassMemberEntry {
    Method(MethodEntry),
    Constant(ClassConstantEntry),
    EnumCase(u32),
}

impl ClassMemberEntry {
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Method(_) => "method",
            Self::Constant(_) => "constant",
            Self::EnumCase(_) => "case",
        }
    }
}

#[derive(Clone)]
pub(crate) struct PropertyInfo {
    // note: the name doesnot include `$` prefix
    pub name: Atom,
    pub visibility: Visibility,
    pub is_readonly: bool,
    pub declaring_class: ClassId,
    pub default: Option<PropertyDefault>,
    pub declared_type: Option<TypeDescriptor>,
}

#[derive(Clone)]
pub(crate) enum PropertyDefault {
    Value(Value),
    Pending {
        context: Rc<UnitContext>,
        class_position: u32,
        property_position: u32,
    },
}

#[derive(Clone)]
pub(crate) struct RuntimeBase {
    pub class: ClassId,
    pub type_arguments: Option<Vec<TypeDescriptor>>,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "these independent class facts are clearer as named fields"
)]
pub(crate) struct RuntimeClass {
    pub name: Atom,
    pub kind: ClassLikeKind,
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,
    pub consistent_constructor: bool,
    pub consistent_generics: bool,
    pub type_parameter_arity: Option<(u32, u32)>,
    pub type_parameters: Rc<[CompiledTypeParameter]>,
    pub direct_bases: Vec<RuntimeBase>,
    pub base_specializations: HashMap<ClassId, Vec<TypeDescriptor>>,
    pub parent: Option<ClassId>,
    pub interfaces: PersistentVector<ClassId>,
    pub interface_set: PersistentHashSet<ClassId>,
    pub slot_names: PersistentHashMap<Atom, u32>,
    pub private_slots: PersistentHashMap<(ClassId, Atom), u32>,
    pub slots: PersistentVector<PropertyInfo>,
    pub default_slots: PersistentVector<u32>,
    pub initial_slots: PersistentVector<Value>,
    non_acyclic_slots: u32,
    non_simple_slots: u32,
    pub slots_are_acyclic: bool,
    pub simple_instance: bool,
    pub allocates_plainly: bool,
    pub static_names: HashMap<Atom, u32>,
    pub statics: RefCell<Vec<Value>>,
    pub statics_info: Vec<PropertyInfo>,
    pub members: PersistentHashMap<Atom, ClassMemberEntry>,
    pub destructor: Option<MethodEntry>,
    pub private_methods: PersistentHashMap<(ClassId, Atom), MethodEntry>,
    pub enum_cases: Vec<EnumCaseDeclaration>,
    pub case_instances: RefCell<HashMap<Atom, Value>>,
    pub sealed_to: Option<Vec<Atom>>,
    pub built_in_state_hooks: Rc<[&'static BuiltInHooks]>,
    pub built_in_state_initializers: Rc<[BuiltInInitializer]>,
    pub attribute_flags: Option<i64>,
    pub attribute_declarations: Vec<CompiledAttribute>,
    pub attribute_unit: Option<Rc<UnitContext>>,
}

impl RuntimeClass {
    #[must_use]
    pub(crate) fn new(name: Atom, kind: ClassLikeKind) -> Self {
        Self {
            name,
            kind,
            is_readonly: false,
            type_parameter_arity: None,
            type_parameters: Rc::from([]),
            direct_bases: Vec::new(),
            base_specializations: HashMap::new(),
            is_abstract: false,
            is_final: false,
            parent: None,
            consistent_constructor: false,
            consistent_generics: false,
            interfaces: PersistentVector::new(),
            interface_set: PersistentHashSet::new(),
            slot_names: PersistentHashMap::new(),
            private_slots: PersistentHashMap::new(),
            slots: PersistentVector::new(),
            default_slots: PersistentVector::new(),
            initial_slots: PersistentVector::new(),
            non_acyclic_slots: 0,
            non_simple_slots: 0,
            slots_are_acyclic: true,
            simple_instance: false,
            allocates_plainly: false,
            static_names: HashMap::new(),
            statics: RefCell::new(Vec::new()),
            statics_info: Vec::new(),
            members: PersistentHashMap::new(),
            destructor: None,
            private_methods: PersistentHashMap::new(),
            enum_cases: Vec::new(),
            case_instances: RefCell::new(HashMap::new()),
            sealed_to: None,
            built_in_state_hooks: Rc::from([]),
            built_in_state_initializers: Rc::from([]),
            attribute_flags: None,
            attribute_declarations: Vec::new(),
            attribute_unit: None,
        }
    }

    #[inline]
    pub(crate) fn insert_interface(&mut self, interface: ClassId) -> bool {
        if self.interface_set.insert(interface).is_some() {
            return false;
        }

        self.interfaces.push_back(interface);
        true
    }

    #[inline]
    pub(crate) fn inherit_interfaces(&mut self, parent: &Self) {
        self.interfaces.clone_from(&parent.interfaces);
        self.interface_set.clone_from(&parent.interface_set);
    }

    pub(crate) fn inherit_properties(&mut self, parent: &Self) {
        self.slot_names.clone_from(&parent.slot_names);
        self.private_slots.clone_from(&parent.private_slots);
        self.slots.clone_from(&parent.slots);
        self.default_slots.clone_from(&parent.default_slots);
        self.initial_slots.clone_from(&parent.initial_slots);
        self.non_acyclic_slots = parent.non_acyclic_slots;
        self.non_simple_slots = parent.non_simple_slots;
    }

    pub(crate) fn append_property(&mut self, property: PropertyInfo) -> u32 {
        let slot = u32_index(self.slots.len());
        if property.default.is_some() {
            self.default_slots.push_back(slot);
        }
        self.initial_slots
            .push_back(property_initial_value(&property));
        self.non_acyclic_slots += u32::from(!property_is_acyclic(&property));
        self.non_simple_slots += u32::from(!property_is_simple(&property));
        self.slots.push_back(property);
        slot
    }

    pub(crate) fn replace_property(&mut self, slot: u32, property: PropertyInfo) {
        let index = slot as usize;
        let previous = &self.slots[index];
        self.non_acyclic_slots -= u32::from(!property_is_acyclic(previous));
        self.non_simple_slots -= u32::from(!property_is_simple(previous));

        match (previous.default.is_some(), property.default.is_some()) {
            (false, true) => {
                let position = self
                    .default_slots
                    .iter()
                    .position(|candidate| *candidate > slot)
                    .unwrap_or(self.default_slots.len());
                self.default_slots.insert(position, slot);
            }
            (true, false) => {
                let position = self
                    .default_slots
                    .iter()
                    .position(|candidate| *candidate == slot)
                    .expect("default slot metadata contains every default");
                self.default_slots.remove(position);
            }
            (false, false) | (true, true) => {}
        }

        self.initial_slots
            .set(index, property_initial_value(&property));
        self.non_acyclic_slots += u32::from(!property_is_acyclic(&property));
        self.non_simple_slots += u32::from(!property_is_simple(&property));
        self.slots.set(index, property);
    }

    pub(crate) fn finalize_layout(&mut self, destructor_name: &Atom) {
        self.destructor = self
            .method(destructor_name)
            .filter(|entry| !entry.is_abstract);

        self.slots_are_acyclic = self.non_acyclic_slots == 0;

        self.allocates_plainly = !self.is_abstract
            && self.kind == ClassLikeKind::Class
            && self.destructor.is_none()
            && self.built_in_state_initializers.is_empty()
            && self.default_slots.is_empty();
        self.simple_instance = !self.is_abstract
            && self.kind == ClassLikeKind::Class
            && self.type_parameters.is_empty()
            && self.built_in_state_initializers.is_empty()
            && self.non_simple_slots == 0;
    }

    #[inline]
    pub(crate) fn method(&self, name: &Atom) -> Option<MethodEntry> {
        match self.members.get(name) {
            Some(ClassMemberEntry::Method(entry)) => Some(*entry),
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn constant(&self, name: &Atom) -> Option<&ClassConstantEntry> {
        match self.members.get(name) {
            Some(ClassMemberEntry::Constant(entry)) => Some(entry),
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn constant_mut(&mut self, name: &Atom) -> Option<&mut ClassConstantEntry> {
        match self.members.get_mut(name) {
            Some(ClassMemberEntry::Constant(entry)) => Some(entry),
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn enum_case(&self, name: &Atom) -> Option<&EnumCaseDeclaration> {
        let Some(ClassMemberEntry::EnumCase(position)) = self.members.get(name) else {
            return None;
        };
        self.enum_cases.get(*position as usize)
    }

    pub(crate) fn methods(&self) -> impl Iterator<Item = (&Atom, MethodEntry)> {
        self.members
            .iter()
            .filter_map(|(name, member)| match member {
                ClassMemberEntry::Method(entry) => Some((name, *entry)),
                _ => None,
            })
    }

    pub(crate) fn constants(&self) -> impl Iterator<Item = (&Atom, &ClassConstantEntry)> {
        self.members
            .iter()
            .filter_map(|(name, member)| match member {
                ClassMemberEntry::Constant(entry) => Some((name, entry)),
                _ => None,
            })
    }
}

fn property_initial_value(property: &PropertyInfo) -> Value {
    match property.default.as_ref() {
        Some(PropertyDefault::Value(value)) => value.clone(),
        Some(PropertyDefault::Pending { .. }) | None => Value::uninitialized(),
    }
}

fn property_is_acyclic(property: &PropertyInfo) -> bool {
    property
        .declared_type
        .as_ref()
        .is_some_and(descriptor_is_acyclic)
}

fn property_is_simple(property: &PropertyInfo) -> bool {
    let Some(default) = property.default.as_ref() else {
        return true;
    };

    let PropertyDefault::Value(default) = default else {
        return false;
    };

    property
        .declared_type
        .as_ref()
        .is_none_or(|descriptor| check_trivial_descriptor(descriptor, default) == Some(true))
}

fn descriptor_is_acyclic(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::StringLength { .. }
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Classname(_) => true,
        TypeDescriptor::Union(members) => members.iter().all(descriptor_is_acyclic),
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Object
        | TypeDescriptor::Named { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::StaticClass
        | TypeDescriptor::Array(_)
        | TypeDescriptor::Vector(_)
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::Dictionary(_)
        | TypeDescriptor::DictionaryShape { .. }
        | TypeDescriptor::Callable(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::TupleAny
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Intersection(_)
        | TypeDescriptor::Negated(_) => false,
    }
}

#[must_use]
pub(crate) fn is_instance_of(classes: &[RuntimeClass], class: ClassId, target: ClassId) -> bool {
    if classes[class.0 as usize].interface_set.contains(&target) {
        return true;
    }

    let mut current = Some(class);
    while let Some(id) = current {
        if id == target {
            return true;
        }

        let entry = &classes[id.0 as usize];
        current = entry.parent;
    }

    false
}

#[must_use]
pub(crate) fn extends_or_is(classes: &[RuntimeClass], class: ClassId, ancestor: ClassId) -> bool {
    let mut current = Some(class);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }

        current = classes[id.0 as usize].parent;
    }

    false
}

/// Whether a member is accessible from `scope`.
#[must_use]
pub(crate) fn visibility_allows(
    classes: &[RuntimeClass],
    visibility: Visibility,
    declaring_class: ClassId,
    scope: Option<ClassId>,
) -> bool {
    match visibility {
        Visibility::Public => true,
        Visibility::Private => scope == Some(declaring_class),
        Visibility::Protected => scope.is_some_and(|scope| {
            extends_or_is(classes, scope, declaring_class)
                || extends_or_is(classes, declaring_class, scope)
        }),
    }
}
