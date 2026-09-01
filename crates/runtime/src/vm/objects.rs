//! Instantiating classes and stepping object iterators.

use std::ptr::NonNull;
use std::rc::Rc;

use crate::bytecode::aliases::expand_aliases;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::literal_value;
use crate::classes::PropertyDefault;
use crate::engine::builtins::BuiltInCallable;
use crate::value::ValueView;
use crate::vm::CachedInstantiationEnvironment;
use crate::vm::Chunk;
use crate::vm::ClassId;
use crate::vm::IcDescriptor;
use crate::vm::InstanceObject;
use crate::vm::InstantiationWays;
use crate::vm::IteratorObject;
use crate::vm::ManagedRef;
use crate::vm::MethodBodyKind;
use crate::vm::MethodContext;
use crate::vm::SymbolKind;
use crate::vm::Throw;
use crate::vm::TypeDescriptor;
use crate::vm::TypeEnvironmentId;
use crate::vm::UserCallContext;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::is_instance_of;
use crate::vm::unreachable_invariant;

impl VirtualMachine<'_> {
    /// Instantiates one statically named class site, binding written generic
    /// arguments only after autoloading has made the class metadata available.
    #[inline(never)]
    pub(in crate::vm) fn new_static_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
    ) -> Result<Value, VirtualMachineControl> {
        let outer = self.current_frame().type_environment;

        if let Some(cached) = self.cached_instantiation_environment(site, outer) {
            if cached.allocates_plainly {
                return Ok(Value::object(InstanceObject::new_typed_with_layout(
                    &self.heap,
                    cached.class,
                    cached.slot_count as usize,
                    cached.environment,
                    cached.slots_are_acyclic,
                )));
            }

            return self.new_instance_in_environment(cached.class, cached.environment);
        }

        let (name, written_arguments) = match &chunk.ic_descriptors[site] {
            IcDescriptor::Member {
                name,
                type_arguments,
            } => (name.clone(), type_arguments.clone()),
            // SAFETY: the surrounding invariant makes this path unreachable.
            IcDescriptor::ClassMember { .. } => unsafe {
                unreachable_invariant("a NewStatic site resolves a class name")
            },
        };
        if name != self.engine.tables.static_atom {
            let entry = match self.engine.tables.symbols.get(&name).copied() {
                Some(entry) => Some(entry),
                None => self.resolve_checked_name(name.clone())?,
            };
            if entry.is_some_and(|entry| entry.kind == SymbolKind::TypeAlias) {
                let descriptor = TypeDescriptor::Named {
                    name,
                    arguments: written_arguments,
                    recursive: false,
                };
                let concrete = self.substitute_descriptor(&descriptor, outer, 0);
                let expanded = expand_aliases(&concrete, &self.engine.tables.type_aliases);
                return match expanded {
                    TypeDescriptor::Named {
                        name, arguments, ..
                    } => {
                        let class = self.resolve_class_reference(name)?;
                        let environment = self.instantiation_environment(
                            site,
                            class,
                            arguments.as_deref(),
                            outer,
                        )?;
                        self.new_instance_in_environment(class, environment)
                    }
                    other => Err(self.throw_well_known(
                        self.engine.tables.well_known.type_error,
                        format!(
                            "cannot instantiate the non-class type {}",
                            self.render_descriptor(&other)
                        ),
                    )),
                };
            }
        }

        let type_arguments = match &chunk.ic_descriptors[site] {
            IcDescriptor::Member { type_arguments, .. } => type_arguments.as_deref(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            IcDescriptor::ClassMember { .. } => unsafe {
                unreachable_invariant("a NewStatic site resolves a class name")
            },
        };

        if type_arguments.is_some_and(|arguments| !arguments.is_empty()) {
            let class = self.resolve_class_site(site, chunk)?;
            let environment = self.instantiation_environment(site, class, type_arguments, outer)?;
            return self.new_instance_in_environment(class, environment);
        }

        let class = self.resolve_class_site(site, chunk)?;
        if type_arguments.is_some_and(<[TypeDescriptor]>::is_empty) {
            let inherited = match self.current_this().cloned() {
                Some(instance) => self
                    .environment_for_class(instance.class(), instance.type_environment(), class, 0)?
                    .unwrap_or(outer),
                None => outer,
            };

            return self.new_instance_in_environment(class, inherited);
        }

        if !self.engine.tables.classes[class.0 as usize]
            .type_parameters
            .is_empty()
        {
            let environment = self.instantiation_environment(site, class, type_arguments, outer)?;
            return self.new_instance_in_environment(class, environment);
        }

        if name != self.engine.tables.static_atom {
            let entry = &self.engine.tables.classes[class.0 as usize];
            if entry.allocates_plainly {
                self.record_instantiation_environment(
                    site,
                    CachedInstantiationEnvironment {
                        class,
                        outer,
                        environment: outer,
                        allocates_plainly: true,
                        slots_are_acyclic: entry.slots_are_acyclic,
                        slot_count: entry.slots.len() as u32,
                    },
                );
            }
        }

        self.new_instance_typed(class, type_arguments, outer)
    }

    /// Returns the hot cached class and environment of a named `new` site.
    pub(in crate::vm) fn cached_instantiation_environment(
        &self,
        site: usize,
        outer: TypeEnvironmentId,
    ) -> Option<CachedInstantiationEnvironment> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe {
            &*self
                .current_frame()
                .cache
                .as_ref()
                .instantiation_environments()
        };

        cache.get(site)?.get(outer)
    }

    /// Resolves and caches the reified environment of one named `new` site.
    pub(in crate::vm) fn instantiation_environment(
        &mut self,
        site: usize,
        class: ClassId,
        supplied: Option<&[TypeDescriptor]>,
        outer: TypeEnvironmentId,
    ) -> Result<TypeEnvironmentId, VirtualMachineControl> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe {
            &mut *self
                .current_frame()
                .cache
                .as_ref()
                .instantiation_environments()
        };

        if let Some(entry) = cache.get(site).and_then(|ways| ways.get(outer))
            && entry.class == class
        {
            return Ok(entry.environment);
        }

        let (type_parameters, class_name) = {
            let entry = &self.engine.tables.classes[class.0 as usize];
            (Rc::clone(&entry.type_parameters), entry.name.clone())
        };

        let environment =
            self.bind_type_parameters(&type_parameters, supplied, outer, class_name.as_bytes())?;

        let (allocates_plainly, slots_are_acyclic, slot_count) = {
            let entry = &self.engine.tables.classes[class.0 as usize];
            (
                entry.allocates_plainly,
                entry.slots_are_acyclic,
                entry.slots.len() as u32,
            )
        };

        self.record_instantiation_environment(
            site,
            CachedInstantiationEnvironment {
                class,
                outer,
                environment,
                allocates_plainly,
                slots_are_acyclic,
                slot_count,
            },
        );

        Ok(environment)
    }

    fn record_instantiation_environment(
        &mut self,
        site: usize,
        cached: CachedInstantiationEnvironment,
    ) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe {
            &mut *self
                .current_frame()
                .cache
                .as_ref()
                .instantiation_environments()
        };
        if cache.len() <= site {
            cache.resize(site + 1, InstantiationWays::EMPTY);
        }
        cache[site].record(cached);
    }

    pub(crate) fn value_class_name(&self, instance: &ManagedRef<InstanceObject>) -> String {
        String::from_utf8_lossy(
            self.engine.tables.classes[instance.class().0 as usize]
                .name
                .as_bytes(),
        )
        .into_owned()
    }
    /// The loop cursor of an object subject: an `Iterator` is shared as the
    /// cursor itself (reference semantics; the copy-on-iteration behavior of
    /// collection subjects does not apply), and a `ToIterator` produces one
    /// fresh iterator through `toIterator()`.
    pub(in crate::vm) fn object_cursor(
        &mut self,
        instance: ManagedRef<InstanceObject>,
    ) -> Result<Value, VirtualMachineControl> {
        let iterator_interface = self.engine.tables.iterate_classes.iterator;
        let to_iterator_interface = self.engine.tables.iterate_classes.to_iterator;
        if is_instance_of(
            &self.engine.tables.classes,
            instance.class(),
            iterator_interface,
        ) {
            let next = self.engine.tables.classes[instance.class().0 as usize]
                .method(&self.engine.tables.next_atom)
                // SAFETY: the surrounding invariant makes this path unreachable.
                .unwrap_or_else(|| unsafe {
                    unreachable_invariant("an Iterator implementation has a next method")
                });
            let next_environment = self
                .environment_for_class(
                    instance.class(),
                    instance.type_environment(),
                    next.declaring_class,
                    0,
                )?
                .unwrap_or_else(TypeEnvironmentId::default);
            return Ok(Value::iterator(IteratorObject::new_object(
                &self.heap,
                instance,
                match next.body {
                    MethodBodyKind::Bytecode(function) => Some((function, next.declaring_class)),
                    MethodBodyKind::BuiltIn(_) => None,
                },
                next_environment,
            )));
        }

        if is_instance_of(
            &self.engine.tables.classes,
            instance.class(),
            to_iterator_interface,
        ) {
            let to_iterator_atom = self.engine.tables.to_iterator_atom.clone();
            let produced = self.invoke_method(&instance, to_iterator_atom, &[])?;
            let valid = produced.as_object().is_some_and(|iterator| {
                is_instance_of(
                    &self.engine.tables.classes,
                    iterator.class(),
                    iterator_interface,
                )
            });

            if !valid {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "toIterator() must yield a Whim\\Iterate\\Iterator, {} given",
                        self.value_type_name(&produced)
                    ),
                ));
            }

            let Some(iterator) = produced.as_object() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("the check proved the object") }
            };

            let next = self.engine.tables.classes[iterator.class().0 as usize]
                .method(&self.engine.tables.next_atom)
                // SAFETY: the surrounding invariant makes this path unreachable.
                .unwrap_or_else(|| unsafe {
                    unreachable_invariant("an Iterator implementation has a next method")
                });
            let next_environment = self
                .environment_for_class(
                    iterator.class(),
                    iterator.type_environment(),
                    next.declaring_class,
                    0,
                )?
                .unwrap_or_else(TypeEnvironmentId::default);
            return Ok(Value::iterator(IteratorObject::new_object(
                &self.heap,
                iterator.clone(),
                match next.body {
                    MethodBodyKind::Bytecode(function) => Some((function, next.declaring_class)),
                    MethodBodyKind::BuiltIn(_) => None,
                },
                next_environment,
            )));
        }
        Err(self.throw_well_known(
            self.engine.tables.well_known.type_error,
            format!(
                "foreach over {} requires Whim\\Iterate\\Iterator or Whim\\Iterate\\ToIterator",
                self.value_type_name(&Value::object(instance.clone()))
            ),
        ))
    }

    /// Advances an object cursor implemented by a built-in `next()` method.
    pub(in crate::vm) fn advance_built_in_object_cursor(
        &mut self,
        instance: &ManagedRef<InstanceObject>,
    ) -> Result<Option<(Value, Value)>, VirtualMachineControl> {
        let next_atom = self.engine.tables.next_atom.clone();
        let produced = self.invoke_method(instance, next_atom, &[])?;
        self.decode_object_cursor_result(&produced)
    }

    /// Validates and unpacks the value returned by an iterator's `next()`.
    pub(in crate::vm) fn decode_object_cursor_result(
        &mut self,
        produced: &Value,
    ) -> Result<Option<(Value, Value)>, VirtualMachineControl> {
        match produced.transparent() {
            ValueView::Null => Ok(None),
            ValueView::Tuple(pair) if pair.len() == 2 => {
                let elements = pair.as_slice();
                // SAFETY: the surrounding invariant keeps this index in bounds.
                let key = unsafe { elements.get_unchecked(0) }.clone();
                // SAFETY: the surrounding invariant keeps this index in bounds.
                let value = unsafe { elements.get_unchecked(1) }.clone();

                Ok(Some((key, value)))
            }
            other => Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!(
                    "next() must yield null or a (key, value) tuple, {} given",
                    other.kind_name()
                ),
            )),
        }
    }

    /// Builds an instance after binding and validating the class's concrete
    /// type arguments. Written arguments are resolved in `outer`, so a nested
    /// `new Box::<T>()` retains the caller's binding of `T`.
    #[inline(always)]
    pub(in crate::vm) fn new_instance_typed(
        &mut self,
        class: ClassId,
        supplied: Option<&[TypeDescriptor]>,
        outer: TypeEnvironmentId,
    ) -> Result<Value, VirtualMachineControl> {
        if supplied.is_none() {
            let entry = &self.engine.tables.classes[class.0 as usize];
            if entry.simple_instance {
                let instance = match (entry.destructor.is_some(), entry.default_slots.is_empty()) {
                    (false, true) => InstanceObject::new_typed_with_layout(
                        &self.heap,
                        class,
                        entry.slots.len(),
                        outer,
                        entry.slots_are_acyclic,
                    ),
                    (false, false) => InstanceObject::new_initialized_typed_with_layout(
                        &self.heap,
                        class,
                        entry.initial_slots.len(),
                        outer,
                        entry.slots_are_acyclic,
                        |index| entry.initial_slots[index].clone_inline_scalar(),
                    ),
                    (true, true) => InstanceObject::new_finalizable_typed_with_layout(
                        &self.heap,
                        class,
                        entry.slots.len(),
                        outer,
                        entry.slots_are_acyclic,
                    ),
                    (true, false) => InstanceObject::new_initialized_finalizable_typed_with_layout(
                        &self.heap,
                        class,
                        entry.initial_slots.len(),
                        outer,
                        entry.slots_are_acyclic,
                        |index| entry.initial_slots[index].clone_inline_scalar(),
                    ),
                };
                return Ok(Value::object(instance));
            }
        }

        self.new_instance_typed_slow(class, supplied, outer)
    }

    #[inline(never)]
    fn new_instance_typed_slow(
        &mut self,
        class: ClassId,
        supplied: Option<&[TypeDescriptor]>,
        outer: TypeEnvironmentId,
    ) -> Result<Value, VirtualMachineControl> {
        if supplied.is_none()
            && self.engine.tables.classes[class.0 as usize]
                .type_parameters
                .is_empty()
        {
            return self.new_instance_in_environment(class, outer);
        }

        let (type_parameters, class_name) = {
            let entry = &self.engine.tables.classes[class.0 as usize];
            (Rc::clone(&entry.type_parameters), entry.name.clone())
        };

        let type_environment =
            self.bind_type_parameters(&type_parameters, supplied, outer, class_name.as_bytes())?;
        self.new_instance_in_environment(class, type_environment)
    }

    /// Builds an instance with a class environment already resolved from an
    /// existing `self`, `parent`, or late-static receiver specialization.
    pub(crate) fn new_instance_in_environment(
        &mut self,
        class: ClassId,
        type_environment: TypeEnvironmentId,
    ) -> Result<Value, VirtualMachineControl> {
        let entry = &self.engine.tables.classes[class.0 as usize];
        if entry.allocates_plainly {
            let instance = InstanceObject::new_typed_with_layout(
                &self.heap,
                class,
                entry.slots.len(),
                type_environment,
                entry.slots_are_acyclic,
            );

            return Ok(Value::object(instance));
        }

        let (
            is_instantiable,
            slot_count,
            slots_are_acyclic,
            has_destructor,
            built_in_state_hooks,
            built_in_state_initializers,
        ) = {
            let entry = &self.engine.tables.classes[class.0 as usize];
            (
                !entry.is_abstract && entry.kind == ClassLikeKind::Class,
                entry.slots.len(),
                entry.slots_are_acyclic,
                entry.destructor.is_some(),
                Rc::clone(&entry.built_in_state_hooks),
                Rc::clone(&entry.built_in_state_initializers),
            )
        };

        if !is_instantiable {
            let name = self.engine.tables.classes[class.0 as usize].name.clone();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.instantiation_error,
                format!("cannot instantiate {}", name.to_string_lossy()),
            ));
        }

        let instance = if built_in_state_hooks.is_empty() {
            if has_destructor {
                InstanceObject::new_finalizable_typed_with_layout(
                    &self.heap,
                    class,
                    slot_count,
                    type_environment,
                    slots_are_acyclic,
                )
            } else {
                InstanceObject::new_typed_with_layout(
                    &self.heap,
                    class,
                    slot_count,
                    type_environment,
                    slots_are_acyclic,
                )
            }
        } else {
            debug_assert_eq!(
                built_in_state_hooks.len(),
                built_in_state_initializers.len(),
                "built-in state hooks and initializers are parallel"
            );
            let heap = Rc::clone(&self.heap);
            let outcome = if has_destructor {
                InstanceObject::with_finalizable_built_in_states_typed(
                    &heap,
                    class,
                    slot_count,
                    type_environment,
                    &built_in_state_hooks,
                    |index, destination| built_in_state_initializers[index](self, destination),
                )
            } else {
                InstanceObject::with_built_in_states_typed(
                    &heap,
                    class,
                    slot_count,
                    type_environment,
                    &built_in_state_hooks,
                    |index, destination| built_in_state_initializers[index](self, destination),
                )
            };
            if let Some(code) = self.pending_exit.take() {
                return Err(VirtualMachineControl::Exit(code));
            }
            outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value))?
        };

        let default_count = self.engine.tables.classes[class.0 as usize]
            .default_slots
            .len();
        for index in 0..default_count {
            let slot = self.engine.tables.classes[class.0 as usize].default_slots[index];

            let default = self.engine.tables.classes[class.0 as usize].slots[slot as usize]
                .default
                .clone();
            let Some(default) = default else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("default slot metadata is exact") }
            };

            let default = match default {
                PropertyDefault::Value(value) => value,
                PropertyDefault::Pending {
                    context,
                    class_position,
                    property_position,
                } => {
                    let initializer = &context.unit.classes[class_position as usize].properties
                        [property_position as usize]
                        .default;
                    match initializer {
                        Some(ConstantInitializer::Literal(literal)) => literal_value(literal),
                        Some(ConstantInitializer::Thunk(chunk)) => {
                            self.run_initializer(NonNull::from(&**chunk), &context)?
                        }
                        // SAFETY: the surrounding invariant makes this path unreachable.
                        None => unsafe {
                            unreachable_invariant(
                                "pending property default points to an initializer",
                            )
                        },
                    }
                }
            };

            self.check_instance_property_value(&instance, slot, &default)?;
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            drop(unsafe {
                instance.write_slot_unchecked_with_unique_receiver(slot as usize, default, true)
            });
        }

        Ok(Value::object(instance))
    }

    /// Builds an instance of `class`, cloning slot defaults and running the
    /// constructor when one is declared.
    pub(in crate::vm) fn instantiate_class(
        &mut self,
        class: ClassId,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let created = self.new_instance_typed(class, None, TypeEnvironmentId::default())?;
        self.finish_instance_construction(class, arguments, created)
    }

    fn finish_instance_construction(
        &mut self,
        class: ClassId,
        arguments: &[Value],
        created: Value,
    ) -> Result<Value, VirtualMachineControl> {
        let Some(instance) = created.as_object().cloned() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a new instance is an object") }
        };

        let constructor = self.engine.tables.classes[class.0 as usize]
            .method(&self.engine.tables.constructor_name.clone());

        if let Some(entry) = constructor {
            let type_environment = self
                .environment_for_class(
                    instance.class(),
                    instance.type_environment(),
                    entry.declaring_class,
                    0,
                )?
                .unwrap_or_else(TypeEnvironmentId::default);
            match entry.body {
                MethodBodyKind::Bytecode(function) => {
                    self.call_user_with_context(UserCallContext {
                        function,
                        this: Some(instance.clone()),
                        captures: &[],
                        arguments,
                        method: Some(MethodContext {
                            scope: entry.declaring_class,
                            called: class,
                            is_constructor: true,
                        }),
                        declared_scope: None,
                        type_environment,
                        type_arguments_bound: false,
                    })?;
                }
                MethodBodyKind::BuiltIn(body) => {
                    self.invoke_built_in_callable(
                        BuiltInCallable::Method {
                            body,
                            name: self.engine.tables.constructor_name.clone(),
                        },
                        Some(&instance),
                        arguments,
                    )?;
                }
            }
        }

        Ok(Value::object(instance))
    }
}
