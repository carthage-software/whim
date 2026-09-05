//! Entering user frames and the re-entrant built-in invocation paths.

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::vm::call::Atom;
use crate::vm::call::BuiltInCallable;
use crate::vm::call::CallTarget;
use crate::vm::call::ClassId;
use crate::vm::call::Frame;
use crate::vm::call::FrameFlags;
use crate::vm::call::FuncId;
use crate::vm::call::FunctionObject;
use crate::vm::call::InstanceObject;
use crate::vm::call::ManagedRef;
use crate::vm::call::MethodBodyKind;
use crate::vm::call::MethodContext;
use crate::vm::call::NonNull;
use crate::vm::call::OptionalClassId;
use crate::vm::call::OptionalFuncId;
use crate::vm::call::TypeEnvironmentId;
use crate::vm::call::UserCallContext;
use crate::vm::call::Value;
use crate::vm::call::VirtualMachine;
use crate::vm::call::VirtualMachineControl;
use crate::vm::call::check_trivial_descriptor;
use crate::vm::call::frame_argument_count;
use crate::vm::call::frame_stack_floor_offset;
use crate::vm::call::literal_value;
use crate::vm::call::live_parameter_mask;
use crate::vm::call::unreachable_invariant;

impl VirtualMachine<'_> {
    /// Checks a user call and lays out its receiver, parameters, and captures.
    #[expect(
        clippy::too_many_arguments,
        reason = "the call-shape parameters are one site each"
    )]
    pub(in crate::vm) fn push_user_frame(
        &mut self,
        function: FuncId,
        return_register: u16,
        this: Option<ManagedRef<InstanceObject>>,
        captures: &[Value],
        window_start: usize,
        argc: usize,
        method: Option<MethodContext>,
        declared_scope: Option<ClassId>,
        stack_floor: Option<u32>,
        outer_type_environment: TypeEnvironmentId,
        type_arguments_bound: bool,
        arguments_proven: bool,
        discard_result: bool,
        return_to_host: bool,
    ) -> Result<(), VirtualMachineControl> {
        let frameless = self.engine.tables.functions[function.0 as usize]
            .frameless_literal
            .is_some();
        if !frameless && self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }
        self.engine.optimize_callable_once(function)?;
        let (
            chunk,
            cache,
            unit,
            required,
            declared,
            captures_this,
            reference_parameter_mask,
            type_parameters,
        ) = {
            let runtime = &self.engine.tables.functions[function.0 as usize];
            (
                runtime.chunk,
                NonNull::from(&*runtime.cache),
                NonNull::from(&*runtime.unit),
                usize::from(runtime.required_parameters),
                usize::from(runtime.declared_parameters),
                runtime.captures_this,
                runtime.reference_parameter_mask,
                if type_arguments_bound || runtime.type_parameters().is_empty() {
                    None
                } else {
                    Some(runtime.type_parameters)
                },
            )
        };
        if argc < required {
            let rendered = String::from_utf8_lossy(
                self.engine.tables.functions[function.0 as usize]
                    .name
                    .as_bytes(),
            )
            .into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.argument_count_error,
                format!(
                    "too few arguments to {rendered}: {argc} given, at least {required} expected"
                ),
            ));
        }
        if argc > declared {
            let rendered = String::from_utf8_lossy(
                self.engine.tables.functions[function.0 as usize]
                    .name
                    .as_bytes(),
            )
            .into_owned();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.argument_count_error,
                format!(
                    "too many arguments to {rendered}: {argc} given, at most {declared} accepted"
                ),
            ));
        }
        let type_environment = if let Some(type_parameters) = type_parameters {
            let subject = self.engine.tables.functions[function.0 as usize]
                .name
                .clone();
            self.bind_type_parameters(
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                unsafe { type_parameters.as_ref() },
                None,
                outer_type_environment,
                subject.as_bytes(),
            )?
        } else {
            outer_type_environment
        };
        let called = method
            .map(|context| context.called)
            .or_else(|| this.as_ref().map(|instance| instance.class()));
        let checked_parameters = if arguments_proven { 0 } else { argc };
        for position in 0..checked_parameters {
            let descriptor = {
                let parameter =
                    &self.engine.tables.functions[function.0 as usize].parameters()[position];
                let Some(descriptor) = parameter.declared_type.as_ref() else {
                    continue;
                };
                let value = &self.stack[window_start + position];
                if value.is_uninitialized()
                    || check_trivial_descriptor(descriptor, value) == Some(true)
                {
                    continue;
                }
                NonNull::from(descriptor)
            };
            // SAFETY: parameter storage belongs to the function's retained
            // unit and never moves, even when the check below loads another
            // unit re-entrantly.
            let descriptor = unsafe { descriptor.as_ref() };
            let value = self.stack[window_start + position].clone();
            let valid = match check_trivial_descriptor(descriptor, &value) {
                Some(valid) => valid,
                None => self.check_descriptor_with_array_id(
                    descriptor,
                    &value,
                    called,
                    type_environment,
                    None,
                    0,
                )?,
            };
            if !valid {
                let concrete = self.substitute_descriptor(descriptor, type_environment, 0);
                let expected = self.render_descriptor(&concrete);
                let found = self.value_type_name(&value);
                let function_name = String::from_utf8_lossy(
                    self.engine.tables.functions[function.0 as usize]
                        .name
                        .as_bytes(),
                );
                let parameter_name = String::from_utf8_lossy(
                    self.engine.tables.functions[function.0 as usize].parameters()[position]
                        .name
                        .as_bytes(),
                );
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "argument ${parameter_name} passed to {function_name} must be of type {expected}, {found} given"
                    ),
                ));
            }
        }
        if frameless {
            self.execute_frameless_call(
                function,
                return_register,
                window_start,
                argc,
                stack_floor,
                called,
                type_environment,
                discard_result,
                return_to_host,
            )?;
            return Ok(());
        }
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        let (has_this, this_offset, capture_skip) = if captures_this {
            let Some(leading) = captures.first().filter(|value| value.is_object()) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant(
                        "a this-capturing closure carries the receiver as its first capture",
                    )
                }
            };
            self.stack[base] = leading.clone();
            (true, 1, 1)
        } else if let Some(instance) = this {
            self.stack[base] = Value::object(instance);
            (true, 1, 0)
        } else {
            (false, 0, 0)
        };
        self.reset_omitted_parameters(base, this_offset, argc, declared);
        for position in 0..argc.min(declared) {
            self.stack
                .swap(base + this_offset + position, window_start + position);
        }
        let limit = base + register_count;
        for (position, capture) in captures[capture_skip..].iter().enumerate() {
            let slot = base + this_offset + declared + position;
            if slot >= limit {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    "the compiled unit declares more captures than the frame reserves".to_string(),
                ));
            }
            self.stack[slot] = capture.clone();
        }
        let receiver_class = has_this.then(|| {
            let Some(receiver) = self.stack[base].as_object() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant("a frame with `$this` owns an object in register zero")
                }
            };
            receiver.class()
        });
        let class_scope = match method {
            Some(context) => Some(context.scope),
            None => declared_scope.or(receiver_class),
        };
        let called_class = match method {
            Some(context) => Some(context.called),
            None => receiver_class,
        };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            if this_offset != 0 {
                reference_register_mask |= 1;
            }
            reference_register_mask |= live_parameter_mask(
                &self.stack,
                base,
                this_offset,
                argc.min(declared),
                reference_parameter_mask,
            );
            for position in 0..captures.len().saturating_sub(capture_skip) {
                let register = this_offset + declared + position;
                if self.stack[base + register].is_reference_counted() {
                    reference_register_mask |= 1u64 << register;
                }
            }
        }
        self.snapshot_trace_arguments(base, chunk, argc, &mut reference_register_mask);
        // SAFETY: the call-entry guard reserved a frame within the depth limit.
        unsafe {
            self.push_frame_unchecked(Frame {
                chunk,
                cache,
                unit,
                function: OptionalFuncId::some(function),
                ip: 0,
                base: base as u32,
                argc: frame_argument_count(argc),
                called_class: OptionalClassId::from_option(called_class),
                class_scope: OptionalClassId::from_option(class_scope),
                stack_floor_offset: frame_stack_floor_offset(
                    base,
                    stack_floor.unwrap_or(base as u32),
                ),
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(
                    has_this,
                    false,
                    method.is_some_and(|context| context.is_constructor),
                )
                .with_discard_result(discard_result),
                type_environment,
            });
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the frameless boundary mirrors the user-call boundary"
    )]
    pub(in crate::vm) fn execute_frameless_call(
        &mut self,
        function: FuncId,
        return_register: u16,
        window_start: usize,
        argc: usize,
        stack_floor: Option<u32>,
        called: Option<ClassId>,
        type_environment: TypeEnvironmentId,
        discard_result: bool,
        return_to_host: bool,
    ) -> Result<(), VirtualMachineControl> {
        let literal = self.engine.tables.functions[function.0 as usize]
            .frameless_literal
            .clone();
        let Some(literal) = literal else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a frameless call has a validated literal result") }
        };
        let result = literal_value(&literal);
        if !self.callable_return_value_is_valid(function, &result, called, type_environment)? {
            return Err(self.callable_return_type_mismatch(function, &result, type_environment));
        }

        self.remember_frameless_discarded_result(function, discard_result);
        self.clear_argument_window(window_start, argc);
        if let Some(stack_floor) = stack_floor {
            self.truncate_stack(stack_floor as usize);
        }
        if return_to_host {
            self.push_stack_value(result);
        } else {
            self.store_frameless_result(return_register, result);
        }

        Ok(())
    }

    fn store_frameless_result(&mut self, register: u16, result: Value) {
        let result_is_reference = result.is_reference_counted();
        let frame = self.current_frame_mut();
        let target = frame.base as usize + usize::from(register);
        if result_is_reference
            && register < REFERENCE_REGISTER_LIMIT
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            && unsafe { frame.chunk.as_ref() }.register_count <= REFERENCE_REGISTER_LIMIT
        {
            frame.reference_register_mask |= 1u64 << register;
        }
        self.stack[target] = result;
    }

    /// Calls a user function re-entrantly: the arguments are copied onto the
    /// stack as a synthetic window and released after the call.
    pub(in crate::vm) fn call_user_with_context(
        &mut self,
        call: UserCallContext<'_>,
    ) -> Result<Value, VirtualMachineControl> {
        let UserCallContext {
            function,
            this,
            captures,
            arguments,
            method,
            declared_scope,
            type_environment,
            type_arguments_bound,
        } = call;
        let window_start = self.stack.len();
        for argument in arguments {
            self.push_stack_value(argument.clone());
        }
        let floor = self.frames.len();
        let frameless = self.engine.tables.functions[function.0 as usize]
            .frameless_literal
            .is_some();
        let outcome = self.push_user_frame(
            function,
            0,
            this,
            captures,
            window_start,
            arguments.len(),
            method,
            declared_scope,
            None,
            type_environment,
            type_arguments_bound,
            false,
            false,
            true,
        );
        let result = match outcome {
            Ok(()) if frameless => match self.stack.pop() {
                Some(result) => Ok(result),
                // SAFETY: the surrounding invariant makes this path unreachable.
                None => unsafe {
                    unreachable_invariant("a frameless host call leaves its literal result")
                },
            },
            Ok(()) => self.run(floor),
            Err(control) => Err(control),
        };
        self.truncate_stack(window_start);
        result
    }

    /// The method context a bound `FunctionObject` dispatches under: the
    /// target's declaring class scopes visibility, the receiver's runtime
    /// class binds `static`.
    pub(in crate::vm) fn method_context_for(
        &self,
        function: &ManagedRef<FunctionObject>,
    ) -> Option<MethodContext> {
        let called = function
            .this()
            .map(|instance| instance.class())
            .or_else(|| function.called())?;
        let scope = match function.target() {
            CallTarget::User(id) => self.engine.tables.functions[id.0 as usize]
                .declaring_class
                .unwrap_or(called),
            CallTarget::BuiltIn(_) => function.scope().unwrap_or(called),
        };
        Some(MethodContext {
            scope,
            called,
            is_constructor: false,
        })
    }

    /// Calls any callable value re-entrantly, resolving its shape and
    /// applying its presets.
    pub(in crate::vm) fn call_callee_reentrant(
        &mut self,
        callee: &Value,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let shape = self.resolve_callee_shape(callee)?;
        let final_arguments = self.build_final_arguments(&shape, arguments.to_vec(), &[])?;
        let (type_environment, type_arguments_bound) = self.callee_type_environment(&shape)?;
        match shape.target {
            CallTarget::User(id) => {
                let captures: Vec<Value> = match &shape.holder {
                    Some(holder) => holder.captures().to_vec(),
                    None => Vec::new(),
                };
                self.call_user_with_context(UserCallContext {
                    function: id,
                    this: shape.this.clone(),
                    captures: &captures,
                    arguments: &final_arguments,
                    method: shape.method,
                    declared_scope: shape.holder.as_ref().and_then(|holder| holder.scope()),
                    type_environment,
                    type_arguments_bound,
                })
            }
            CallTarget::BuiltIn(id) => {
                let callable = self.engine.tables.built_in_functions[id.0 as usize].clone();
                let called_class = shape.method.map(|method| method.called);
                self.invoke_built_in_callable_called(
                    callable,
                    shape.this.as_ref(),
                    &final_arguments,
                    called_class,
                    type_environment,
                    type_arguments_bound,
                    shape
                        .holder
                        .as_ref()
                        .map_or(&[], |holder| holder.captures()),
                )
            }
        }
    }

    /// Dispatches a method on an instance, as `$receiver->name(...)` would.
    pub(in crate::vm) fn invoke_method(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        name: Atom,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let entry = self.engine.tables.classes[receiver.class().0 as usize].method(&name);
        let Some(entry) = entry else {
            if name == self.engine.tables.constructor_name {
                if arguments.is_empty() {
                    return Ok(Value::null());
                }
                let class_name = self.engine.tables.classes[receiver.class().0 as usize]
                    .name
                    .clone();
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.argument_count_error,
                    format!(
                        "{} has no constructor, {} arguments given",
                        class_name.to_string_lossy(),
                        arguments.len()
                    ),
                ));
            }
            let class_name = self.engine.tables.classes[receiver.class().0 as usize]
                .name
                .clone();
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!(
                    "call to undefined method {}::{}",
                    class_name.to_string_lossy(),
                    name.to_string_lossy()
                ),
            ));
        };
        let type_environment = self
            .environment_for_class(
                receiver.class(),
                receiver.type_environment(),
                entry.declaring_class,
                0,
            )?
            .unwrap_or_else(TypeEnvironmentId::default);
        match entry.body {
            MethodBodyKind::Bytecode(function) => self.call_user_with_context(UserCallContext {
                function,
                this: Some(receiver.clone()),
                captures: &[],
                arguments,
                method: Some(MethodContext {
                    scope: entry.declaring_class,
                    called: receiver.class(),
                    is_constructor: name == self.engine.tables.constructor_name,
                }),
                declared_scope: None,
                type_environment,
                type_arguments_bound: false,
            }),
            MethodBodyKind::BuiltIn(body) => self.invoke_built_in_callable_called(
                BuiltInCallable::Method { body, name },
                Some(receiver),
                arguments,
                Some(receiver.class()),
                type_environment,
                false,
                &[],
            ),
        }
    }
}
