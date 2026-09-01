//! Resolving a callee's dispatch shape and materializing the final
//! argument window, including bound-callable partials.

use std::mem;
use std::rc::Rc;

use crate::symbols::CachedNamedArguments;
use crate::value::ValueView;
use crate::vm::call::ArgumentSlot;
use crate::vm::call::Atom;
use crate::vm::call::BuiltInCallable;
use crate::vm::call::BuiltInId;
use crate::vm::call::CacheEntry;
use crate::vm::call::CachedBoundCallable;
use crate::vm::call::CallDescriptor;
use crate::vm::call::CallTarget;
use crate::vm::call::CalleeShape;
use crate::vm::call::ClassId;
use crate::vm::call::FunctionObject;
use crate::vm::call::ManagedRef;
use crate::vm::call::MethodBodyKind;
use crate::vm::call::MethodContext;
use crate::vm::call::PresetArg;
use crate::vm::call::PresetDescriptor;
use crate::vm::call::PresetSlot;
use crate::vm::call::TypeEnvironmentId;
use crate::vm::call::Value;
use crate::vm::call::VirtualMachine;
use crate::vm::call::VirtualMachineControl;
use crate::vm::call::built_in_type_parameters;
use crate::vm::call::find_double_colon;
use crate::vm::call::reduce_signature;
use crate::vm::call::unreachable_invariant;
use crate::vm::call::visibility_allows;
use crate::vm::call::visibility_name;

