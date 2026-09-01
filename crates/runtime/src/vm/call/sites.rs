//! Resolving and executing the inline-cache call sites.

use std::rc::Rc;

use crate::symbols::CachedNewtypeConstructor;
use crate::symbols::ExactBuiltInFunctionEntry;
use crate::symbols::NewtypeConstructorWays;
use crate::value::ValueView;
use crate::value::newtype::NewtypeId;
use crate::vm::call::BuiltInCallable;
use crate::vm::call::BuiltInId;
use crate::vm::call::CacheEntry;
use crate::vm::call::CachedCallEnvironment;
use crate::vm::call::CallDescriptor;
use crate::vm::call::CallTarget;
use crate::vm::call::Chunk;
use crate::vm::call::FunctionSpec;
use crate::vm::call::IcDescriptor;
use crate::vm::call::TypeDescriptor;
use crate::vm::call::TypeEnvironmentId;
use crate::vm::call::Value;
use crate::vm::call::VirtualMachine;
use crate::vm::call::VirtualMachineControl;
use crate::vm::call::argument_guard;
use crate::vm::call::built_in_type_parameters;
use crate::vm::call::guard_allows;
use crate::vm::call::unreachable_invariant;

impl VirtualMachine<'_> {
    /// Invokes the already-resolved plain built-in function at a named call site.
    #[inline(always)]
    pub(in crate::vm) fn call_cached_built_in_function_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
    ) -> Option<Result<(), VirtualMachineControl>> {
        let IcDescriptor::Member { type_arguments, .. } =
            // SAFETY: the surrounding invariant keeps this index in bounds.
            (unsafe { chunk.ic_descriptors.get_unchecked(site) })
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a CallNamed site resolves a function name") }
        };
        if type_arguments.is_some() {
            return None;
        }

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &*self.current_frame().cache.as_ref().entries() };
        let CacheEntry::BuiltInCallable(index) = cache.get(site)? else {
            return None;
        };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let BuiltInCallable::Function(spec) = (unsafe {
            self.engine
                .tables
                .built_in_functions
                .get_unchecked(*index as usize)
        }) else {
            return None;
        };
        if !spec.type_parameters.is_empty() {
            return None;
        }
        let spec: FunctionSpec = *spec;

        let outcome = self.invoke_indexed_built_in_function_from_stack(
            BuiltInId(*index),
            spec,
            window_start,
            count,
        );
        self.clear_argument_window(window_start, count);
        Some(outcome.map(|value| {
            let target = self.current_base() + usize::from(destination);
            self.stack[target] = value;
        }))
    }

    /// Resolves and caches the environment of one explicit named turbofish.
    pub(in crate::vm) fn named_call_environment(
        &mut self,
        site: usize,
        target: CallTarget,
        supplied: &[TypeDescriptor],
        outer: TypeEnvironmentId,
    ) -> Result<TypeEnvironmentId, VirtualMachineControl> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *self.current_frame().cache.as_ref().call_environments() };
        if let Some(entry) = cache.get(site).and_then(Option::as_ref)
            && entry.target == target
            && entry.outer == outer
        {
            return Ok(entry.environment);
        }

        let environment = match target {
            CallTarget::User(function) => {
                let (parameters, subject) = {
                    let function = &self.engine.tables.functions[function.0 as usize];
                    (function.type_parameters, function.name.clone())
                };
                self.bind_type_parameters(
                    // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                    unsafe { parameters.as_ref() },
                    Some(supplied),
                    outer,
                    subject.as_bytes(),
                )?
            }
            CallTarget::BuiltIn(function) => {
                let callable = self.engine.tables.built_in_functions[function.0 as usize].clone();
                let parameters = built_in_type_parameters(&self.heap, callable.type_parameters());
                let subject = callable.display_name();
                self.bind_type_parameters(&parameters, Some(supplied), outer, subject.as_bytes())?
            }
        };

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *self.current_frame().cache.as_ref().call_environments() };
        if cache.len() <= site {
            cache.resize(site + 1, None);
        }
        cache[site] = Some(CachedCallEnvironment {
            target,
            outer,
            environment,
        });
        Ok(environment)
    }

    /// Resolves and invokes a checked named call outside the dispatch loop.
    #[inline(never)]
    pub(in crate::vm) fn call_named_site(
        &mut self,
        site: usize,
        chunk: &Chunk,
        destination: u16,
        window_start: usize,
        count: usize,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        let type_arguments = match &chunk.ic_descriptors[site] {
            IcDescriptor::Member { type_arguments, .. } => type_arguments.as_deref(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            IcDescriptor::ClassMember { .. } => unsafe {
                unreachable_invariant("a CallNamed site resolves a function name")
            },
        };
        let entry = self.resolve_call_site(site, chunk)?;
        match entry {
            CacheEntry::Function(function) => {
                let (environment, type_arguments_bound) =
                    if let Some(type_arguments) = type_arguments {
                        let outer = self.current_frame().type_environment;
                        (
                            self.named_call_environment(
                                site,
                                CallTarget::User(function),
                                type_arguments,
                                outer,
                            )?,
                            true,
                        )
                    } else {
                        (TypeEnvironmentId::default(), false)
                    };
                let caller_cache = self.current_frame().cache;
                let arguments_proven = self.cached_argument_guards_match(
                    caller_cache,
                    site,
                    function,
                    environment,
                    window_start..window_start + count,
                    None,
                )?;
                let frame_start = self.stack.len();
                let exact = arguments_proven
                    && (type_arguments_bound
                        || self.engine.tables.functions[function.0 as usize]
                            .type_parameters()
                            .is_empty());

                let outcome = if exact {
                    self.push_exact_generic_function_frame(
                        function,
                        destination,
                        window_start,
                        count,
                        environment,
                        discard_result,
                    )
                } else {
                    self.push_user_frame(
                        function,
                        destination,
                        None,
                        &[],
                        window_start,
                        count,
                        None,
                        None,
                        None,
                        environment,
                        type_arguments_bound,
                        arguments_proven,
                        discard_result,
                        false,
                    )
                };
                if outcome.is_ok() && !arguments_proven {
                    self.cache_argument_guards(
                        caller_cache,
                        site,
                        function,
                        environment,
                        frame_start,
                        count,
                    );
                }
                outcome
            }
            CacheEntry::BuiltInCallable(index) => {
                let callable = self.engine.tables.built_in_functions[index as usize].clone();
                if type_arguments.is_none()
                    && let BuiltInCallable::Function(spec) = &callable
                    && spec.type_parameters.is_empty()
                {
                    let outcome = self.invoke_indexed_built_in_function_from_stack(
                        BuiltInId(index),
                        *spec,
                        window_start,
                        count,
                    );
                    self.clear_argument_window(window_start, count);
                    let value = outcome?;
                    let target = self.current_base() + usize::from(destination);
                    self.stack[target] = value;
                    return Ok(());
                }

                let mut arguments = Vec::with_capacity(count);
                for position in 0..count {
                    arguments.push(self.stack[window_start + position].clone());
                }
                let (environment, type_arguments_bound) =
                    if let Some(type_arguments) = type_arguments {
                        let outer = self.current_frame().type_environment;
                        (
                            self.named_call_environment(
                                site,
                                CallTarget::BuiltIn(BuiltInId(index)),
                                type_arguments,
                                outer,
                            )?,
                            true,
                        )
                    } else {
                        (TypeEnvironmentId::default(), false)
                    };
                let outcome = self.invoke_built_in_callable_called(
                    callable,
                    None,
                    &arguments,
                    None,
                    environment,
                    type_arguments_bound,
                    &[],
                );
                self.clear_argument_window(window_start, count);
                let value = outcome?;
                let target = self.current_base() + usize::from(destination);
                self.stack[target] = value;
                Ok(())
            }
            CacheEntry::Newtype(declaration) => self.construct_newtype_from_stack(
                site,
                declaration,
                type_arguments,
                destination,
                window_start,
                count,
            ),
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("a call site resolves to a callee") },
        }
    }

    fn construct_newtype_from_stack(
        &mut self,
        site: usize,
        id: NewtypeId,
        type_arguments: Option<&[TypeDescriptor]>,
        destination: u16,
        window_start: usize,
        count: usize,
    ) -> Result<(), VirtualMachineControl> {
        if count != 1 {
            let name = self.engine.tables.newtypes[id.0 as usize].name.clone();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.argument_count_error,
                format!(
                    "{} expects exactly 1 argument, {count} provided",
                    name.to_string_lossy()
                ),
            ));
        }

        let outer = self.current_frame().type_environment;
        let value = self.stack[window_start].clone();
        let parent = value.newtype_id();
        let cached = {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let cache = unsafe { &*self.current_frame().cache.as_ref().newtype_constructors() };
            cache
                .get(site)
                .and_then(|ways| ways.get(outer, parent))
                .map(|cached| {
                    let allowed = cached
                        .guard
                        .as_ref()
                        .is_some_and(|guard| guard_allows(guard, &value));
                    let check =
                        (!allowed).then(|| (cached.environment, Rc::clone(&cached.backing)));
                    (cached.tag, check)
                })
        };
        if let Some((tag, check)) = cached {
            if let Some((environment, backing)) = check
                && !self.check_descriptor(backing.as_ref(), &value, None, environment, 0)?
            {
                return Err(self.newtype_construction_type_error(id, &value, backing.as_ref()));
            }

            self.clear_argument_window(window_start, count);
            let tagged = Value::newtype(value, tag);
            let target = self.current_base() + usize::from(destination);
            self.stack[target] = tagged;
            return Ok(());
        }

        let (parameters, backing, name) = {
            let declaration = &self.engine.tables.newtypes[id.0 as usize];
            (
                declaration.type_parameters.clone(),
                declaration.backing.clone(),
                declaration.name.clone(),
            )
        };
        let environment =
            self.bind_type_parameters(&parameters, type_arguments, outer, name.as_bytes())?;
        let backing = Rc::new(self.substitute_descriptor(&backing, environment, 0));
        if !self.check_descriptor(backing.as_ref(), &value, None, environment, 0)? {
            return Err(self.newtype_construction_type_error(id, &value, backing.as_ref()));
        }

        let tag = self
            .engine
            .tables
            .intern_newtype_value(id, environment, parent);
        let entry = CachedNewtypeConstructor {
            outer,
            parent,
            environment,
            guard: argument_guard(backing.as_ref(), &value),
            backing,
            tag,
        };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *self.current_frame().cache.as_ref().newtype_constructors() };
        if cache.len() <= site {
            cache.resize(site + 1, NewtypeConstructorWays::EMPTY);
        }
        cache[site].record(entry);

        self.clear_argument_window(window_start, count);
        let tagged = Value::newtype(value, tag);
        let target = self.current_base() + usize::from(destination);
        self.stack[target] = tagged;
        Ok(())
    }

    fn newtype_construction_type_error(
        &mut self,
        id: NewtypeId,
        value: &Value,
        backing: &TypeDescriptor,
    ) -> VirtualMachineControl {
        let name = self.engine.tables.newtypes[id.0 as usize].name.clone();
        let actual = self.runtime_type_name(value);
        let expected = self.render_descriptor(backing);
        self.throw_well_known(
            self.engine.tables.well_known.type_error,
            format!(
                "{} cannot be constructed from {actual}; expected {expected}",
                name.to_string_lossy()
            ),
        )
    }

    /// Pushes the narrow frame for a named function prelinked when its unit
    /// was declared.
    #[inline(always)]
    pub(in crate::vm) fn call_exact_function_site(
        &mut self,
        site: usize,
        destination: u16,
        window_start: usize,
        count: usize,
    ) -> Result<(), VirtualMachineControl> {
        if let Some(entry) = self.prelinked_built_in_function_site(site) {
            let function = entry.function;
            if let Some(handler) = entry.direct_handler {
                let outcome = self.invoke_prelinked_direct_built_in_function_from_stack(
                    handler,
                    function,
                    window_start,
                    count,
                );
                self.clear_argument_window(window_start, count);
                let value = outcome?;
                let target = self.current_base() + usize::from(destination);
                self.stack[target] = value;
                return Ok(());
            }
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let BuiltInCallable::Function(spec) = (unsafe {
                self.engine
                    .tables
                    .built_in_functions
                    .get_unchecked(function.0 as usize)
            }) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a named built-in symbol is a function") }
            };
            let spec = *spec;
            if !spec.type_parameters.is_empty() {
                let IcDescriptor::Member {
                    type_arguments: Some(type_arguments),
                    ..
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                } = &unsafe { self.current_frame().chunk.as_ref() }.ic_descriptors[site]
                else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe {
                        unreachable_invariant(
                            "an unchecked generic call has explicit type arguments",
                        )
                    }
                };
                let outer = self.current_frame().type_environment;
                let environment = self.named_call_environment(
                    site,
                    CallTarget::BuiltIn(function),
                    type_arguments,
                    outer,
                )?;
                let mut arguments = Vec::with_capacity(count);
                for position in 0..count {
                    arguments.push(self.stack[window_start + position].clone());
                }
                let outcome = self.invoke_built_in_callable_called(
                    BuiltInCallable::Function(spec),
                    None,
                    &arguments,
                    None,
                    environment,
                    true,
                    &[],
                );
                self.clear_argument_window(window_start, count);
                let value = outcome?;
                let target = self.current_base() + usize::from(destination);
                self.stack[target] = value;
                return Ok(());
            }
            let exact_handler = count == spec.parameters.len();
            let outcome = if exact_handler && spec.direct_handler.is_some() {
                self.invoke_direct_built_in_function_from_stack(spec, window_start, count)
            } else {
                self.invoke_proven_built_in_function_from_stack(spec, window_start, count)
            };
            self.clear_argument_window(window_start, count);
            let value = outcome?;
            let target = self.current_base() + usize::from(destination);
            self.stack[target] = value;
            return Ok(());
        }
        let entry = self.prelinked_function_site(site);
        let function = entry.function;
        if entry.has_type_parameters {
            let IcDescriptor::Member {
                type_arguments: Some(type_arguments),
                ..
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            } = &unsafe { self.current_frame().chunk.as_ref() }.ic_descriptors[site]
            else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant("an unchecked generic call has explicit type arguments")
                }
            };
            let outer = self.current_frame().type_environment;
            let environment = self.named_call_environment(
                site,
                CallTarget::User(function),
                type_arguments,
                outer,
            )?;
            return self.push_exact_generic_function_frame(
                function,
                destination,
                window_start,
                count,
                environment,
                false,
            );
        }
        self.push_exact_function_frame(site, entry, destination, window_start, count)
    }

    #[inline(always)]
    pub(in crate::vm) fn prelinked_built_in_function_site(
        &self,
        site: usize,
    ) -> Option<ExactBuiltInFunctionEntry> {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: the surrounding invariant keeps this index in bounds.
        let cache = unsafe { &*cache_cell.exact_built_in_functions() };
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe { cache.get_unchecked(site).as_ref().copied() }
    }

    /// Resolves a `CallNamed` site through its inline cache.
    pub(in crate::vm) fn resolve_call_site(
        &mut self,
        slot: usize,
        chunk: &Chunk,
    ) -> Result<CacheEntry, VirtualMachineControl> {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let cache = unsafe { &mut *cache_cell.entries() };
            if cache.is_empty() {
                cache.resize(chunk.ic_descriptors.len(), CacheEntry::Empty);
            }
            if let entry @ (CacheEntry::Function(_)
            | CacheEntry::BuiltInCallable(_)
            | CacheEntry::Newtype(_)) = &cache[slot]
            {
                return Ok(entry.clone());
            }
        }
        let name = match &chunk.ic_descriptors[slot] {
            IcDescriptor::Member { name, .. } => name.clone(),
            // SAFETY: the surrounding invariant makes this path unreachable.
            IcDescriptor::ClassMember { .. } => unsafe {
                unreachable_invariant("a CallNamed site resolves a member descriptor")
            },
        };
        let entry = self.resolve_named_callable(name)?;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *cache_cell.entries() };
        cache[slot] = entry.clone();
        Ok(entry)
    }

    /// Calls a callable value whose arguments already sit in the caller's
    /// window, writing the result into the caller's destination register.
    pub(in crate::vm) fn call_value_in_place(
        &mut self,
        callee: Value,
        destination: u16,
        window_start: usize,
        count: usize,
        discard_result: bool,
        arguments_proven: bool,
    ) -> Result<(), VirtualMachineControl> {
        if let ValueView::Function(function) = callee.transparent()
            && function.presets().is_empty()
            && let CallTarget::User(id) = function.target()
        {
            let type_arguments_bound = function.type_arguments_bound();
            let type_parameters_empty = self.engine.tables.functions[id.0 as usize]
                .type_parameters()
                .is_empty();
            if arguments_proven
                && function.this().is_none()
                && function.captures().is_empty()
                && function.scope().is_none()
                && (type_arguments_bound || type_parameters_empty)
            {
                return self.push_exact_generic_function_frame(
                    id,
                    destination,
                    window_start,
                    count,
                    function.type_environment(),
                    discard_result,
                );
            }

            return self.push_user_frame(
                id,
                destination,
                function.this().cloned(),
                function.captures(),
                window_start,
                count,
                self.method_context_for(function),
                function.scope(),
                None,
                function.type_environment(),
                function.type_arguments_bound(),
                arguments_proven,
                discard_result,
                false,
            );
        }
        let shape = self.resolve_callee_shape(&callee)?;
        let mut positional = Vec::with_capacity(count);
        for position in 0..count {
            positional.push(self.stack[window_start + position].clone());
        }
        let arguments = self.build_final_arguments(&shape, positional, &[])?;
        let outcome = self.dispatch_shape_in_place(shape, arguments, destination, discard_result);
        self.clear_argument_window(window_start, count);
        outcome
    }

    /// Calls a value whose arguments were proven by type flow, borrowing a
    /// plain function value from the caller register instead of retaining and
    /// releasing its handle on every invocation.
    pub(in crate::vm) fn call_proven_value_site(
        &mut self,
        callee_register: usize,
        destination: u16,
        window_start: usize,
        count: usize,
    ) -> Result<(), VirtualMachineControl> {
        let direct = match self.stack[callee_register].transparent() {
            ValueView::Function(function)
                if function.presets().is_empty()
                    && function.this().is_none()
                    && function.captures().is_empty()
                    && function.scope().is_none() =>
            {
                match function.target() {
                    CallTarget::User(function_id) => {
                        let type_parameters_empty = self.engine.tables.functions
                            [function_id.0 as usize]
                            .type_parameters()
                            .is_empty();
                        (function.type_arguments_bound() || type_parameters_empty)
                            .then_some((function_id, function.type_environment()))
                    }
                    CallTarget::BuiltIn(_) => None,
                }
            }
            _ => None,
        };
        if let Some((function, environment)) = direct {
            return self.push_exact_generic_function_frame(
                function,
                destination,
                window_start,
                count,
                environment,
                false,
            );
        }

        self.call_value_in_place(
            self.stack[callee_register].clone(),
            destination,
            window_start,
            count,
            false,
            true,
        )
    }

    /// Dispatches a `CallWithNames` site: the window carries the positional
    /// values then the named values in descriptor order.
    pub(in crate::vm) fn call_with_names_site(
        &mut self,
        callee: &Value,
        site: usize,
        descriptor: &CallDescriptor,
        window_start: usize,
        destination: u16,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        let shape = self.resolve_callee_shape(callee)?;
        let arguments = self.build_named_arguments(site, &shape, descriptor, window_start)?;
        let count = usize::from(descriptor.positional) + descriptor.named.len();
        let outcome = self.dispatch_shape_in_place(shape, arguments, destination, discard_result);
        self.clear_argument_window(window_start, count);
        outcome
    }
}
