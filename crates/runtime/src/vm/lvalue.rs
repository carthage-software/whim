//! Resolving a property, static, or class-constant slot to read or write.

use std::io;
use std::mem;

use crate::bytecode::chunk::descriptors::check_trivial_descriptor;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::Visibility;
use crate::bytecode::unit::literal_value;
use crate::classes::ClassConstantValue;
use crate::classes::extends_or_is;
use crate::core::private::syscall::StandardStream;
use crate::engine::Engine;
use crate::vm::ArgumentGuard;
use crate::vm::Atom;
use crate::vm::CacheEntry;
use crate::vm::CachedPropertyGuard;
use crate::vm::CachedPropertySlot;
use crate::vm::Chunk;
use crate::vm::ClassId;
use crate::vm::InlineCache;
use crate::vm::InstanceObject;
use crate::vm::ManagedRef;
use crate::vm::NonNull;
use crate::vm::PropertyGuardWays;
use crate::vm::Rc;
use crate::vm::TypeDescriptor;
use crate::vm::TypeEnvironmentId;
use crate::vm::UnitContext;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::call::argument_guard;
use crate::vm::call::guard_allows;
use crate::vm::class_member_atoms;
use crate::vm::name_atom;
use crate::vm::ops;
use crate::vm::unreachable_invariant;
use crate::vm::visibility_allows;
use crate::vm::visibility_name;