impl VirtualMachine<'_> {
    /// The resolved callee of a value: a function value keeps its target,
    /// receiver, captures, and presets; a `'name'` string resolves a
    /// function; a `'Class::method'` string resolves a static method; a
    /// `(receiver, 'method')` tuple resolves an instance method.
    pub(in crate::vm) fn resolve_callee_shape(
        &mut self,
        callee: &Value,
    ) -> Result<CalleeShape, VirtualMachineControl> {
        match callee.transparent() {
            ValueView::Function(function) => Ok(CalleeShape {
                target: function.target(),
                this: function.this().cloned(),
                holder: Some(function.clone()),
                method: self.method_context_for(function),
            }),
            ValueView::String(_) | ValueView::ShortString(_) => {
                // SAFETY: the value's tag proves this projection is valid.
                let bytes = unsafe { callee.as_string_bytes().unwrap_unchecked() };
                let bytes = match bytes.first() {
                    Some(b'\\') => &bytes[1..],
                    _ => bytes,
                };
                if let Some(split) = find_double_colon(bytes) {
                    let class_atom = self.heap.intern(&bytes[..split]);
                    let member = self.heap.intern(&bytes[split + 2..]);
                    let class = self.resolve_class_reference(class_atom)?;
                    return self.static_method_shape(class, member);
                }
                let atom = self.heap.intern(bytes);
                let target = match self.resolve_function(atom)? {
                    CacheEntry::Function(id) => CallTarget::User(id),
                    CacheEntry::BuiltInCallable(index) => CallTarget::BuiltIn(BuiltInId(index)),
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    _ => unsafe { unreachable_invariant("function resolution yields a callee") },
                };
                Ok(CalleeShape {
                    target,
                    this: None,
                    holder: None,
                    method: None,
                })
            }
            ValueView::Tuple(pair) => {
                let receiver = pair.get(0).and_then(Value::as_object).cloned();
                let member = pair.get(1).and_then(Value::as_string_bytes);
                let (Some(receiver), Some(member)) = (receiver, member) else {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.type_error,
                        "a callable tuple pairs a receiver with a method name".to_string(),
                    ));
                };
                if pair.len() != 2 {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.type_error,
                        "a callable tuple pairs a receiver with a method name".to_string(),
                    ));
                }
                let name = self.heap.intern(member);
                let class = &self.engine.tables.classes[receiver.class().0 as usize];
                let entry = self
                    .current_frame()
                    .class_scope
                    .get()
                    .and_then(|scope| class.private_methods.get(&(scope, name.clone())).copied())
                    .or_else(|| class.method(&name));
                let Some(entry) = entry else {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.type_error,
                        format!(
                            "call to undefined method {}::{}",
                            self.value_type_name(&Value::object(receiver)),
                            name.to_string_lossy()
                        ),
                    ));
                };
                if !visibility_allows(
                    &self.engine.tables.classes,
                    entry.visibility,
                    entry.declaring_class,
                    self.current_frame().class_scope.get(),
                ) {
                    let visibility = visibility_name(entry.visibility);
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.visibility_error,
                        format!(
                            "cannot reference {visibility} method {}",
                            name.to_string_lossy()
                        ),
                    ));
                }
                let this = if entry.is_static {
                    None
                } else {
                    Some(receiver.clone())
                };
                let is_constructor = name == self.engine.tables.constructor_name;
                let target = match entry.body {
                    MethodBodyKind::Bytecode(function) => CallTarget::User(function),
                    MethodBodyKind::BuiltIn(_) => {
                        CallTarget::BuiltIn(self.built_in_id_for_method(&entry, name))
                    }
                };
                Ok(CalleeShape {
                    target,
                    this,
                    holder: None,
                    method: Some(MethodContext {
                        scope: entry.declaring_class,
                        called: receiver.class(),
                        is_constructor,
                    }),
                })
            }
            other => Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("a {} value is not callable", other.kind_name()),
            )),
        }
    }

    /// The generic environment a resolved callable starts from. Closures and
    /// specialized values carry their own environment; instance methods view
    /// the receiver through the class that actually declared the method.
    pub(in crate::vm) fn callee_type_environment(
        &mut self,
        shape: &CalleeShape,
    ) -> Result<(TypeEnvironmentId, bool), VirtualMachineControl> {
        if let Some(holder) = &shape.holder {
            return Ok((holder.type_environment(), holder.type_arguments_bound()));
        }
        let Some(this) = &shape.this else {
            return Ok((TypeEnvironmentId::default(), false));
        };
        let mut environment = this.type_environment();
        let declaring = shape.method.map(|method| method.scope).or_else(|| {
            let CallTarget::User(function) = shape.target else {
                return None;
            };
            self.engine.tables.functions[function.0 as usize].declaring_class
        });
        if let Some(declaring) = declaring {
            environment = self
                .environment_for_class(this.class(), environment, declaring, 0)?
                .unwrap_or_else(TypeEnvironmentId::default);
        }
        Ok((environment, false))
    }

    pub(in crate::vm) fn static_method_shape(
        &mut self,
        class: ClassId,
        member: Atom,
    ) -> Result<CalleeShape, VirtualMachineControl> {
        let runtime = &self.engine.tables.classes[class.0 as usize];
        let entry = self
            .current_frame()
            .class_scope
            .get()
            .and_then(|scope| {
                runtime
                    .private_methods
                    .get(&(scope, member.clone()))
                    .copied()
            })
            .or_else(|| runtime.method(&member));
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
        if !entry.is_static {
            let member_text = member.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!("cannot reference the instance method {member_text} statically"),
            ));
        }
        if !visibility_allows(
            &self.engine.tables.classes,
            entry.visibility,
            entry.declaring_class,
            self.current_frame().class_scope.get(),
        ) {
            let visibility = visibility_name(entry.visibility);
            let member_text = member.to_string_lossy().into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.visibility_error,
                format!("cannot reference {visibility} method {member_text}"),
            ));
        }
        let target = match entry.body {
            MethodBodyKind::Bytecode(function) => CallTarget::User(function),
            MethodBodyKind::BuiltIn(_) => {
                CallTarget::BuiltIn(self.built_in_id_for_method(&entry, member))
            }
        };
        Ok(CalleeShape {
            target,
            this: None,
            holder: None,
            method: Some(MethodContext {
                scope: entry.declaring_class,
                called: class,
                is_constructor: false,
            }),
        })
    }

    pub(in crate::vm) fn parameter_position(
        &self,
        target: CallTarget,
        name: Atom,
    ) -> Option<usize> {
        match target {
            CallTarget::User(id) => self.engine.tables.functions[id.0 as usize]
                .parameters()
                .iter()
                .position(|parameter| parameter.name == name),
            CallTarget::BuiltIn(id) => self.engine.tables.built_in_functions[id.0 as usize]
                .parameters()
                .iter()
                .position(|parameter| parameter.name.as_bytes() == name.as_bytes()),
        }
    }

    fn parameter_count(&self, target: CallTarget) -> usize {
        match target {
            CallTarget::User(id) => self.engine.tables.functions[id.0 as usize]
                .parameters()
                .len(),
            CallTarget::BuiltIn(id) => self.engine.tables.built_in_functions[id.0 as usize]
                .parameters()
                .len(),
        }
    }

    fn parameter_is_optional(&self, target: CallTarget, position: usize) -> bool {
        match target {
            CallTarget::User(id) => self.engine.tables.functions[id.0 as usize]
                .parameters()
                .get(position)
                .is_some_and(|parameter| parameter.has_default),
            CallTarget::BuiltIn(id) => self.engine.tables.built_in_functions[id.0 as usize]
                .parameters()
                .get(position)
                .is_some_and(|parameter| parameter.optional),
        }
    }

    fn parameter_name(&self, target: CallTarget, position: usize) -> String {
        match target {
            CallTarget::User(id) => self.engine.tables.functions[id.0 as usize].parameters()
                [position]
                .name
                .to_string(),
            CallTarget::BuiltIn(id) => self.engine.tables.built_in_functions[id.0 as usize]
                .parameters()[position]
                .name
                .to_string(),
        }
    }

    fn callable_name(&self, target: CallTarget) -> String {
        match target {
            CallTarget::User(id) => self.engine.tables.functions[id.0 as usize]
                .name
                .to_string_lossy()
                .into_owned(),
            CallTarget::BuiltIn(id) => {
                self.engine.tables.built_in_functions[id.0 as usize].display_name()
            }
        }
    }

    /// Builds the final positional argument vector of a call: the callee's
    /// presets first, call-time positional arguments filling explicit holes
    /// left to right, and named arguments landing only on open positions.
    /// A position bound twice, an argument beyond the partial's holes, or a
    /// name matching no open parameter is an `ArgumentCountError`.
    pub(in crate::vm) fn build_final_arguments(
        &mut self,
        shape: &CalleeShape,
        positional: Vec<Value>,
        named: &[(Atom, Value)],
    ) -> Result<Vec<Value>, VirtualMachineControl> {
        let mut slots: Vec<ArgumentSlot> = match &shape.holder {
            Some(holder) => holder
                .presets()
                .iter()
                .map(|preset| match preset {
                    PresetArg::Hole(order) => ArgumentSlot::Hole(*order),
                    PresetArg::Given(value) => {
                        if value.is_uninitialized() {
                            ArgumentSlot::Gap
                        } else {
                            ArgumentSlot::Filled(value.clone())
                        }
                    }
                })
                .collect(),
            None => Vec::new(),
        };
        let has_presets = shape
            .holder
            .as_ref()
            .is_some_and(|holder| !holder.presets().is_empty());
        for value in positional {
            let open = slots
                .iter()
                .enumerate()
                .filter_map(|(position, slot)| match slot {
                    ArgumentSlot::Hole(order) => Some((position, *order)),
                    _ => None,
                })
                .min_by_key(|(_, order)| *order)
                .map(|(position, _)| position);
            match open {
                Some(position) => slots[position] = ArgumentSlot::Filled(value),
                None if !has_presets => slots.push(ArgumentSlot::Filled(value)),
                None => {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.argument_count_error,
                        "the partial application received too many arguments".to_string(),
                    ));
                }
            }
        }
        for (name, value) in named {
            let Some(position) = self.parameter_position(shape.target, name.clone()) else {
                let rendered = name.to_string_lossy().into_owned();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.argument_count_error,
                    format!("no parameter matches the named argument {rendered}"),
                ));
            };
            let slot = slots.get(position);
            match slot {
                Some(ArgumentSlot::Filled(_)) => {
                    let rendered = name.to_string_lossy().into_owned();
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.argument_count_error,
                        format!("the parameter {rendered} is passed twice"),
                    ));
                }
                Some(ArgumentSlot::Hole(_)) => {
                    slots[position] = ArgumentSlot::Filled(value.clone());
                }
                Some(ArgumentSlot::Gap) | None if !has_presets => {
                    while slots.len() <= position {
                        slots.push(ArgumentSlot::Gap);
                    }
                    slots[position] = ArgumentSlot::Filled(value.clone());
                }
                Some(ArgumentSlot::Gap) | None => {
                    let rendered = name.to_string_lossy().into_owned();
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.argument_count_error,
                        format!("the partial application has no open parameter named {rendered}"),
                    ));
                }
            }
        }
        let mut arguments: Vec<Value> = slots
            .into_iter()
            .map(|slot| match slot {
                ArgumentSlot::Filled(value) => value,
                ArgumentSlot::Hole(_) | ArgumentSlot::Gap => Value::uninitialized(),
            })
            .collect();
        while arguments.last().is_some_and(Value::is_uninitialized) {
            arguments.pop();
        }
        let parameter_count = self.parameter_count(shape.target);
        for (position, argument) in arguments.iter().enumerate() {
            if argument.is_uninitialized()
                && position < parameter_count
                && !self.parameter_is_optional(shape.target, position)
            {
                let parameter = self.parameter_name(shape.target, position);
                let function = self.callable_name(shape.target);
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.argument_count_error,
                    format!("the required parameter ${parameter} of {function} is not given"),
                ));
            }
        }
        Ok(arguments)
    }

    pub(in crate::vm) fn build_named_arguments(
        &mut self,
        site: usize,
        shape: &CalleeShape,
        descriptor: &CallDescriptor,
        window_start: usize,
    ) -> Result<Vec<Value>, VirtualMachineControl> {
        if shape
            .holder
            .as_ref()
            .is_some_and(|holder| !holder.presets().is_empty())
        {
            let positional = (0..usize::from(descriptor.positional))
                .map(|position| self.stack[window_start + position].clone())
                .collect();
            let named: Vec<_> = descriptor
                .named
                .iter()
                .enumerate()
                .map(|(offset, name)| {
                    let position = window_start + usize::from(descriptor.positional) + offset;
                    (name.clone(), self.stack[position].clone())
                })
                .collect();

            return self.build_final_arguments(shape, positional, &named);
        }

        let mapping = self.named_argument_mapping(site, shape.target, descriptor)?;
        let mut arguments = vec![Value::uninitialized(); mapping.final_count];
        let positional = usize::from(descriptor.positional);
        for (position, argument) in arguments.iter_mut().take(positional).enumerate() {
            *argument = mem::replace(
                &mut self.stack[window_start + position],
                Value::uninitialized(),
            );
        }

        for (offset, position) in mapping.positions.iter().copied().enumerate() {
            arguments[position] = mem::replace(
                &mut self.stack[window_start + positional + offset],
                Value::uninitialized(),
            );
        }

        Ok(arguments)
    }

    fn named_argument_mapping(
        &mut self,
        site: usize,
        target: CallTarget,
        descriptor: &CallDescriptor,
    ) -> Result<CachedNamedArguments, VirtualMachineControl> {
        // SAFETY: the active VM exclusively owns the current frame's cache.
        let cache = unsafe { &*self.current_frame().cache.as_ref().named_arguments() };
        if let Some(mapping) = cache.get(site).and_then(|ways| ways.get(target)) {
            return Ok(mapping);
        }

        let positional = usize::from(descriptor.positional);
        let mut occupied = vec![true; positional];
        let mut positions = Vec::with_capacity(descriptor.named.len());
        for name in &descriptor.named {
            let Some(position) = self.parameter_position(target, name.clone()) else {
                let rendered = name.to_string_lossy().into_owned();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.argument_count_error,
                    format!("no parameter matches the named argument {rendered}"),
                ));
            };

            if occupied.get(position).copied().unwrap_or(false) {
                let rendered = name.to_string_lossy().into_owned();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.argument_count_error,
                    format!("the parameter {rendered} is passed twice"),
                ));
            }

            if occupied.len() <= position {
                occupied.resize(position + 1, false);
            }

            occupied[position] = true;
            positions.push(position);
        }

        while occupied.last() == Some(&false) {
            occupied.pop();
        }

        let final_count = occupied.len();
        let parameter_count = self.parameter_count(target);
        for (position, occupied) in occupied.into_iter().enumerate() {
            if !occupied
                && position < parameter_count
                && !self.parameter_is_optional(target, position)
            {
                let parameter = self.parameter_name(target, position);
                let function = self.callable_name(target);
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.argument_count_error,
                    format!("the required parameter ${parameter} of {function} is not given"),
                ));
            }
        }

        let mapping = CachedNamedArguments {
            target,
            positions: Rc::from(positions),
            final_count,
        };

        // SAFETY: the active VM exclusively owns the current frame's cache.
        let cache = unsafe { &mut *self.current_frame().cache.as_ref().named_arguments() };
        if cache.len() <= site {
            cache.resize_with(site + 1, Default::default);
        }

        cache[site].record(mapping.clone());

        Ok(mapping)
    }

    /// Dispatches a resolved callee over final arguments, writing the result
    /// into the caller's destination register; user targets push a frame
    /// over a synthetic window that pops with it.
    pub(in crate::vm) fn dispatch_shape_in_place(
        &mut self,
        shape: CalleeShape,
        arguments: Vec<Value>,
        destination: u16,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        let (type_environment, type_arguments_bound) = self.callee_type_environment(&shape)?;
        match shape.target {
            CallTarget::User(id) => {
                let window_start = self.stack.len();
                let argc = arguments.len();
                for argument in arguments {
                    self.push_stack_value(argument);
                }
                let captures: &[Value] = match &shape.holder {
                    Some(holder) => holder.captures(),
                    None => &[],
                };
                self.push_user_frame(
                    id,
                    destination,
                    shape.this.clone(),
                    captures,
                    window_start,
                    argc,
                    shape.method,
                    shape.holder.as_ref().and_then(|holder| holder.scope()),
                    Some(window_start as u32),
                    type_environment,
                    type_arguments_bound,
                    false,
                    discard_result,
                    false,
                )
            }
            CallTarget::BuiltIn(id) => {
                let callable = self.engine.tables.built_in_functions[id.0 as usize].clone();
                let called_class = shape.method.map(|method| method.called);
                let value = self.invoke_built_in_callable_called(
                    callable,
                    shape.this.as_ref(),
                    &arguments,
                    called_class,
                    type_environment,
                    type_arguments_bound,
                    shape
                        .holder
                        .as_ref()
                        .map_or(&[], |holder| holder.captures()),
                )?;
                let target = self.current_frame().base as usize + usize::from(destination);
                self.stack[target] = value;
                Ok(())
            }
        }
    }

    pub(in crate::vm) fn cached_bound_callable(
        &self,
        site: usize,
        target: CallTarget,
        argument_environment: TypeEnvironmentId,
    ) -> Option<ManagedRef<FunctionObject>> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &*self.current_frame().cache.as_ref().bound_callables() };
        let entry = cache.get(site)?.as_ref()?;
        (entry.target == target && entry.argument_environment == argument_environment)
            .then(|| entry.callable.clone())
    }

    pub(in crate::vm) fn cache_bound_callable(
        &mut self,
        site: usize,
        target: CallTarget,
        argument_environment: TypeEnvironmentId,
        callable: ManagedRef<FunctionObject>,
    ) {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let cache = unsafe { &mut *self.current_frame().cache.as_ref().bound_callables() };
        if cache.len() <= site {
            cache.resize_with(site + 1, || None);
        }
        cache[site] = Some(CachedBoundCallable {
            target,
            argument_environment,
            callable,
        });
    }

    /// Builds a bound callable: the callee value's target, receiver, and
    /// captures carry over, and the preset descriptor's slots normalize onto
    /// the target's parameter positions (an empty descriptor is a pure
    /// binding). A skipped middle position is stored as a given
    /// uninitialized value, the internal gap marker no program can observe.
    pub(in crate::vm) fn bind_partial(
        &mut self,
        callee: &Value,
        descriptor: &PresetDescriptor,
        site: usize,
        window_start: usize,
    ) -> Result<Value, VirtualMachineControl> {
        let argument_environment = self.current_frame().type_environment;
        let shape = self.resolve_callee_shape(callee)?;
        let slots = &descriptor.slots;
        let cacheable = slots.is_empty()
            && !descriptor.open_remaining
            && descriptor.type_arguments.is_some()
            && shape.this.is_none()
            && shape.holder.is_none();
        if cacheable
            && let Some(callable) =
                self.cached_bound_callable(site, shape.target, argument_environment)
        {
            return Ok(Value::function(callable));
        }
        let mut merged: Vec<ArgumentSlot> = match &shape.holder {
            Some(holder) => holder
                .presets()
                .iter()
                .map(|preset| match preset {
                    PresetArg::Hole(order) => ArgumentSlot::Hole(*order),
                    PresetArg::Given(value) => {
                        if value.is_uninitialized() {
                            ArgumentSlot::Gap
                        } else {
                            ArgumentSlot::Filled(value.clone())
                        }
                    }
                })
                .collect(),
            None => Vec::new(),
        };
        let mut window_cursor = window_start;
        let is_application = !slots.is_empty() || descriptor.open_remaining;
        if is_application {
            let parameter_count = self.parameter_count(shape.target);
            merged.resize_with(parameter_count, || ArgumentSlot::Gap);
            let mut available: Vec<(u32, usize)> = match &shape.holder {
                Some(holder) if !holder.presets().is_empty() => holder
                    .presets()
                    .iter()
                    .enumerate()
                    .filter_map(|(position, preset)| match preset {
                        PresetArg::Hole(order) => Some((*order, position)),
                        PresetArg::Given(_) => None,
                    })
                    .collect(),
                _ => (0..parameter_count)
                    .map(|position| (position as u32, position))
                    .collect(),
            };
            available.sort_unstable_by_key(|(order, _)| *order);
            let available: Vec<usize> = available
                .into_iter()
                .map(|(_, position)| position)
                .collect();
            for position in &available {
                merged[*position] = ArgumentSlot::Gap;
            }

            let reserved_named: Vec<usize> = slots
                .iter()
                .filter_map(|slot| match slot {
                    PresetSlot::GivenNamed(name) | PresetSlot::HoleNamed(name) => {
                        self.parameter_position(shape.target, name.clone())
                    }
                    _ => None,
                })
                .collect();
            let mut used = vec![false; parameter_count];
            let mut open_cursor = 0;
            let mut next_hole_order = 0;
            for slot in slots {
                let position = match slot {
                    PresetSlot::GivenPositional | PresetSlot::HolePositional => loop {
                        let Some(position) = available.get(open_cursor).copied() else {
                            return Err(self.throw_well_known(
                                self.engine.tables.well_known.argument_count_error,
                                "the partial application passes too many arguments".to_string(),
                            ));
                        };
                        open_cursor += 1;
                        if !reserved_named.contains(&position) && !used[position] {
                            break position;
                        }
                    },
                    PresetSlot::GivenNamed(name) | PresetSlot::HoleNamed(name) => {
                        let Some(position) = self.parameter_position(shape.target, name.clone())
                        else {
                            let rendered = name.to_string_lossy().into_owned();
                            return Err(self.throw_well_known(
                                self.engine.tables.well_known.argument_count_error,
                                format!("no parameter matches the named argument {rendered}"),
                            ));
                        };
                        if !available.contains(&position) {
                            let rendered = name.to_string_lossy().into_owned();
                            return Err(self.throw_well_known(
                                self.engine.tables.well_known.argument_count_error,
                                format!(
                                    "the partial application has no open parameter named {rendered}"
                                ),
                            ));
                        }
                        if used[position] {
                            let rendered = name.to_string_lossy().into_owned();
                            return Err(self.throw_well_known(
                                self.engine.tables.well_known.argument_count_error,
                                format!("the parameter {rendered} is passed twice"),
                            ));
                        }
                        position
                    }
                };
                used[position] = true;
                if matches!(
                    slot,
                    PresetSlot::GivenPositional | PresetSlot::GivenNamed(_)
                ) {
                    merged[position] = ArgumentSlot::Filled(self.stack[window_cursor].clone());
                    window_cursor += 1;
                } else {
                    merged[position] = ArgumentSlot::Hole(next_hole_order);
                    next_hole_order += 1;
                }
            }

            for position in available {
                if used[position] {
                    continue;
                }
                if descriptor.open_remaining {
                    merged[position] = ArgumentSlot::Hole(next_hole_order);
                    next_hole_order += 1;
                } else if !self.parameter_is_optional(shape.target, position) {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.argument_count_error,
                        "the partial application does not cover every required parameter"
                            .to_string(),
                    ));
                }
            }
        }
        let presets: Vec<PresetArg> = merged
            .into_iter()
            .map(|slot| match slot {
                ArgumentSlot::Filled(value) => PresetArg::Given(value),
                ArgumentSlot::Hole(order) => PresetArg::Hole(order),
                ArgumentSlot::Gap => PresetArg::Given(Value::uninitialized()),
            })
            .collect();
        let full_signature = match shape.target {
            CallTarget::User(id) => self.engine.tables.functions[id.0 as usize]
                .signature
                .clone(),
            CallTarget::BuiltIn(id) => match self.engine.tables.built_in_functions[id.0 as usize] {
                BuiltInCallable::Function(spec) => self.heap.intern(spec.signature.as_bytes()),
                BuiltInCallable::Method { body, .. } => self.heap.intern(body.signature.as_bytes()),
            },
        };
        let mut visible: Vec<(u32, usize)> = presets
            .iter()
            .enumerate()
            .filter_map(|(position, preset)| match preset {
                PresetArg::Hole(order) => Some((*order, position)),
                PresetArg::Given(_) => None,
            })
            .collect();
        visible.sort_unstable_by_key(|(order, _)| *order);
        let visible: Vec<usize> = visible.into_iter().map(|(_, position)| position).collect();
        let signature = if presets.is_empty() {
            full_signature
        } else {
            self.heap
                .intern(reduce_signature(full_signature.as_bytes(), &visible).as_bytes())
        };
        let captures: Vec<Value> = match &shape.holder {
            Some(holder) => holder.captures().to_vec(),
            None => Vec::new(),
        };
        let scope = shape
            .holder
            .as_ref()
            .and_then(|holder| holder.scope())
            .or_else(|| shape.method.map(|method| method.scope));
        let (outer_environment, already_bound) = self.callee_type_environment(&shape)?;
        let (type_environment, type_arguments_bound) = match &descriptor.type_arguments {
            Some(arguments) => {
                if already_bound {
                    return Err(self.throw_well_known(
                        self.engine.tables.well_known.type_error,
                        "the callable is already specialized and takes no type arguments"
                            .to_string(),
                    ));
                }
                let type_environment = match shape.target {
                    CallTarget::User(id) => {
                        let (parameters, subject) = {
                            let function = &self.engine.tables.functions[id.0 as usize];
                            (function.type_parameters, function.name.clone())
                        };
                        self.bind_type_parameters_from(
                            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                            unsafe { parameters.as_ref() },
                            Some(arguments),
                            argument_environment,
                            outer_environment,
                            subject.as_bytes(),
                        )?
                    }
                    CallTarget::BuiltIn(id) => {
                        let callable = self.engine.tables.built_in_functions[id.0 as usize].clone();
                        let parameters =
                            built_in_type_parameters(&self.heap, callable.type_parameters());
                        let subject = callable.display_name();
                        self.bind_type_parameters_from(
                            &parameters,
                            Some(arguments),
                            argument_environment,
                            outer_environment,
                            subject.as_bytes(),
                        )?
                    }
                };
                (type_environment, true)
            }
            None => (outer_environment, already_bound),
        };
        let target = shape.target;
        let called_class = shape.method.map(|method| method.called);
        let callable = FunctionObject::partial(
            &self.heap,
            target,
            shape.this,
            captures,
            presets,
            signature,
            scope,
            called_class,
            type_environment,
            type_arguments_bound,
        );
        if cacheable {
            self.cache_bound_callable(site, target, argument_environment, callable.clone());
        }
        Ok(Value::function(callable))
    }
}
