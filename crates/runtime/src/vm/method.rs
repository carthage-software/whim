//! Instance and static method dispatch.

use std::mem;

use crate::bytecode::instruction::operands::PropertyReadMode;
use crate::classes::MethodEntry;
use crate::engine::builtins;
use crate::engine::builtins::BuiltInCallable;
use crate::symbols::GuardedMethodWays;
use crate::vm::Atom;
use crate::vm::CacheEntry;
use crate::vm::CachedExactMethod;
use crate::vm::CachedGuardedMethod;
use crate::vm::CachedMethodArguments;
use crate::vm::CachedMethodFastPath;
use crate::vm::CachedTurbofishEnvironment;
use crate::vm::Chunk;
use crate::vm::ClassId;
use crate::vm::ExactFunctionEntry;
use crate::vm::ExactMethodEntry;
use crate::vm::ExactMethodWays;
use crate::vm::InlineCache;
use crate::vm::InstanceObject;
use crate::vm::Instruction;
use crate::vm::ManagedRef;
use crate::vm::MethodBodyKind;
use crate::vm::MethodContext;
use crate::vm::NonNull;
use crate::vm::TypeEnvironmentId;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::call::guard_allows;
use crate::vm::class_member_atoms;
use crate::vm::name_atom;
use crate::vm::site_type_arguments;
use crate::vm::unreachable_invariant;
use crate::vm::visibility_allows;
use crate::vm::visibility_name;

fn returned_register(instruction: Instruction) -> Option<u16> {
    match instruction {
        Instruction::ReturnUnchecked { source }
        | Instruction::ReturnReferenceUnchecked { source }
        | Instruction::ReturnScalarUnchecked { source } => Some(source.index()),
        _ => None,
    }
}