impl VirtualMachine<'_> {
    /// Stringifies a register window and writes it to a process stream in one
    /// unbuffered write.
    pub(in crate::vm) fn write_values(
        &mut self,
        start: usize,
        count: usize,
        error_stream: bool,
        line: bool,
    ) -> Result<(), VirtualMachineControl> {
        let mut bytes = Vec::new();
        for position in 0..count {
            let (rendered, kind) = {
                let value = &self.stack[start + position];
                (
                    ops::stringify_for_concat(&self.heap, value),
                    value.kind_name(),
                )
            };

            let Some(rendered) = rendered else {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "a {kind} value cannot be written; write accepts string, int, and float"
                    ),
                ));
            };

            bytes.extend_from_slice(rendered.flatten());
        }

        if line {
            bytes.push(b'\n');
        }

        let stream = if error_stream {
            StandardStream::Error
        } else {
            StandardStream::Output
        };
        let result = Engine::write_standard_stream(stream, &bytes);
        self.observe_write_result(result)?;

        Ok(())
    }

    fn observe_write_result(
        &mut self,
        result: io::Result<()>,
    ) -> Result<(), VirtualMachineControl> {
        let Err(error) = result else {
            return Ok(());
        };
        let broken_pipe = error.kind() == io::ErrorKind::BrokenPipe;
        self.engine.note_output_failure(Err(error));
        if broken_pipe {
            Err(VirtualMachineControl::Exit(0))
        } else {
            Ok(())
        }
    }

    /// Resolves a property site to its slot index through the inline cache.
    #[inline(always)]
    pub(in crate::vm) fn cached_property_slot(
        &self,
        site: usize,
        receiver_class: ClassId,
    ) -> Option<u32> {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &*cache_cell.property_slots() };
        if site >= cache.len() {
            return None;
        }

        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { cache.get_unchecked(site) }.get(receiver_class)
    }

    #[inline(always)]
    pub(in crate::vm) fn property_slot_for(
        &mut self,
        site: usize,
        chunk: &Chunk,
        receiver_class: ClassId,
    ) -> Result<u32, VirtualMachineControl> {
        if let Some(slot) = self.cached_property_slot(site, receiver_class) {
            return Ok(slot);
        }

        self.resolve_property_slot(site, chunk, receiver_class)
    }

    /// Resolves the slot a raw property initialization writes, caching it by
    /// receiver class at the site.
    #[inline(always)]
    pub(in crate::vm) fn raw_property_slot(
        &mut self,
        site: usize,
        chunk: &Chunk,
        receiver_class: ClassId,
    ) -> Result<u32, VirtualMachineControl> {
        if let Some(slot) = self.cached_property_slot(site, receiver_class) {
            return Ok(slot);
        }

        self.resolve_raw_property_slot(site, chunk, receiver_class)
    }

    /// Resolves and fills a raw property cache miss.
    #[cold]
    #[inline(never)]
    fn resolve_raw_property_slot(
        &mut self,
        site: usize,
        chunk: &Chunk,
        receiver_class: ClassId,
    ) -> Result<u32, VirtualMachineControl> {
        let name = name_atom(chunk, site);
        let class = &self.engine.tables.classes[receiver_class.0 as usize];
        let scope = self.current_frame().class_scope.get();
        let Some(slot) = scope
            .and_then(|scope| class.private_slots.get(&(scope, name.clone())).copied())
            .or_else(|| class.slot_names.get(name).copied())
        else {
            let class_name = class.name.to_string();
            let member = name.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("the property {class_name}::{member} is not declared"),
            ));
        };

        let info = &class.slots[slot as usize];
        if !visibility_allows(
            &self.engine.tables.classes,
            info.visibility,
            info.declaring_class,
            scope,
        ) {
            let class_name = class.name.to_string();
            let member = info.name.to_string();
            let visibility = visibility_name(info.visibility);
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot access {visibility} property {class_name}::{member}"),
            ));
        }

        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_pointer.as_ref().property_slots() };
        if cache.is_empty() {
            cache.resize(chunk.ic_descriptors.len(), CachedPropertySlot::EMPTY);
        }

        cache[site] = CachedPropertySlot::new(receiver_class, slot);
        Ok(slot)
    }

    /// Resolves and fills a property cache miss.
    #[cold]
    #[inline(never)]
    fn resolve_property_slot(
        &mut self,
        site: usize,
        chunk: &Chunk,
        receiver_class: ClassId,
    ) -> Result<u32, VirtualMachineControl> {
        let name = name_atom(chunk, site);
        let class = &self.engine.tables.classes[receiver_class.0 as usize];
        let scope = self.current_frame().class_scope.get();
        let Some(slot) = scope
            .and_then(|scope| class.private_slots.get(&(scope, name.clone())).copied())
            .or_else(|| class.slot_names.get(name).copied())
        else {
            let class_name = class.name.to_string();
            let member = name.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("the property {class_name}::{member} is not declared"),
            ));
        };

        let info = &class.slots[slot as usize];
        if !visibility_allows(
            &self.engine.tables.classes,
            info.visibility,
            info.declaring_class,
            self.current_frame().class_scope.get(),
        ) {
            let info = &self.engine.tables.classes[receiver_class.0 as usize].slots[slot as usize];
            let class_name = String::from_utf8_lossy(
                self.engine.tables.classes[receiver_class.0 as usize]
                    .name
                    .as_bytes(),
            )
            .into_owned();
            let member = info.name.to_string();
            let rendered = visibility_name(info.visibility);
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot access {rendered} property {class_name}::{member}"),
            ));
        }

        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.property_slots() };
        if cache.is_empty() {
            cache.resize(chunk.ic_descriptors.len(), CachedPropertySlot::EMPTY);
        }

        cache[site] = CachedPropertySlot::new(receiver_class, slot);
        Ok(slot)
    }

    /// Enforces the readonly write-once rule: a readonly slot is writable
    /// only while unwritten and from a constructor that can access it.
    pub(in crate::vm) fn check_readonly_write(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        chunk: &Chunk,
        site: usize,
    ) -> Result<(), VirtualMachineControl> {
        let receiver_class = receiver.class();
        let info = &self.engine.tables.classes[receiver_class.0 as usize].slots[slot as usize];
        if !info.is_readonly {
            return Ok(());
        }

        let frame = self.current_frame();
        let allowed = receiver.slot_is_uninitialized(slot as usize) && frame.in_constructor();

        if allowed {
            return Ok(());
        }

        let name = name_atom(chunk, site);
        let message = if receiver.slot_is_uninitialized(slot as usize) {
            format!("cannot write readonly property {name} outside its constructor")
        } else {
            format!("cannot write readonly property {name} twice")
        };

        Err(self.throw_well_known(self.engine.tables.well_known.readonly_error, message))
    }

    /// Enforces the two intentional raw-write cases.
    pub(in crate::vm) fn check_raw_property_write(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
    ) -> Result<(), VirtualMachineControl> {
        let class = receiver.class();
        let (is_readonly, visibility, declaring) = {
            let runtime_class = &self.engine.tables.classes[class.0 as usize];
            let property = &runtime_class.slots[slot as usize];
            (
                property.is_readonly,
                property.visibility,
                property.declaring_class,
            )
        };

        if !is_readonly {
            return Ok(());
        }

        let frame = self.current_frame();
        let constructor_initialization = receiver.slot_is_uninitialized(slot as usize)
            && frame.in_constructor()
            && self
                .current_this()
                .is_some_and(|this| this.ptr_eq(receiver));

        if constructor_initialization {
            return Ok(());
        }

        let qualified = {
            let declaring_class = &self.engine.tables.classes[declaring.0 as usize];
            let property_name =
                &self.engine.tables.classes[class.0 as usize].slots[slot as usize].name;
            format!("{}::${property_name}", declaring_class.name)
        };

        if !self.engine.tables.classes[class.0 as usize].is_readonly {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.readonly_error,
                format!(
                    "cannot override readonly property {qualified} with clone!; readonly clone \
                     overrides are only available on readonly classes"
                ),
            ));
        }

        if visibility == Visibility::Private {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.readonly_error,
                format!(
                    "cannot override private readonly property {qualified} with clone!; only \
                     public and protected readonly properties may be overridden"
                ),
            ));
        }

        let scope = frame.class_scope.get();
        if !scope.is_some_and(|scope| extends_or_is(&self.engine.tables.classes, scope, declaring))
        {
            let declaring_name = self.engine.tables.classes[declaring.0 as usize]
                .name
                .clone();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.readonly_error,
                format!(
                    "cannot override readonly property {qualified} with clone! outside the \
                     protected scope of {declaring_name}"
                ),
            ));
        }

        Ok(())
    }

    fn fill_specialized_property_type(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        memo: (ClassId, TypeEnvironmentId, u32),
    ) -> Result<(), VirtualMachineControl> {
        let class = receiver.class();
        let environment = receiver.type_environment();
        let (declaring, declared) = {
            let info = &self.engine.tables.classes[class.0 as usize].slots[slot as usize];
            (info.declaring_class, info.declared_type.clone())
        };

        let specialized = match declared {
            None => None,
            Some(descriptor) => {
                let environment = self
                    .environment_for_class(class, environment, declaring, 0)?
                    .unwrap_or_else(TypeEnvironmentId::default);
                Some(self.substitute_descriptor(&descriptor, environment, 0))
            }
        };

        self.engine
            .tables
            .property_type_cache
            .insert(memo, specialized);
        Ok(())
    }

    /// Checks a property write at a cache site, answering from the site's own
    /// guard when the receiver's specialization and the written value's shape
    /// are the ones the site already proved.
    pub(in crate::vm) fn check_instance_property_value_at_site(
        &mut self,
        site: usize,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        value: &Value,
    ) -> Result<(), VirtualMachineControl> {
        let cache = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let guards = unsafe { &*cache.as_ref().property_guards() };
        let class = receiver.class();
        let environment = receiver.type_environment();
        if let Some(ways) = guards.get(site)
            && ways.iter().flatten().any(|entry| {
                entry.slot == slot
                    && entry.class == class
                    && entry.environment == environment
                    && guard_allows(&entry.guard, value)
            })
        {
            return Ok(());
        }

        self.check_instance_property_value(receiver, slot, value)?;
        self.cache_property_guard(cache, site, receiver, slot, value);
        Ok(())
    }

    /// Records the fact a successful property write established, when the
    /// declared type reduces to one. Only a success is recorded, so a check
    /// that must throw always throws.
    fn cache_property_guard(
        &self,
        cache: NonNull<InlineCache>,
        site: usize,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        value: &Value,
    ) {
        let class = receiver.class();
        let environment = receiver.type_environment();
        let declared = self.engine.tables.classes[class.0 as usize].slots[slot as usize]
            .declared_type
            .as_ref();

        let guard = match declared {
            None => Some(ArgumentGuard::Any),
            Some(descriptor) => match argument_guard(descriptor, value) {
                Some(guard) => Some(guard),
                None => self
                    .engine
                    .tables
                    .property_type_cache
                    .get(&(class, environment, slot))
                    .and_then(|specialized| specialized.as_ref())
                    .and_then(|specialized| argument_guard(specialized, value)),
            },
        };

        let Some(guard) = guard else {
            return;
        };

        let entry = CachedPropertyGuard {
            class,
            environment,
            slot,
            guard,
        };

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let entries = unsafe { &mut *cache.as_ref().property_guards() };
        if entries.len() <= site {
            entries.resize(site + 1, PropertyGuardWays::EMPTY);
        }

        entries[site].record(entry);
    }

    /// Checks an instance property value in the environment of the class
    /// that declared that slot, including inherited generic substitutions.
    pub(in crate::vm) fn check_instance_property_value(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        value: &Value,
    ) -> Result<(), VirtualMachineControl> {
        let descriptor = self.engine.tables.classes[receiver.class().0 as usize].slots
            [slot as usize]
            .declared_type
            .as_ref();
        if let Some(descriptor) = descriptor
            && let Some(valid) = check_trivial_descriptor(descriptor, value)
            && valid
        {
            return Ok(());
        }

        let memo = (receiver.class(), receiver.type_environment(), slot);
        if let Some(specialized) = self.engine.tables.property_type_cache.get(&memo) {
            if let Some(descriptor) = specialized
                && let Some(valid) = check_trivial_descriptor(descriptor, value)
                && valid
            {
                return Ok(());
            }
        } else {
            self.fill_specialized_property_type(receiver, slot, memo)?;
            if let Some(Some(descriptor)) = self.engine.tables.property_type_cache.get(&memo)
                && let Some(valid) = check_trivial_descriptor(descriptor, value)
                && valid
            {
                return Ok(());
            }
        }

        let (declaring, name, descriptor) = {
            let info =
                &self.engine.tables.classes[receiver.class().0 as usize].slots[slot as usize];
            (
                info.declaring_class,
                info.name.clone(),
                info.declared_type.clone(),
            )
        };

        let Some(descriptor) = descriptor else {
            return Ok(());
        };

        let environment = self
            .environment_for_class(receiver.class(), receiver.type_environment(), declaring, 0)?
            .unwrap_or_else(TypeEnvironmentId::default);
        if self.check_descriptor(&descriptor, value, Some(receiver.class()), environment, 0)? {
            return Ok(());
        }

        let concrete = self.substitute_descriptor(&descriptor, environment, 0);
        let expected = self.render_descriptor(&concrete);
        let found = self.value_type_name(value);
        let class = &self.engine.tables.classes[declaring.0 as usize].name;
        Err(self.throw_well_known(
            self.engine.tables.well_known.type_error,
            format!(
                "property {}::${} must be of type {expected}, {found} given",
                class.to_string_lossy(),
                name.to_string_lossy()
            ),
        ))
    }

    fn built_in_property_slot(
        &mut self,
        receiver_class: ClassId,
        name: &Atom,
        scope: Option<ClassId>,
    ) -> Result<u32, VirtualMachineControl> {
        let class = &self.engine.tables.classes[receiver_class.0 as usize];
        let slot = scope
            .and_then(|scope| class.private_slots.get(&(scope, name.clone())).copied())
            .or_else(|| class.slot_names.get(name).copied());
        let Some(slot) = slot else {
            let class_name = class.name.to_string();
            let member = name.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("the property {class_name}::{member} is not declared"),
            ));
        };
        let info = &class.slots[slot as usize];
        if !visibility_allows(
            &self.engine.tables.classes,
            info.visibility,
            info.declaring_class,
            scope,
        ) {
            let rendered = visibility_name(info.visibility);
            let member = info.name.to_string();
            let class_name = self.engine.tables.classes[receiver_class.0 as usize]
                .name
                .to_string();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot access {rendered} property {class_name}::{member}"),
            ));
        }
        Ok(slot)
    }

    pub(crate) fn built_in_read_property(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        name: &Atom,
        scope: Option<ClassId>,
    ) -> Result<Value, VirtualMachineControl> {
        let slot = self.built_in_property_slot(receiver.class(), name, scope)?;
        let value = receiver.read_slot(slot as usize);
        if value.is_uninitialized() {
            return Err(self.uninitialized_property_error(receiver, name));
        }
        Ok(value)
    }

    /// Writes an instance property by name through the checked path. Enforces
    /// declaration, visibility, `readonly` (a readonly slot is writable only
    /// while uninitialized), and the declared type.
    pub(crate) fn built_in_write_property(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        name: &Atom,
        value: Value,
        scope: Option<ClassId>,
    ) -> Result<(), VirtualMachineControl> {
        let slot = self.built_in_property_slot(receiver.class(), name, scope)?;
        let info = &self.engine.tables.classes[receiver.class().0 as usize].slots[slot as usize];
        if info.is_readonly && !receiver.slot_is_uninitialized(slot as usize) {
            let member = info.name.to_string();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.readonly_error,
                format!("cannot write readonly property {member} twice"),
            ));
        }
        self.check_instance_property_value(receiver, slot, &value)?;
        drop(receiver.write_slot(slot as usize, value));
        Ok(())
    }

    /// Answers whether replacing one existing collection element preserves a
    /// property's declared type without rescanning its untouched elements.
    pub(in crate::vm) fn instance_property_index_update_preserves_type(
        &self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        value: &Value,
    ) -> bool {
        let descriptor = self.engine.tables.classes[receiver.class().0 as usize].slots
            [slot as usize]
            .declared_type
            .as_ref();
        match descriptor {
            None
            | Some(
                TypeDescriptor::Wildcard
                | TypeDescriptor::Mixed
                | TypeDescriptor::Vector(None)
                | TypeDescriptor::Dictionary(None),
            ) => true,
            Some(TypeDescriptor::Vector(Some(element)))
            | Some(TypeDescriptor::Dictionary(Some((_, element)))) => {
                check_trivial_descriptor(element, value) == Some(true)
            }
            _ => false,
        }
    }

    /// Answers whether setting one collection element preserves a property's
    /// declared key and value types without rescanning the collection.
    pub(in crate::vm) fn instance_property_index_set_preserves_type(
        &self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        index: &Value,
        value: &Value,
    ) -> bool {
        let descriptor = self.engine.tables.classes[receiver.class().0 as usize].slots
            [slot as usize]
            .declared_type
            .as_ref();
        match descriptor {
            None
            | Some(
                TypeDescriptor::Wildcard
                | TypeDescriptor::Mixed
                | TypeDescriptor::Vector(None)
                | TypeDescriptor::Dictionary(None),
            ) => true,
            Some(TypeDescriptor::Vector(Some(element))) => {
                check_trivial_descriptor(element, value) == Some(true)
            }
            Some(TypeDescriptor::Dictionary(Some((key, element)))) => {
                check_trivial_descriptor(key, index) == Some(true)
                    && check_trivial_descriptor(element, value) == Some(true)
            }
            _ => false,
        }
    }

    /// Checks a static property value. Static declarations cannot capture a
    /// class binder, so their descriptors resolve in the empty environment.
    pub(crate) fn check_static_property_value(
        &mut self,
        class: ClassId,
        slot: u32,
        value: &Value,
    ) -> Result<(), VirtualMachineControl> {
        let (declaring, name, descriptor) = {
            let info = &self.engine.tables.classes[class.0 as usize].statics_info[slot as usize];
            (
                info.declaring_class,
                info.name.clone(),
                info.declared_type.clone(),
            )
        };

        let Some(descriptor) = descriptor else {
            return Ok(());
        };

        if self.check_declared_value(&descriptor, value)? {
            return Ok(());
        }

        let expected = self.render_descriptor(&descriptor);
        let found = self.value_type_name(value);
        let class = &self.engine.tables.classes[declaring.0 as usize].name;
        let error = if self.engine.declaration_depth == 0 {
            self.engine.tables.well_known.type_error
        } else {
            self.engine.tables.well_known.linker_error
        };

        Err(self.throw_well_known(
            error,
            format!(
                "static property {}::${} must be of type {expected}, {found} given",
                class.to_string_lossy(),
                name.to_string_lossy()
            ),
        ))
    }

    /// Resolves a static-property site to its storage class and index.
    pub(in crate::vm) fn static_slot_for(
        &mut self,
        site: usize,
        chunk: &Chunk,
    ) -> Result<(ClassId, u32), VirtualMachineControl> {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };
        if cache.is_empty() {
            cache.resize(chunk.ic_descriptors.len(), CacheEntry::Empty);
        }

        if let CacheEntry::StaticSlot { class, slot } = &cache[site] {
            return Ok((*class, *slot));
        }

        let (class_atom, member) = class_member_atoms(chunk, site);
        let named_class = self.resolve_class_reference(class_atom.clone())?;
        let mut current = Some(named_class);
        let found = loop {
            let Some(class) = current else {
                let member_text = member.to_string_lossy().into_owned();
                let class_text = String::from_utf8_lossy(
                    self.engine.tables.classes[named_class.0 as usize]
                        .name
                        .as_bytes(),
                )
                .into_owned();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!("the static property {class_text}::${member_text} is not declared"),
                ));
            };

            let entry = &self.engine.tables.classes[class.0 as usize];
            if let Some(slot) = entry.static_names.get(member).copied() {
                break (class, slot);
            }

            current = entry.parent;
        };

        let info = &self.engine.tables.classes[found.0.0 as usize].statics_info[found.1 as usize];
        if !visibility_allows(
            &self.engine.tables.classes,
            info.visibility,
            info.declaring_class,
            self.current_frame().class_scope.get(),
        ) {
            let rendered = visibility_name(info.visibility);
            let member_text = member.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot access {rendered} static property ${member_text}"),
            ));
        }

        if *class_atom != self.engine.tables.static_atom {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let cache = unsafe { &mut *cache_cell.entries() };
            cache[site] = CacheEntry::StaticSlot {
                class: found.0,
                slot: found.1,
            };
        }

        Ok(found)
    }

    /// Resolves a class-constant site to its value: a declared constant or
    /// a lazily materialized enum case singleton.
    pub(in crate::vm) fn class_constant_for(
        &mut self,
        site: usize,
        chunk: &Chunk,
    ) -> Result<Value, VirtualMachineControl> {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };

        if cache.is_empty() {
            cache.resize(chunk.ic_descriptors.len(), CacheEntry::Empty);
        }

        if let CacheEntry::ClassConstant(value) = &cache[site] {
            return Ok(value.clone());
        }

        let (class_atom, member) = class_member_atoms(chunk, site);
        let class = self.resolve_class_reference(class_atom.clone())?;
        let (value, forced) = if let Some(case) = self.enum_case_instance(class, member.clone()) {
            (case, false)
        } else {
            let visibility = self.engine.tables.classes[class.0 as usize]
                .constant(member)
                .map(|entry| (entry.visibility, entry.declaring_class));
            let Some((visibility, declaring_class)) = visibility else {
                let class_text = self.engine.tables.classes[class.0 as usize]
                    .name
                    .to_string();
                let member_text = member.to_string_lossy().into_owned();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.undefined_symbol_error,
                    format!("the class constant {class_text}::{member_text} is not defined"),
                ));
            };

            if !visibility_allows(
                &self.engine.tables.classes,
                visibility,
                declaring_class,
                self.current_frame().class_scope.get(),
            ) {
                let rendered = visibility_name(visibility);
                let member_text = member.to_string_lossy().into_owned();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.visibility_error,
                    format!("cannot access {rendered} constant {member_text}"),
                ));
            }

            (self.force_class_constant(class, member.clone())?, true)
        };

        if *class_atom != self.engine.tables.static_atom
            && (!forced || self.class_constant_evaluated(class, member.clone()))
        {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let cache = unsafe { &mut *cache_cell.entries() };
            cache[site] = CacheEntry::ClassConstant(value.clone());
        }

        Ok(value)
    }

    /// Whether a class constant has fully evaluated, for success-only inline
    /// caching.
    fn class_constant_evaluated(&self, class: ClassId, member: Atom) -> bool {
        matches!(self.engine.tables.classes[class.0 as usize].constant(&member), Some(entry) if(matches!(entry.value, ClassConstantValue::Evaluated(_))))
    }

    /// Forces a class-constant slot: an evaluated slot clones its value, a
    /// pending slot runs its initializer once, validates the result against
    /// the declared type, and caches it, and a slot already being forced is a
    /// self-referential cycle. On failure the slot returns to pending.
    pub(crate) fn force_class_constant(
        &mut self,
        class: ClassId,
        member: Atom,
    ) -> Result<Value, VirtualMachineControl> {
        let entry = self.engine.tables.classes[class.0 as usize]
            .constant(&member)
            // SAFETY: the surrounding invariant makes this path unreachable.
            .unwrap_or_else(|| unsafe { unreachable_invariant("the constant was present") });
        match &entry.value {
            ClassConstantValue::Evaluated(value) => return Ok(value.clone()),
            ClassConstantValue::Evaluating => {
                let rendered = self.class_member_text(class, member.clone());
                let error = if self.engine.declaration_depth == 0 {
                    self.engine.tables.well_known.error
                } else {
                    self.engine.tables.well_known.linker_error
                };
                return Err(self
                    .throw_well_known(error, format!("the constant {rendered} refers to itself")));
            }
            ClassConstantValue::Pending { .. } => {}
        }

        let entry = self.engine.tables.classes[class.0 as usize]
            .constant_mut(&member)
            // SAFETY: the surrounding invariant makes this path unreachable.
            .unwrap_or_else(|| unsafe { unreachable_invariant("the constant was present") });
        let declaring_class = entry.declaring_class;
        let declared_type = entry.declared_type.clone();
        let ClassConstantValue::Pending {
            context,
            class_position,
            constant_position,
        } = mem::replace(&mut entry.value, ClassConstantValue::Evaluating)
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the slot was pending") }
        };

        let outcome = match &context.unit.classes[class_position as usize].constants
            [constant_position as usize]
            .initializer
        {
            ConstantInitializer::Literal(literal) => Ok(literal_value(literal)),
            ConstantInitializer::Thunk(chunk) => {
                let chunk_pointer = NonNull::from(&**chunk);
                self.run_initializer(chunk_pointer, &context)
            }
        };

        let value = match outcome {
            Ok(value) => value,
            Err(control) => {
                self.restore_pending_class_constant(
                    class,
                    member,
                    context,
                    class_position,
                    constant_position,
                );

                return Err(control);
            }
        };

        if let Some(declared) = &declared_type {
            let satisfied = self.check_declared_value(declared, &value)?;
            if !satisfied {
                let rendered_type = self.render_descriptor(declared);
                let rendered = self.class_member_text(declaring_class, member.clone());
                let kind = value.kind_name();
                self.restore_pending_class_constant(
                    class,
                    member,
                    context,
                    class_position,
                    constant_position,
                );

                let error = if self.engine.declaration_depth == 0 {
                    self.engine.tables.well_known.type_error
                } else {
                    self.engine.tables.well_known.linker_error
                };

                return Err(self.throw_well_known(
                    error,
                    format!("the constant {rendered} declares {rendered_type}, {kind} given"),
                ));
            }
        }

        if let Some(entry) = self.engine.tables.classes[class.0 as usize].constant_mut(&member) {
            entry.value = ClassConstantValue::Evaluated(value.clone());
        }

        Ok(value)
    }

    /// Restores a class constant to its pending state after a failed force.
    fn restore_pending_class_constant(
        &mut self,
        class: ClassId,
        member: Atom,
        context: Rc<UnitContext>,
        class_position: u32,
        constant_position: u32,
    ) {
        if let Some(entry) = self.engine.tables.classes[class.0 as usize].constant_mut(&member) {
            entry.value = ClassConstantValue::Pending {
                context,
                class_position,
                constant_position,
            };
        }
    }

    fn class_member_text(&self, class: ClassId, member: Atom) -> String {
        format!(
            "{}::{}",
            self.engine.tables.classes[class.0 as usize].name,
            member.to_string_lossy()
        )
    }
}
