//! The complete symbol world owned by an engine.

use hashbrown::HashMap;

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledBuiltInFunction;
use crate::bytecode::unit::CompiledNewtype;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::classes::RuntimeClass;
use crate::core::classes::EnumClasses;
use crate::core::classes::IterateClasses;
use crate::core::classes::WellKnown;
use crate::core::classes::WhimClasses;
use crate::core::classes::validate_required_classes;
use crate::core::declarations;
use crate::engine::builtins::BuiltInCallable;
use crate::engine::declare::ConstantSlot;
use crate::symbols::RuntimeFunction;
use crate::symbols::RuntimeTypeEnvironment;
use crate::symbols::SymbolEntry;
use crate::unwrap_result_invariant;
use crate::value::atom::Atom;
use crate::value::function::BuiltInId;
use crate::value::heap::Heap;
use crate::value::newtype::NewtypeId;
use crate::value::newtype::NewtypeValueDescriptor;
use crate::value::newtype::NewtypeValueId;
use crate::value::object::ClassId;
use crate::value::object::TypeEnvironmentId;

pub(crate) struct RuntimeTables {
    pub(crate) symbols: HashMap<Atom, SymbolEntry>,
    pub(crate) functions: Vec<RuntimeFunction>,
    pub(crate) classes: Vec<RuntimeClass>,
    pub(crate) constants: Vec<ConstantSlot>,
    pub(crate) type_aliases: Vec<CompiledTypeAlias>,
    pub(crate) newtypes: Vec<CompiledNewtype>,
    pub(crate) newtype_values: Vec<NewtypeValueDescriptor>,
    pub(crate) newtype_value_cache: HashMap<NewtypeValueDescriptor, NewtypeValueId>,
    pub(crate) type_environments: Vec<RuntimeTypeEnvironment>,
    pub(crate) type_environment_cache: HashMap<u64, Vec<TypeEnvironmentId>>,
    pub(crate) type_debug_string_cache: HashMap<TypeEnvironmentId, Atom>,
    pub(crate) base_environment_cache:
        HashMap<(ClassId, TypeEnvironmentId, ClassId), Option<TypeEnvironmentId>>,
    pub(crate) nominal_compatibility_cache:
        HashMap<(ClassId, TypeEnvironmentId, TypeEnvironmentId), bool>,
    pub(crate) property_type_cache:
        HashMap<(ClassId, TypeEnvironmentId, u32), Option<TypeDescriptor>>,
    pub(crate) constructor_name: Atom,
    pub(crate) destructor_name: Atom,
    pub(crate) has_destructor_classes: bool,
    pub(crate) static_atom: Atom,
    pub(crate) type_id_atom: Atom,
    pub(crate) next_atom: Atom,
    pub(crate) to_iterator_atom: Atom,
    pub(crate) built_in_functions: Vec<BuiltInCallable>,
    pub(crate) built_in_function_ids: HashMap<(usize, &'static str), BuiltInId>,
    pub(crate) built_in_method_ids: HashMap<(ClassId, Atom), BuiltInId>,
    pub(crate) built_in_function_declarations: Vec<CompiledBuiltInFunction>,
    pub(crate) well_known: WellKnown,
    pub(crate) iterate_classes: IterateClasses,
    pub(crate) enum_classes: EnumClasses,
    pub(crate) whim_classes: WhimClasses,
}

impl RuntimeTables {
    pub(crate) fn new(heap: &Heap) -> Self {
        let core = declarations();
        let well_known = WellKnown::resolve(&core);
        let enum_classes = EnumClasses::resolve(&core);
        let whim_classes = WhimClasses::resolve(&core);
        let iterate_classes = IterateClasses::resolve(&core);
        validate_required_classes(&core);
        let mut tables = Self {
            symbols: HashMap::new(),
            functions: Vec::new(),
            classes: Vec::new(),
            constants: Vec::new(),
            type_aliases: Vec::new(),
            newtypes: Vec::new(),
            newtype_values: Vec::new(),
            newtype_value_cache: HashMap::new(),
            type_environments: vec![RuntimeTypeEnvironment {
                parent: None,
                binding: None,
            }],
            type_environment_cache: HashMap::new(),
            type_debug_string_cache: HashMap::new(),
            base_environment_cache: HashMap::new(),
            nominal_compatibility_cache: HashMap::new(),
            property_type_cache: HashMap::new(),
            constructor_name: heap.intern(b"__construct"),
            destructor_name: heap.intern(b"__destruct"),
            has_destructor_classes: false,
            static_atom: heap.intern(b"static"),
            type_id_atom: heap.intern(b"@type-id"),
            next_atom: heap.intern(b"next"),
            to_iterator_atom: heap.intern(b"toIterator"),
            built_in_functions: Vec::new(),
            built_in_function_ids: HashMap::new(),
            built_in_method_ids: HashMap::new(),
            built_in_function_declarations: Vec::new(),
            well_known,
            iterate_classes,
            enum_classes,
            whim_classes,
        };
        tables.register_core(heap, &core);
        tables
    }

    pub(crate) fn intern_newtype_value(
        &mut self,
        declaration: NewtypeId,
        type_environment: TypeEnvironmentId,
        parent: Option<NewtypeValueId>,
    ) -> NewtypeValueId {
        let descriptor = NewtypeValueDescriptor {
            declaration,
            type_environment,
            parent,
        };
        if let Some(id) = self.newtype_value_cache.get(&descriptor) {
            return *id;
        }

        // SAFETY: the surrounding invariant proves this result is successful.
        let id = NewtypeValueId(unsafe {
            unwrap_result_invariant(
                u32::try_from(self.newtype_values.len()),
                "the runtime cannot contain more than u32::MAX newtype values",
            )
        });
        self.newtype_values.push(descriptor);
        self.newtype_value_cache.insert(descriptor, id);
        id
    }

    #[must_use]
    pub(crate) fn newtype_value(&self, id: NewtypeValueId) -> NewtypeValueDescriptor {
        self.newtype_values[id.0 as usize]
    }
}