impl VirtualMachine<'_> {
    /// Dispatches a method site whose exact final receiver, non-generic
    /// method, arity, and argument types were proven by whole-unit type flow.
    #[inline(always)]
    pub(in crate::vm) fn call_exact_method_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
    ) -> Result<(), VirtualMachineControl> {
        debug_assert_ne!(count, 0);
        if site_type_arguments(chunk, site).is_some() {
            return self.call_exact_generic_method_site(
                site,
                chunk,
                destination,
                window_start,
                count,
            );
        }

        // SAFETY: the value's tag proves this projection is valid.
        let receiver = unsafe {
            mem::replace(&mut self.stack[window_start], Value::uninitialized())
                .into_object_unchecked()
        };
        let receiver_class = receiver.class();
        let (method, is_constructor) = self.exact_method_entry_for(site, chunk, receiver_class)?;
        if matches!(method.body, MethodBodyKind::BuiltIn(_)) {
            return self.call_exact_built_in_method(
                site,
                chunk,
                destination,
                window_start,
                count,
                receiver,
                method,
                true,
            );
        }

        let entry = self.exact_bytecode_method_frame_entry_for(
            site,
            chunk,
            method,
            receiver_class,
            is_constructor,
            destination,
        );
        let type_environment = self.exact_method_environment(
            receiver_class,
            receiver.type_environment(),
            entry.scope,
        )?;

        self.push_exact_method_frame(
            entry,
            destination,
            receiver,
            window_start + 1,
            count - 1,
            type_environment,
            false,
        )
    }

    /// Dispatches an optimizer-proven generic method after binding its written
    /// turbofish once for the guarded receiver and caller environments.
    fn call_exact_generic_method_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
    ) -> Result<(), VirtualMachineControl> {
        // SAFETY: the value's tag proves this projection is valid.
        let receiver = unsafe {
            mem::replace(&mut self.stack[window_start], Value::uninitialized())
                .into_object_unchecked()
        };

        let receiver_class = receiver.class();
        let receiver_environment = receiver.type_environment();
        let caller_cache = self.current_frame().cache;
        if let Some(cached) =
            self.cached_guarded_method(caller_cache, site, receiver_class, receiver_environment)
        {
            if let Some(parameter_count) = cached.trivial_constructor_parameters {
                self.call_trivial_constructor(
                    cached.entry,
                    destination,
                    receiver,
                    window_start + 1,
                    parameter_count,
                    false,
                );
                return Ok(());
            }

            return self.push_exact_method_frame(
                cached.entry,
                destination,
                receiver,
                window_start + 1,
                count - 1,
                cached.method_environment,
                false,
            );
        }

        let (method, is_constructor) = self.exact_method_entry_for(site, chunk, receiver_class)?;
        if matches!(method.body, MethodBodyKind::BuiltIn(_)) {
            return self.call_exact_built_in_method(
                site,
                chunk,
                destination,
                window_start,
                count,
                receiver,
                method,
                true,
            );
        }

        let mut entry = self.exact_bytecode_method_frame_entry_for(
            site,
            chunk,
            method,
            receiver_class,
            is_constructor,
            destination,
        );
        entry.function = self.ensure_function_entry_finalized(entry.function)?;
        let outer =
            self.exact_method_environment(receiver_class, receiver_environment, entry.scope)?;
        let name = name_atom(chunk, site);
        let method_environment = self.bind_site_turbofish(chunk, site, method.body, name, outer)?;
        let trivial_constructor_parameters =
            Self::trivial_constructor_parameter_count(&entry, count - 1);

        let caller_environment = self.current_frame().type_environment;
        Self::cache_guarded_method(
            caller_cache,
            site,
            CachedGuardedMethod {
                receiver_class,
                receiver_environment,
                caller_environment,
                method_environment,
                entry,
                arguments: CachedMethodArguments::Proven,
                trivial_constructor_parameters,
                fast_path: CachedMethodFastPath::None,
            },
        );

        if let Some(parameter_count) = trivial_constructor_parameters {
            self.call_trivial_constructor(
                entry,
                destination,
                receiver,
                window_start + 1,
                parameter_count,
                false,
            );
            return Ok(());
        }

        self.push_exact_method_frame(
            entry,
            destination,
            receiver,
            window_start + 1,
            count - 1,
            method_environment,
            false,
        )
    }

    /// Dispatches a whole-unit-proven method directly from caller registers.
    /// The caller remains suspended with the receiver alive, so a narrow
    /// callee frame can borrow it instead of retaining and releasing it.
    pub(in crate::vm) fn call_direct_method_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
    ) -> Result<(), VirtualMachineControl> {
        debug_assert_ne!(count, 0);
        let (receiver_class, receiver_environment) = {
            // SAFETY: the value's tag proves this projection is valid.
            let receiver = unsafe { self.stack[window_start].as_object_unchecked() };
            (receiver.class(), receiver.type_environment())
        };
        if site_type_arguments(chunk, site).is_some() {
            let caller_cache = self.current_frame().cache;
            if let Some(cached) =
                self.cached_guarded_method(caller_cache, site, receiver_class, receiver_environment)
            {
                return self.push_direct_method_frame(
                    cached.entry,
                    destination,
                    window_start,
                    count - 1,
                    cached.method_environment,
                );
            }

            let (method, is_constructor) =
                self.exact_method_entry_for(site, chunk, receiver_class)?;
            if matches!(method.body, MethodBodyKind::BuiltIn(_)) {
                // SAFETY: the value's tag proves this projection is valid.
                let receiver = unsafe { self.stack[window_start].as_object_unchecked().clone() };
                return self.call_exact_built_in_method(
                    site,
                    chunk,
                    destination,
                    window_start,
                    count,
                    receiver,
                    method,
                    false,
                );
            }

            let entry = self.exact_bytecode_method_frame_entry_for(
                site,
                chunk,
                method,
                receiver_class,
                is_constructor,
                destination,
            );
            let outer =
                self.exact_method_environment(receiver_class, receiver_environment, entry.scope)?;
            let name = name_atom(chunk, site);
            let method_environment =
                self.bind_site_turbofish(chunk, site, method.body, name, outer)?;

            let caller_environment = self.current_frame().type_environment;
            Self::cache_guarded_method(
                caller_cache,
                site,
                CachedGuardedMethod {
                    receiver_class,
                    receiver_environment,
                    caller_environment,
                    method_environment,
                    entry,
                    arguments: CachedMethodArguments::Proven,
                    trivial_constructor_parameters: None,
                    fast_path: CachedMethodFastPath::None,
                },
            );

            return self.push_direct_method_frame(
                entry,
                destination,
                window_start,
                count - 1,
                method_environment,
            );
        }

        let (method, is_constructor) = self.exact_method_entry_for(site, chunk, receiver_class)?;
        if matches!(method.body, MethodBodyKind::BuiltIn(_)) {
            // SAFETY: the value's tag proves this projection is valid.
            let receiver = unsafe { self.stack[window_start].as_object_unchecked().clone() };
            return self.call_exact_built_in_method(
                site,
                chunk,
                destination,
                window_start,
                count,
                receiver,
                method,
                false,
            );
        }

        let entry = self.exact_bytecode_method_frame_entry_for(
            site,
            chunk,
            method,
            receiver_class,
            is_constructor,
            destination,
        );
        let type_environment =
            self.exact_method_environment(receiver_class, receiver_environment, entry.scope)?;
        self.push_direct_method_frame(
            entry,
            destination,
            window_start,
            count - 1,
            type_environment,
        )
    }

    /// Returns the compact frame metadata for an exact direct method site,
    /// resolving and copying it out of the engine's wider tables once.
    #[inline(always)]
    fn exact_bytecode_method_frame_entry_for(
        &self,
        site: usize,
        chunk: &Chunk,
        method: MethodEntry,
        receiver_class: ClassId,
        is_constructor: bool,
        destination: u16,
    ) -> ExactMethodEntry {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &*cache_cell.exact_methods() };
        if !cache.is_empty()
            // SAFETY: the surrounding invariant keeps this index in bounds.
            && let Some(entry) = unsafe { *cache.get_unchecked(site) }
            && entry.called == receiver_class
        {
            return entry;
        }

        let entry = self.exact_method_frame_entry(
            chunk,
            method,
            receiver_class,
            is_constructor,
            destination,
        );

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.exact_methods() };
        if cache.is_empty() {
            cache.resize(chunk.ic_descriptors.len(), None);
        }

        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe {
            *cache.get_unchecked_mut(site) = Some(entry);
        }

        entry
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the arguments describe the two optimized method-call window shapes"
    )]
    fn call_exact_built_in_method(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
        receiver: ManagedRef<InstanceObject>,
        method: MethodEntry,
        clear_window: bool,
    ) -> Result<(), VirtualMachineControl> {
        let MethodBodyKind::BuiltIn(body) = method.body else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a built-in exact call has a built-in method body") }
        };
        let receiver_class = receiver.class();
        let outer = self.exact_method_environment(
            receiver_class,
            receiver.type_environment(),
            method.declaring_class,
        )?;
        let name = name_atom(chunk, site);
        let turbofish_bound = site_type_arguments(chunk, site).is_some();
        let environment = self.bind_site_turbofish(chunk, site, method.body, name, outer)?;
        if !clear_window && turbofish_bound != body.type_parameters.is_empty() {
            let outcome = self.invoke_proven_built_in_method_from_stack(
                body,
                name.clone(),
                receiver_class,
                environment,
                window_start,
                count,
            );
            let value = outcome?;
            let target = self.current_frame().base as usize + usize::from(destination);
            self.stack[target] = value;
            return Ok(());
        }

        let mut arguments = Vec::with_capacity(count - 1);
        for position in 1..count {
            arguments.push(self.stack[window_start + position].clone());
        }

        let outcome = self.invoke_built_in_callable_called(
            BuiltInCallable::Method {
                body,
                name: name.clone(),
            },
            Some(&receiver),
            &arguments,
            Some(receiver_class),
            environment,
            turbofish_bound,
            &[],
        );
        if clear_window {
            self.clear_argument_window(window_start, count);
        }

        let value = outcome?;
        let target = self.current_frame().base as usize + usize::from(destination);
        self.stack[target] = value;
        Ok(())
    }

    fn exact_method_frame_entry(
        &self,
        caller: &Chunk,
        method: MethodEntry,
        receiver_class: ClassId,
        is_constructor: bool,
        destination: u16,
    ) -> ExactMethodEntry {
        let MethodBodyKind::Bytecode(function) = method.body else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("an exact method resolves a bytecode body") }
        };

        debug_assert!(!method.is_static);
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let runtime = unsafe {
            self.engine
                .tables
                .functions
                .get_unchecked(function.0 as usize)
        };
        ExactMethodEntry {
            function: ExactFunctionEntry::from_call_site(function, runtime, caller, destination),
            scope: method.declaring_class,
            called: receiver_class,
            is_constructor,
        }
    }

    fn trivial_constructor_parameter_count(
        entry: &ExactMethodEntry,
        argument_count: usize,
    ) -> Option<u8> {
        if !entry.is_constructor
            || !entry.function.finalized
            || usize::from(entry.function.declared_parameters) != argument_count
        {
            return None;
        }

        let parameter_count = u8::try_from(argument_count).ok()?;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { entry.function.chunk.as_ref() };
        if chunk.code.len() != argument_count + 1
            || !matches!(chunk.code.last(), Some(Instruction::ReturnNullUnchecked))
        {
            return None;
        }

        for (position, instruction) in chunk.code[..argument_count].iter().enumerate() {
            let Instruction::PropertySetUnchecked {
                object,
                value,
                value_mode,
                ..
            } = instruction
            else {
                return None;
            };
            if object.index() != 0
                || value.index() != u16::try_from(position + 1).ok()?
                || !value_mode.fresh_receiver()
            {
                return None;
            }
        }

        Some(parameter_count)
    }

    fn cached_method_fast_path(
        entry: &ExactMethodEntry,
        argument_count: usize,
    ) -> CachedMethodFastPath {
        if !entry.function.finalized {
            return CachedMethodFastPath::None;
        }

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { entry.function.chunk.as_ref() };
        let mut code = chunk.code.as_slice();
        if matches!(code.last(), Some(Instruction::ReturnNull)) {
            code = &code[..code.len() - 1];
        }

        if let [instruction] = code
            && let Some(source) = returned_register(*instruction)
        {
            if source == 0 {
                return CachedMethodFastPath::ReturnReceiver;
            }
            if usize::from(source) <= argument_count {
                return CachedMethodFastPath::ReturnArgument(source as u8 - 1);
            }
        }

        if let [
            Instruction::PropertyGetUnchecked {
                destination,
                object,
                slot,
                value_mode: PropertyReadMode::Clone,
            },
            returned,
        ] = code
            && object.index() == 0
            && returned_register(*returned) == Some(destination.index())
        {
            return CachedMethodFastPath::ReturnProperty(slot.index());
        }

        CachedMethodFastPath::None
    }

    #[inline(always)]
    fn call_cached_method_fast_path(
        &mut self,
        fast_path: CachedMethodFastPath,
        destination: u16,
        receiver: ManagedRef<InstanceObject>,
        argument_start: usize,
        argument_count: usize,
    ) -> Result<(), ManagedRef<InstanceObject>> {
        let value = match fast_path {
            CachedMethodFastPath::None => return Err(receiver),
            CachedMethodFastPath::ReturnReceiver => Value::object(receiver),
            CachedMethodFastPath::ReturnArgument(position) => {
                let value = mem::replace(
                    &mut self.stack[argument_start + usize::from(position)],
                    Value::uninitialized(),
                );
                drop(receiver);
                value
            }
            CachedMethodFastPath::ReturnProperty(slot) => {
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                let value = unsafe { receiver.read_slot_unchecked(usize::from(slot)) };
                if value.is_uninitialized() {
                    drop(value);
                    return Err(receiver);
                }
                drop(receiver);
                value
            }
        };

        self.clear_argument_window(argument_start, argument_count);
        let target = self.current_frame().base as usize + usize::from(destination);
        self.stack[target] = value;
        Ok(())
    }

    #[inline(always)]
    fn call_trivial_constructor(
        &mut self,
        entry: ExactMethodEntry,
        destination: u16,
        receiver: ManagedRef<InstanceObject>,
        argument_start: usize,
        parameter_count: u8,
        discard_result: bool,
    ) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let code = &unsafe { entry.function.chunk.as_ref() }.code;
        for position in 0..usize::from(parameter_count) {
            let Instruction::PropertySetUnchecked { slot, .. } =
                // SAFETY: the surrounding invariant keeps this index in bounds.
                (unsafe { code.get_unchecked(position) })
            else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant("a trivial constructor contains only property writes")
                }
            };
            let value = mem::replace(
                &mut self.stack[argument_start + position],
                Value::uninitialized(),
            );
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            unsafe {
                receiver.write_fresh_slot_unchecked(usize::from(slot.index()), value);
            }
        }

        if !discard_result {
            let target = self.current_frame().base as usize + usize::from(destination);
            self.stack[target] = Value::null();
        }
    }

    #[inline(always)]
    fn cached_guarded_method(
        &self,
        cache: NonNull<InlineCache>,
        site: usize,
        receiver_class: ClassId,
        receiver_environment: TypeEnvironmentId,
    ) -> Option<CachedGuardedMethod> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let entries = unsafe { &*cache.as_ref().guarded_methods() };
        if site >= entries.len() {
            return None;
        }

        // SAFETY: the surrounding invariant keeps this index in bounds.
        let entry = unsafe { *entries.get_unchecked(site) }?;
        (entry.receiver_class == receiver_class
            && entry.receiver_environment == receiver_environment
            && entry.caller_environment == self.current_frame().type_environment)
            .then_some(entry)
    }

    #[inline(always)]
    fn cached_polymorphic_guarded_method(
        &self,
        cache: NonNull<InlineCache>,
        site: usize,
        receiver_class: ClassId,
        receiver_environment: TypeEnvironmentId,
    ) -> Option<CachedGuardedMethod> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let entries = unsafe { &*cache.as_ref().polymorphic_guarded_methods() };
        entries.get(site)?.get(
            receiver_class,
            receiver_environment,
            self.current_frame().type_environment,
        )
    }

    fn cache_polymorphic_guarded_method(
        cache: NonNull<InlineCache>,
        site: usize,
        entry: CachedGuardedMethod,
    ) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let entries = unsafe { &mut *cache.as_ref().polymorphic_guarded_methods() };
        if entries.len() <= site {
            entries.resize(site + 1, GuardedMethodWays::EMPTY);
        }
        entries[site].record(entry);
    }

    fn cache_guarded_method(cache: NonNull<InlineCache>, site: usize, entry: CachedGuardedMethod) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let entries = unsafe { &mut *cache.as_ref().guarded_methods() };
        if entries.len() <= site {
            entries.resize(site + 1, None);
        }

        entries[site] = Some(entry);
    }

    #[inline(always)]
    fn cached_method_arguments_match(
        &mut self,
        cache: NonNull<InlineCache>,
        site: usize,
        cached: CachedGuardedMethod,
        window_start: usize,
        count: usize,
    ) -> Result<bool, VirtualMachineControl> {
        Ok(match cached.arguments {
            CachedMethodArguments::Proven => true,
            CachedMethodArguments::One(guard) => {
                count == 1
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    && guard_allows(&guard, unsafe { self.stack.get_unchecked(window_start) })
            }
            CachedMethodArguments::General => self.cached_argument_guards_match(
                cache,
                site,
                cached.entry.function.function,
                cached.method_environment,
                window_start..window_start + count,
                Some(cached.entry.called),
            )?,
        })
    }

    fn exact_method_environment(
        &mut self,
        receiver_class: ClassId,
        receiver_environment: TypeEnvironmentId,
        declaring_class: ClassId,
    ) -> Result<TypeEnvironmentId, VirtualMachineControl> {
        if receiver_class == declaring_class {
            return Ok(receiver_environment);
        }

        Ok(self
            .environment_for_class(receiver_class, receiver_environment, declaring_class, 0)?
            .unwrap_or_default())
    }

    /// Resolves a whole-unit-proven method once, then uses an unguarded exact
    /// cache entry. Visibility remains checked on the fill path.
    fn exact_method_entry_for(
        &mut self,
        site: usize,
        chunk: &Chunk,
        receiver_class: ClassId,
    ) -> Result<(MethodEntry, bool), VirtualMachineControl> {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let ways = unsafe { &*cache_cell.exact_method_ways() };
        if let Some(way) = ways.get(site)
            && let Some(resolved) = way.get(receiver_class)
        {
            return Ok(resolved);
        }

        let name = name_atom(chunk, site);
        let is_constructor = *name == self.engine.tables.constructor_name;
        let entry = self.method_entry_for(site, chunk, receiver_class, name)?;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let ways = unsafe { &mut *cache_cell.exact_method_ways() };
        if ways.len() <= site {
            ways.resize(
                chunk.ic_descriptors.len().max(site + 1),
                ExactMethodWays::EMPTY,
            );
        }

        ways[site].record(CachedExactMethod {
            class: receiver_class,
            entry,
            is_constructor,
        });

        Ok((entry, is_constructor))
    }

    /// Dispatches a `CallMethod` site: the receiver leads the window, the
    /// method resolves through the class-guarded inline cache, and a
    /// bytecode body becomes a real frame.
    #[expect(
        clippy::too_many_arguments,
        reason = "the hot call site passes its shape without allocating a context"
    )]
    pub(in crate::vm) fn call_method_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
        arguments_proven: bool,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        if count == 0 {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                "a method call requires a receiver".to_string(),
            ));
        }

        let receiver = mem::replace(&mut self.stack[window_start], Value::uninitialized());
        if !receiver.is_object() {
            let kind = receiver.kind_name();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("cannot call a method on {kind}"),
            ));
        }
        // SAFETY: the value's tag proves this projection is valid.
        let mut receiver = unsafe { receiver.into_object_unchecked() };

        let caller_cache = self.current_frame().cache;
        let receiver_class = receiver.class();
        let receiver_environment = receiver.type_environment();
        let argument_start = window_start + 1;
        let argument_count = count - 1;
        if let Some(cached) = self.cached_polymorphic_guarded_method(
            caller_cache,
            site,
            receiver_class,
            receiver_environment,
        ) && (arguments_proven
            || self.cached_method_arguments_match(
                caller_cache,
                site,
                cached,
                argument_start,
                argument_count,
            )?)
        {
            if !discard_result {
                match self.call_cached_method_fast_path(
                    cached.fast_path,
                    destination,
                    receiver,
                    argument_start,
                    argument_count,
                ) {
                    Ok(()) => return Ok(()),
                    Err(returned_receiver) => receiver = returned_receiver,
                }
            }

            if let Some(parameter_count) = cached.trivial_constructor_parameters {
                self.call_trivial_constructor(
                    cached.entry,
                    destination,
                    receiver,
                    argument_start,
                    parameter_count,
                    discard_result,
                );
                return Ok(());
            }

            return self.push_exact_method_frame(
                cached.entry,
                destination,
                receiver,
                argument_start,
                argument_count,
                cached.method_environment,
                discard_result,
            );
        }

        let name = name_atom(chunk, site);
        if *name == self.engine.tables.constructor_name
            && self.engine.tables.classes[receiver.class().0 as usize]
                .method(name)
                .is_none()
        {
            if count > 1 {
                let class_text = self.value_type_name(&Value::object(receiver));
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.argument_count_error,
                    format!(
                        "{class_text} has no constructor, {} arguments given",
                        count - 1
                    ),
                ));
            }

            let target = self.current_frame().base as usize + usize::from(destination);
            self.stack[target] = Value::null();
            return Ok(());
        }

        let entry = self.method_entry_for(site, chunk, receiver.class(), name)?;
        if entry.is_static {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!(
                    "cannot call the static method {} through an instance",
                    name.to_string_lossy()
                ),
            ));
        }

        let is_constructor = *name == self.engine.tables.constructor_name;
        let context = MethodContext {
            scope: entry.declaring_class,
            called: receiver_class,
            is_constructor,
        };

        let type_environment = self
            .environment_for_class(
                receiver_class,
                receiver_environment,
                entry.declaring_class,
                0,
            )?
            .unwrap_or_else(TypeEnvironmentId::default);

        let turbofish_bound = site_type_arguments(chunk, site).is_some();
        let type_environment =
            self.bind_site_turbofish(chunk, site, entry.body, name, type_environment)?;
        match entry.body {
            MethodBodyKind::Bytecode(function) => {
                let arguments_proven = arguments_proven
                    || self.cached_argument_guards_match(
                        caller_cache,
                        site,
                        function,
                        type_environment,
                        argument_start..argument_start + argument_count,
                        Some(context.called),
                    )?;

                let caller_environment = if self.engine.tables.functions[function.0 as usize]
                    .type_parameters()
                    .is_empty()
                    || turbofish_bound
                {
                    Some(self.current_frame().type_environment)
                } else {
                    None
                };

                let frame_start = self.stack.len();
                let outcome = self.push_user_frame(
                    function,
                    destination,
                    Some(receiver),
                    &[],
                    argument_start,
                    argument_count,
                    Some(context),
                    None,
                    None,
                    type_environment,
                    turbofish_bound,
                    arguments_proven,
                    discard_result,
                    false,
                );

                if outcome.is_ok() && !arguments_proven {
                    self.cache_argument_guards(
                        caller_cache,
                        site,
                        function,
                        type_environment,
                        frame_start + 1,
                        argument_count,
                    );
                }

                if outcome.is_ok()
                    && let Some(caller_environment) = caller_environment
                {
                    let exact_entry = self.exact_method_frame_entry(
                        chunk,
                        entry,
                        receiver_class,
                        is_constructor,
                        destination,
                    );
                    let mut cached = CachedGuardedMethod {
                        receiver_class,
                        receiver_environment,
                        caller_environment,
                        method_environment: type_environment,
                        entry: exact_entry,
                        arguments: CachedMethodArguments::General,
                        trivial_constructor_parameters: Self::trivial_constructor_parameter_count(
                            &exact_entry,
                            argument_count,
                        ),
                        fast_path: Self::cached_method_fast_path(&exact_entry, argument_count),
                    };
                    cached.arguments = if arguments_proven || argument_count == 0 {
                        CachedMethodArguments::Proven
                    } else if let Some(guard) = Self::cached_single_argument_guard(
                        caller_cache,
                        site,
                        function,
                        type_environment,
                    ) {
                        CachedMethodArguments::One(guard)
                    } else {
                        CachedMethodArguments::General
                    };
                    Self::cache_polymorphic_guarded_method(caller_cache, site, cached);
                }

                outcome
            }
            MethodBodyKind::BuiltIn(body) => {
                let mut arguments = Vec::with_capacity(count - 1);
                for position in 1..count {
                    arguments.push(self.stack[window_start + position].clone());
                }

                let outcome = self.invoke_built_in_callable_called(
                    BuiltInCallable::Method {
                        body,
                        name: name.clone(),
                    },
                    Some(&receiver),
                    &arguments,
                    Some(context.called),
                    type_environment,
                    turbofish_bound,
                    &[],
                );

                self.clear_argument_window(window_start, count);
                let value = outcome?;

                let target = self.current_frame().base as usize + usize::from(destination);
                self.stack[target] = value;
                Ok(())
            }
        }
    }

    /// Resolves a method through the class-guarded inline cache, checking
    /// abstractness and visibility on the fill path.
    fn method_entry_for(
        &mut self,
        site: usize,
        chunk: &Chunk,
        receiver_class: ClassId,
        name: &Atom,
    ) -> Result<MethodEntry, VirtualMachineControl> {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };
        if cache.is_empty() {
            cache.resize(chunk.ic_descriptors.len(), CacheEntry::Empty);
        }

        if let CacheEntry::Method { class, entry } = &cache[site]
            && *class == receiver_class
        {
            return Ok(*entry);
        }

        let class = &self.engine.tables.classes[receiver_class.0 as usize];
        let entry = self
            .current_frame()
            .class_scope
            .get()
            .and_then(|scope| class.private_methods.get(&(scope, name.clone())).copied())
            .or_else(|| class.method(name));

        let Some(entry) = entry else {
            let class_text = String::from_utf8_lossy(
                self.engine.tables.classes[receiver_class.0 as usize]
                    .name
                    .as_bytes(),
            )
            .into_owned();
            let member = name.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("call to undefined method {class_text}::{member}"),
            ));
        };

        if entry.is_abstract {
            let member = name.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("cannot call the abstract method {member}"),
            ));
        }

        if !visibility_allows(
            &self.engine.tables.classes,
            entry.visibility,
            entry.declaring_class,
            self.current_frame().class_scope.get(),
        ) {
            let rendered = visibility_name(entry.visibility);
            let member = name.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot call {rendered} method {member}"),
            ));
        }

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };
        cache[site] = CacheEntry::Method {
            class: receiver_class,
            entry,
        };

        Ok(entry)
    }

    /// Binds a turbofish written on a call site over `outer`, the environment
    /// the callee's own type parameters are declared in. Returns `outer`
    /// unchanged when the site carries none.
    fn bind_site_turbofish(
        &mut self,
        chunk: &Chunk,
        site: usize,
        body: MethodBodyKind,
        name: &Atom,
        outer: TypeEnvironmentId,
    ) -> Result<TypeEnvironmentId, VirtualMachineControl> {
        let Some(supplied) = site_type_arguments(chunk, site) else {
            return Ok(outer);
        };

        let caller_environment = self.current_frame().type_environment;
        let cache = self.current_frame().cache;
        if let MethodBodyKind::Bytecode(function) = body {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let entries = unsafe { &*cache.as_ref().turbofish_environments() };
            if let Some(Some(entry)) = entries.get(site)
                && entry.function == function
                && entry.outer == outer
                && entry.caller == caller_environment
            {
                return Ok(entry.environment);
            }
        }

        let parameters = match body {
            MethodBodyKind::Bytecode(function) => self.engine.tables.functions[function.0 as usize]
                .type_parameters()
                .to_vec(),
            MethodBodyKind::BuiltIn(body) => {
                let callable = BuiltInCallable::Method {
                    body,
                    name: name.clone(),
                };

                builtins::built_in_type_parameters(&self.heap, callable.type_parameters())
            }
        };

        let environment = self.bind_type_parameters_from(
            &parameters,
            Some(supplied),
            caller_environment,
            outer,
            name.as_bytes(),
        )?;
        if let MethodBodyKind::Bytecode(function) = body {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let entries = unsafe { &mut *cache.as_ref().turbofish_environments() };
            if entries.len() <= site {
                entries.resize(site + 1, None);
            }
            entries[site] = Some(CachedTurbofishEnvironment {
                function,
                outer,
                caller: caller_environment,
                environment,
            });
        }

        Ok(environment)
    }

    /// Dispatches a `CallStatic` site: `static::` resolves through the
    /// caller's late-bound called class; `self::`, `parent::`, and a named
    /// class resolve as an ordinary class reference. A non-static method
    /// reached this way is the non-virtual form: it passes the caller's
    /// receiver through and preserves its called class.
    pub(in crate::vm) fn call_static_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        let (class_atom, member) = class_member_atoms(chunk, site);
        let caller_cache = self.current_frame().cache;
        let caller_environment = self.current_frame().type_environment;
        {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let entries = unsafe { &*caller_cache.as_ref().guarded_methods() };
            if let Some(cached) = entries.get(site).copied().flatten()
                && cached.caller_environment == caller_environment
                && (*class_atom != self.engine.tables.static_atom
                    || self.current_frame().called_class.get() == Some(cached.receiver_class))
                && self.cached_argument_guards_match(
                    caller_cache,
                    site,
                    cached.entry.function.function,
                    cached.method_environment,
                    window_start..window_start + count,
                    Some(cached.entry.called),
                )?
            {
                return self.push_exact_static_method_frame(
                    cached.entry,
                    destination,
                    window_start,
                    count,
                    cached.method_environment,
                    discard_result,
                );
            }
        }

        let class = if *class_atom == self.engine.tables.static_atom
            || class_atom.as_bytes().starts_with(b"@")
        {
            self.resolve_class_reference(class_atom.clone())?
        } else {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let cache_cell = unsafe { self.current_frame().cache.as_ref() };
            let cached = {
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                let cache = unsafe { &mut *cache_cell.entries() };
                if cache.is_empty() {
                    cache.resize(chunk.ic_descriptors.len(), CacheEntry::Empty);
                }
                match cache[site] {
                    CacheEntry::Class(class) => Some(class),
                    _ => None,
                }
            };

            match cached {
                Some(class) => class,
                None => {
                    let class = self.resolve_class_reference(class_atom.clone())?;
                    // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                    let cache = unsafe { &mut *cache_cell.entries() };
                    cache[site] = CacheEntry::Class(class);
                    class
                }
            }
        };

        let entry = self.engine.tables.classes[class.0 as usize].method(member);
        let Some(entry) = entry else {
            let class_text = self.engine.tables.classes[class.0 as usize]
                .name
                .to_string();
            let member_text = member.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("call to undefined method {class_text}::{member_text}"),
            ));
        };

        if entry.is_abstract {
            let member_text = member.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("cannot call the abstract method {member_text}"),
            ));
        }

        if !visibility_allows(
            &self.engine.tables.classes,
            entry.visibility,
            entry.declaring_class,
            self.current_frame().class_scope.get(),
        ) {
            let rendered = visibility_name(entry.visibility);
            let member_text = member.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot call {rendered} method {member_text}"),
            ));
        }

        let (this, context) = if entry.is_static {
            (
                None,
                MethodContext {
                    scope: entry.declaring_class,
                    called: class,
                    is_constructor: false,
                },
            )
        } else {
            let Some(this) = self.current_this().cloned() else {
                let member_text = member.to_string_lossy().into_owned();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!("cannot call the instance method {member_text} statically"),
                ));
            };

            let called = self.current_frame().called_class.get().unwrap_or(class);
            (
                Some(this),
                MethodContext {
                    scope: entry.declaring_class,
                    called,
                    is_constructor: *member == self.engine.tables.constructor_name,
                },
            )
        };

        let type_environment = match &this {
            Some(this) => self
                .environment_for_class(
                    this.class(),
                    this.type_environment(),
                    entry.declaring_class,
                    0,
                )?
                .unwrap_or_else(TypeEnvironmentId::default),
            None => TypeEnvironmentId::default(),
        };

        let turbofish_bound = site_type_arguments(chunk, site).is_some();
        let type_environment =
            self.bind_site_turbofish(chunk, site, entry.body, member, type_environment)?;
        match entry.body {
            MethodBodyKind::Bytecode(function) => {
                let arguments_proven = self.cached_argument_guards_match(
                    caller_cache,
                    site,
                    function,
                    type_environment,
                    window_start..window_start + count,
                    Some(context.called),
                )?;

                let frame_start = self.stack.len();
                let outcome = self.push_user_frame(
                    function,
                    destination,
                    this.clone(),
                    &[],
                    window_start,
                    count,
                    Some(context),
                    None,
                    None,
                    type_environment,
                    turbofish_bound,
                    arguments_proven,
                    discard_result,
                    false,
                );

                if outcome.is_ok() && !arguments_proven {
                    let offset = usize::from(this.is_some());
                    self.cache_argument_guards(
                        caller_cache,
                        site,
                        function,
                        type_environment,
                        frame_start + offset,
                        count,
                    );
                }

                if outcome.is_ok()
                    && this.is_none()
                    && (turbofish_bound
                        || self.engine.tables.functions[function.0 as usize]
                            .type_parameters()
                            .is_empty())
                {
                    let runtime = &self.engine.tables.functions[function.0 as usize];
                    let cached = CachedGuardedMethod {
                        receiver_class: context.called,
                        receiver_environment: TypeEnvironmentId::default(),
                        caller_environment,
                        entry: ExactMethodEntry {
                            function: ExactFunctionEntry::from_call_site(
                                function,
                                runtime,
                                chunk,
                                destination,
                            ),
                            scope: context.scope,
                            called: context.called,
                            is_constructor: false,
                        },
                        method_environment: type_environment,
                        arguments: CachedMethodArguments::General,
                        trivial_constructor_parameters: None,
                        fast_path: CachedMethodFastPath::None,
                    };

                    // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                    let entries = unsafe { &mut *caller_cache.as_ref().guarded_methods() };
                    if entries.len() <= site {
                        entries.resize(site + 1, None);
                    }

                    entries[site] = Some(cached);
                }

                outcome
            }
            MethodBodyKind::BuiltIn(body) => {
                let mut arguments = Vec::with_capacity(count);
                for position in 0..count {
                    arguments.push(self.stack[window_start + position].clone());
                }

                let outcome = self.invoke_built_in_callable_called(
                    BuiltInCallable::Method {
                        body,
                        name: member.clone(),
                    },
                    this.as_ref(),
                    &arguments,
                    Some(context.called),
                    type_environment,
                    turbofish_bound,
                    &[],
                );

                self.clear_argument_window(window_start, count);
                let value = outcome?;

                let target = self.current_frame().base as usize + usize::from(destination);
                self.stack[target] = value;
                Ok(())
            }
        }
    }
}
