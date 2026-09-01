//! Looking up symbols: functions, constants, classes, and enum cases.

use std::mem;

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::literal_value;
use crate::core::symbols::CHECKED_KIND_ORDER;
use crate::core::symbols::CLASS_LIKE_KIND_ORDER;
use crate::engine::declare::ConstantSlot;
use crate::value::newtype::NewtypeId;
use crate::vm::Atom;
use crate::vm::CacheEntry;
use crate::vm::Chunk;
use crate::vm::ClassId;
use crate::vm::FuncId;
use crate::vm::FunctionTable;
use crate::vm::IcDescriptor;
use crate::vm::InstanceObject;
use crate::vm::NonNull;
use crate::vm::SymbolEntry;
use crate::vm::SymbolKind;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::unreachable_invariant;

impl VirtualMachine<'_> {
    /// The declared kind of a name without autoloading.
    pub(crate) fn symbol_kind_of(&self, name: Atom) -> Option<SymbolKind> {
        self.engine
            .tables
            .symbols
            .get(&name)
            .map(|entry| entry.kind)
    }

    pub(crate) fn enum_case_value(&self, class: ClassId, name: Atom) -> Option<Value> {
        self.enum_case_instance(class, name)
    }

    /// Reads a `ConstantGet` site: resolves the name to its store index
    /// through the inline cache, then forces the lazy slot. The index is
    /// cached only once the constant has evaluated, so a miss or a cycle is
    /// never cached.
    pub(in crate::vm) fn constant_value(
        &mut self,
        slot: usize,
        chunk: &Chunk,
    ) -> Result<Value, VirtualMachineControl> {
        let name = match &chunk.ic_descriptors[slot] {
            IcDescriptor::Member { name, .. } => name.clone(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            IcDescriptor::ClassMember { .. } => unsafe {
                unreachable_invariant("a ConstantGet site resolves a member descriptor")
            },
        };

        let cache_pointer = self.current_frame().cache;
        {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let cache_cell = unsafe { cache_pointer.as_ref() };
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let cache = unsafe { &mut *cache_cell.entries() };
            if cache.is_empty() {
                cache.resize(chunk.ic_descriptors.len(), CacheEntry::Empty);
            }

            if let CacheEntry::Constant(index) = cache[slot] {
                return self.force_constant(index, name);
            }
        }

        let index = self.resolve_constant_index(name.clone())?;
        let value = self.force_constant(index, name)?;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };
        cache[slot] = CacheEntry::Constant(index);
        Ok(value)
    }

    /// Resolves a constant name to its store index, running the autoload
    /// chain on a miss before failing.
    fn resolve_constant_index(&mut self, name: Atom) -> Result<u32, VirtualMachineControl> {
        let mut resolved = match self.engine.tables.symbols.get(&name) {
            Some(entry) if entry.kind == SymbolKind::Constant => Some(entry.index),
            _ => None,
        };

        if resolved.is_none() {
            self.run_autoload_chain(SymbolKind::Constant, name.clone())?;
            resolved = match self.engine.tables.symbols.get(&name) {
                Some(entry) if entry.kind == SymbolKind::Constant => Some(entry.index),
                _ => None,
            };
        }

        resolved.ok_or_else(|| {
            self.throw_well_known(
                self.engine.tables.well_known.undefined_symbol_error,
                format!("the constant {} is not defined", name.to_string_lossy()),
            )
        })
    }

    /// Forces a top-level constant slot: an already-evaluated slot clones its
    /// value, a pending slot runs its initializer once and caches the result,
    /// and a slot already being forced is a self-referential cycle. On an
    /// initializer failure the slot returns to pending, so a caught failure
    /// can be retried rather than misreported as a cycle.
    pub(crate) fn force_constant(
        &mut self,
        index: u32,
        name: Atom,
    ) -> Result<Value, VirtualMachineControl> {
        match &self.engine.tables.constants[index as usize] {
            ConstantSlot::Evaluated(value) => return Ok(value.clone()),
            ConstantSlot::Evaluating => {
                let error = if self.engine.declaration_depth == 0 {
                    self.engine.tables.well_known.error
                } else {
                    self.engine.tables.well_known.linker_error
                };

                return Err(self.throw_well_known(
                    error,
                    format!("the constant {} refers to itself", name.to_string_lossy()),
                ));
            }
            ConstantSlot::Pending { .. } => {}
        }

        let ConstantSlot::Pending { context, position } = mem::replace(
            &mut self.engine.tables.constants[index as usize],
            ConstantSlot::Evaluating,
        ) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the slot was pending") }
        };

        let outcome = match &context.unit.constants[position as usize].initializer {
            ConstantInitializer::Literal(literal) => Ok(literal_value(literal)),
            ConstantInitializer::Thunk(chunk) => {
                let chunk_pointer = NonNull::from(&**chunk);
                self.run_initializer(chunk_pointer, &context)
            }
        };

        match outcome {
            Ok(value) => {
                self.engine.tables.constants[index as usize] =
                    ConstantSlot::Evaluated(value.clone());
                Ok(value)
            }
            Err(control) => {
                self.engine.tables.constants[index as usize] =
                    ConstantSlot::Pending { context, position };
                Err(control)
            }
        }
    }

    /// Resolves a function name to a user, engine, or built-in function; a
    /// miss runs the autoload chain before failing.
    pub(in crate::vm) fn resolve_function(
        &mut self,
        name: Atom,
    ) -> Result<CacheEntry, VirtualMachineControl> {
        if let Some(entry) = self.lookup_function(&name) {
            return Ok(entry);
        }

        self.run_autoload_chain(SymbolKind::Function, name.clone())?;
        match self.lookup_function(&name) {
            Some(entry) => Ok(entry),
            None => {
                let text = name.to_string_lossy().into_owned();
                Err(self.throw_well_known(
                    self.engine.tables.well_known.undefined_symbol_error,
                    format!("the function {text} is not defined"),
                ))
            }
        }
    }

    /// Resolves a call-position name to a function or newtype constructor.
    pub(in crate::vm) fn resolve_named_callable(
        &mut self,
        name: Atom,
    ) -> Result<CacheEntry, VirtualMachineControl> {
        if let Some(entry) = self.lookup_named_callable(&name) {
            return Ok(entry);
        }

        self.run_autoload_chain(SymbolKind::Function, name.clone())?;
        if let Some(entry) = self.lookup_named_callable(&name) {
            return Ok(entry);
        }
        self.run_autoload_chain(SymbolKind::Newtype, name.clone())?;
        if let Some(entry) = self.lookup_named_callable(&name) {
            return Ok(entry);
        }

        let text = name.to_string_lossy().into_owned();
        Err(self.throw_well_known(
            self.engine.tables.well_known.undefined_symbol_error,
            format!("the function or newtype {text} is not defined"),
        ))
    }

    fn lookup_named_callable(&self, name: &Atom) -> Option<CacheEntry> {
        let entry = self.engine.tables.symbols.get(name)?;
        if entry.kind == SymbolKind::Newtype {
            return Some(CacheEntry::Newtype(NewtypeId(entry.index)));
        }

        if entry.kind != SymbolKind::Function {
            return None;
        }

        Some(match entry.table {
            FunctionTable::User => CacheEntry::Function(FuncId(entry.index)),
            FunctionTable::BuiltIn => CacheEntry::BuiltInCallable(entry.index),
        })
    }

    /// One declared-function lookup, without autoloading.
    fn lookup_function(&self, name: &Atom) -> Option<CacheEntry> {
        let entry = self.engine.tables.symbols.get(name)?;
        if entry.kind != SymbolKind::Function {
            return None;
        }

        Some(match entry.table {
            FunctionTable::User => CacheEntry::Function(FuncId(entry.index)),
            FunctionTable::BuiltIn => CacheEntry::BuiltInCallable(entry.index),
        })
    }

    /// Materializes an enum case singleton on first access and caches it in
    /// the class store, so every access yields the identical instance.
    pub(in crate::vm) fn enum_case_instance(&self, class: ClassId, name: Atom) -> Option<Value> {
        let entry = &self.engine.tables.classes[class.0 as usize];
        if let Some(instance) = entry.case_instances.borrow().get(&name) {
            return Some(instance.clone());
        }

        let declaration = entry.enum_case(&name)?.clone();

        let slot_count = entry.slots.len();
        let name_slot = entry.slot_names.get(&self.heap.intern(b"name")).copied();
        let value_slot = entry.slot_names.get(&self.heap.intern(b"value")).copied();
        let instance = InstanceObject::new(&self.heap, class, slot_count);
        if let Some(slot) = name_slot {
            drop(instance.write_slot(slot as usize, Value::string(name.to_handle())));
        }

        if let (Some(slot), Some(backing)) = (value_slot, declaration.backing) {
            drop(instance.write_slot(slot as usize, backing));
        }

        let value = Value::object(instance);
        self.engine.tables.classes[class.0 as usize]
            .case_instances
            .borrow_mut()
            .insert(name, value.clone());

        Some(value)
    }

    /// Resolves a class reference atom: `static` binds to the frame's called
    /// class, anything else resolves through the symbol table, with the
    /// autoload chain consulted on a miss.
    pub(in crate::vm) fn resolve_class_reference(
        &mut self,
        class_atom: Atom,
    ) -> Result<ClassId, VirtualMachineControl> {
        if let Some(binder) = class_atom.as_bytes().strip_prefix(b"@") {
            let name = self.heap.intern(binder);
            return self.resolve_type_parameter_class(&name);
        }

        if class_atom == self.engine.tables.static_atom {
            return match self.current_frame().called_class.get() {
                Some(class) => Ok(class),
                None => Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    "`static` is not bound outside a class context".to_string(),
                )),
            };
        }

        match self.lookup_class_autoloading(class_atom.clone())? {
            Some(class) => Ok(class),
            None => Err(self.throw_well_known(
                self.engine.tables.well_known.undefined_symbol_error,
                format!("the class {} is not defined", class_atom.to_string_lossy()),
            )),
        }
    }

    fn resolve_type_parameter_class(
        &mut self,
        name: &Atom,
    ) -> Result<ClassId, VirtualMachineControl> {
        let environment = self.current_frame().type_environment;
        let mut binder = name.clone();
        let mut hops = 0;
        let descriptor = loop {
            match self.type_environment_binding(environment, &binder).cloned() {
                Some(TypeDescriptor::Parameter(next)) if hops < 32 => {
                    binder = next;
                    hops += 1;
                }
                other => break other,
            }
        };

        let parameter = name.to_string_lossy().into_owned();
        match descriptor {
            Some(TypeDescriptor::Named {
                name: class_name, ..
            }) => match self.lookup_class_autoloading(class_name.clone())? {
                Some(class) => Ok(class),
                None => Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "the type parameter {parameter} is bound to {}, which is not a class",
                        class_name.to_string_lossy()
                    ),
                )),
            },
            Some(TypeDescriptor::StaticClass) => match self.current_frame().called_class.get() {
                Some(class) => Ok(class),
                None => Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    "`static` is not bound outside a class context".to_string(),
                )),
            },
            Some(_) => Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("the type parameter {parameter} is not bound to a class in this call"),
            )),
            None => Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("the type parameter {parameter} is not bound in this call"),
            )),
        }
    }

    /// Resolves and caches an exact named class used by an instantiation site.
    /// Late-static references remain dynamic because their called class is a
    /// property of the active frame rather than the bytecode site.
    pub(in crate::vm) fn resolve_class_site(
        &mut self,
        slot: usize,
        chunk: &Chunk,
    ) -> Result<ClassId, VirtualMachineControl> {
        let IcDescriptor::Member { name, .. } = &chunk.ic_descriptors[slot] else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a NewStatic site resolves a member descriptor") }
        };

        if *name == self.engine.tables.static_atom {
            return self.resolve_class_reference(name.clone());
        }

        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };
        if cache.is_empty() {
            cache.resize(chunk.ic_descriptors.len(), CacheEntry::Empty);
        }

        if let CacheEntry::Class(class) = cache[slot] {
            return Ok(class);
        }

        let class = self.resolve_class_reference(name.clone())?;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };
        cache[slot] = CacheEntry::Class(class);
        Ok(class)
    }

    /// Resolves a class-like name, running the autoload chain on a miss.
    pub(in crate::vm) fn lookup_class_autoloading(
        &mut self,
        name: Atom,
    ) -> Result<Option<ClassId>, VirtualMachineControl> {
        if let Some(class) = self.resolve_class_symbol(&name) {
            return Ok(Some(class));
        }

        for kind in CLASS_LIKE_KIND_ORDER {
            self.run_autoload_chain(kind, name.clone())?;
            if let Some(class) = self.resolve_class_symbol(&name) {
                return Ok(Some(class));
            }
        }

        Ok(None)
    }

    /// Resolves a name reaching a checked type position. A declared symbol
    /// answers directly; a miss probes every symbol kind. Absence is never an error here: an
    /// unresolved name is satisfied by no value. Resolutions are not cached,
    /// so a later declaration or a successful load is always picked up.
    pub(in crate::vm) fn resolve_checked_name(
        &mut self,
        name: Atom,
    ) -> Result<Option<SymbolEntry>, VirtualMachineControl> {
        if let Some(entry) = self.engine.tables.symbols.get(&name) {
            return Ok(Some(*entry));
        }

        if self.engine.autoloader.is_none() {
            return Ok(None);
        }

        for kind in CHECKED_KIND_ORDER {
            self.run_autoload_chain(kind, name.clone())?;
            if let Some(entry) = self.engine.tables.symbols.get(&name) {
                return Ok(Some(*entry));
            }
        }

        Ok(None)
    }

    /// Resolves a class-like name against the symbol table, without
    /// autoloading; [`lookup_class_autoloading`](Self::lookup_class_autoloading)
    /// wraps it with the autoloader chain on a miss.
    pub(crate) fn resolve_class_symbol(&self, name: &Atom) -> Option<ClassId> {
        let entry = self.engine.tables.symbols.get(name)?;
        match entry.kind {
            SymbolKind::Class | SymbolKind::Enum | SymbolKind::Interface => {
                Some(ClassId(entry.index))
            }
            _ => None,
        }
    }
}
