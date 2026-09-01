//! Calling a built-in handler and checking the arguments it receives.

use std::array;
use std::mem::ManuallyDrop;
use std::panic;
use std::ptr;
use std::slice;

use crate::builtin::Context;
use crate::builtin::spec::BuiltInDirectHandler;
use crate::builtin::spec::BuiltInHandler;
use crate::builtin::spec::FunctionSpec;
use crate::builtin::spec::ParameterSpec;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::bytecode::unit::CompiledParameter;
use crate::classes::BuiltInMethodBody;
use crate::classes::MethodEntry;
use crate::engine::builtins::BuiltInCallable;
use crate::engine::builtins::built_in_type_parameters;
use crate::linker::descriptors::descriptor_from_built_in_spec;
use crate::value::Value;
use crate::value::function::BuiltInId;
use crate::vm::Atom;
use crate::vm::ClassId;
use crate::vm::InstanceObject;
use crate::vm::ManagedRef;
use crate::vm::MethodBodyKind;
use crate::vm::TypeDescriptor;
use crate::vm::TypeEnvironmentId;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::unreachable_invariant;

/// Metadata retained across a built-in invocation without eagerly rendering a
/// trace name. The owned method atom is a cheap shared handle; the string is
/// only built when the invocation actually throws.
enum BuiltInTraceMetadata {
    Function(FunctionSpec),
    Method {
        body: BuiltInMethodBody,
        name: Atom,
        called_class: Option<ClassId>,
    },
}

/// The signature of a partial application, retaining target parameter
/// positions in the order exposed by its holes.
pub(in crate::vm) fn reduce_signature(signature: &[u8], positions: &[usize]) -> String {
    let text = String::from_utf8_lossy(signature).into_owned();
    let Some(open) = text.find('(') else {
        return text;
    };
    let mut depth = 0;
    let mut close = text.len();
    let mut parameters = Vec::new();
    let mut start = open + 1;
    for (position, character) in text.char_indices().skip(open) {
        match character {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => {
                depth -= 1;
                if depth == 0 {
                    if start < position {
                        parameters.push(text[start..position].trim().to_string());
                    }
                    close = position;
                    break;
                }
            }
            ',' if depth == 1 => {
                parameters.push(text[start..position].trim().to_string());
                start = position + 1;
            }
            _ => {}
        }
    }
    let kept: Vec<String> = positions
        .iter()
        .filter_map(|position| parameters.get(*position).cloned())
        .collect();
    format!("fn({}){}", kept.join(", "), &text[close + 1..])
}

impl VirtualMachine<'_> {
    fn render_built_in_type_spec(&self, spec: &TypeSpec) -> String {
        self.render_descriptor(&descriptor_from_built_in_spec(&self.heap, spec))
    }

    #[inline(always)]
    fn trivial_built_in_type_spec_accepts(spec: &TypeSpec, value: &Value) -> Option<bool> {
        Some(match spec {
            TypeSpec::Wildcard | TypeSpec::Mixed => true,
            TypeSpec::Void | TypeSpec::Null => value.is_null(),
            TypeSpec::Never => false,
            TypeSpec::Bool => value.is_bool(),
            TypeSpec::Int => value.is_int(),
            TypeSpec::IntRange(min, max) => value.as_int().is_some_and(|value| {
                min.is_none_or(|min| value >= min) && max.is_none_or(|max| value <= max)
            }),
            TypeSpec::Float => value.is_float(),
            TypeSpec::String => value.is_string(),
            TypeSpec::StringLiteral(expected) => value
                .as_string_bytes()
                .is_some_and(|value| value == *expected),
            TypeSpec::Array => value.is_vec() || value.is_dict() || value.is_tuple(),
            TypeSpec::Vec => value.is_vec(),
            TypeSpec::Dict => value.is_dict(),
            TypeSpec::Tuple => value.is_tuple(),
            TypeSpec::Function => value.is_function(),
            TypeSpec::Object => value.is_object(),
            _ => return None,
        })
    }

    fn built_in_type_spec_accepts(
        &mut self,
        spec: &TypeSpec,
        value: &Value,
        called_class: Option<ClassId>,
        environment: TypeEnvironmentId,
    ) -> Result<bool, VirtualMachineControl> {
        if let Some(accepts) = Self::trivial_built_in_type_spec_accepts(spec, value) {
            return Ok(accepts);
        }
        let descriptor = descriptor_from_built_in_spec(&self.heap, spec);
        self.check_descriptor(&descriptor, value, called_class, environment, 0)
    }

    pub(crate) fn built_in_type_descriptor(
        &self,
        spec: &TypeSpec,
        environment: TypeEnvironmentId,
    ) -> TypeDescriptor {
        let descriptor = descriptor_from_built_in_spec(&self.heap, spec);
        self.substitute_descriptor(&descriptor, environment, 0)
    }

    pub(in crate::vm) fn invoke_built_in_function_from_stack(
        &mut self,
        spec: FunctionSpec,
        window_start: usize,
        count: usize,
    ) -> Result<Value, VirtualMachineControl> {
        match count {
            0 => self.invoke_built_in_function_values(spec, &[]),
            1 => self.invoke_built_in_function_array::<1>(spec, window_start),
            2 => self.invoke_built_in_function_array::<2>(spec, window_start),
            3 => self.invoke_built_in_function_array::<3>(spec, window_start),
            4 => self.invoke_built_in_function_array::<4>(spec, window_start),
            _ => {
                let mut arguments = Vec::with_capacity(count);
                for position in 0..count {
                    arguments
                        // SAFETY: the surrounding invariant keeps this index in bounds.
                        .push(unsafe { self.stack.get_unchecked(window_start + position).clone() });
                }
                self.invoke_built_in_function_values(spec, &arguments)
            }
        }
    }

    pub(in crate::vm) fn invoke_indexed_built_in_function_from_stack(
        &mut self,
        function: BuiltInId,
        spec: FunctionSpec,
        window_start: usize,
        count: usize,
    ) -> Result<Value, VirtualMachineControl> {
        match count {
            0 => self.invoke_indexed_built_in_function_values(function, spec, &[]),
            1 => self.invoke_indexed_built_in_function_array::<1>(function, spec, window_start),
            2 => self.invoke_indexed_built_in_function_array::<2>(function, spec, window_start),
            3 => self.invoke_indexed_built_in_function_array::<3>(function, spec, window_start),
            4 => self.invoke_indexed_built_in_function_array::<4>(function, spec, window_start),
            _ => {
                let mut arguments = Vec::with_capacity(count);
                for position in 0..count {
                    arguments
                        // SAFETY: the surrounding invariant keeps this index in bounds.
                        .push(unsafe { self.stack.get_unchecked(window_start + position).clone() });
                }
                self.invoke_indexed_built_in_function_values(function, spec, &arguments)
            }
        }
    }

    pub(in crate::vm) fn invoke_proven_built_in_function_from_stack(
        &mut self,
        spec: FunctionSpec,
        window_start: usize,
        count: usize,
    ) -> Result<Value, VirtualMachineControl> {
        match count {
            0 => self.invoke_proven_built_in_function_values(spec, &[]),
            1 => self.invoke_proven_built_in_function_array::<1>(spec, window_start),
            2 => self.invoke_proven_built_in_function_array::<2>(spec, window_start),
            3 => self.invoke_proven_built_in_function_array::<3>(spec, window_start),
            4 => self.invoke_proven_built_in_function_array::<4>(spec, window_start),
            _ => self.invoke_built_in_function_from_stack(spec, window_start, count),
        }
    }

    #[inline(always)]
    pub(in crate::vm) fn invoke_direct_built_in_function_from_stack(
        &mut self,
        spec: FunctionSpec,
        window_start: usize,
        count: usize,
    ) -> Result<Value, VirtualMachineControl> {
        let arguments =
            // SAFETY: call setup reserves this live stack window.
            unsafe { slice::from_raw_parts(self.stack.as_ptr().add(window_start), count) };
        // SAFETY: direct-call specialization records only specs with a direct handler.
        let handler = unsafe { spec.direct_handler.unwrap_unchecked() };
        let outcome = handler(self, arguments);
        if let Some(code) = self.pending_exit.take() {
            return Err(VirtualMachineControl::Exit(code));
        }
        let outcome = outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value));
        self.finish_built_in_invocation(outcome, spec.name, spec.parameters, arguments)
    }

    #[inline(always)]
    pub(in crate::vm) fn invoke_prelinked_direct_built_in_function_from_stack(
        &mut self,
        handler: BuiltInDirectHandler,
        function: BuiltInId,
        window_start: usize,
        count: usize,
    ) -> Result<Value, VirtualMachineControl> {
        let arguments =
            // SAFETY: the pointer and length share one live allocation.
            unsafe { slice::from_raw_parts(self.stack.as_ptr().add(window_start), count) };
        let outcome = handler(self, arguments);
        if let Some(code) = self.pending_exit.take() {
            return Err(VirtualMachineControl::Exit(code));
        }
        let outcome = outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value));
        if !matches!(outcome, Err(VirtualMachineControl::Throw(_))) {
            return outcome;
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
        self.finish_built_in_invocation(outcome, spec.name, spec.parameters, arguments)
    }

    #[inline(always)]
    fn invoke_built_in_function_array<const N: usize>(
        &mut self,
        spec: FunctionSpec,
        window_start: usize,
    ) -> Result<Value, VirtualMachineControl> {
        let arguments: ManuallyDrop<[Value; N]> =
            // SAFETY: the surrounding invariant keeps this index in bounds.
            ManuallyDrop::new(array::from_fn(|position| unsafe {
                ptr::read(self.stack.get_unchecked(window_start + position))
            }));

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let arguments = unsafe { &*ptr::from_ref(&arguments).cast::<[Value; N]>() };
        self.invoke_built_in_function_values(spec, arguments)
    }

    #[inline(always)]
    fn invoke_indexed_built_in_function_array<const N: usize>(
        &mut self,
        function: BuiltInId,
        spec: FunctionSpec,
        window_start: usize,
    ) -> Result<Value, VirtualMachineControl> {
        let arguments: ManuallyDrop<[Value; N]> =
            // SAFETY: the surrounding invariant keeps this index in bounds.
            ManuallyDrop::new(array::from_fn(|position| unsafe {
                ptr::read(self.stack.get_unchecked(window_start + position))
            }));
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let arguments = unsafe { &*ptr::from_ref(&arguments).cast::<[Value; N]>() };
        self.invoke_indexed_built_in_function_values(function, spec, arguments)
    }

    #[inline(always)]
    fn invoke_proven_built_in_function_array<const N: usize>(
        &mut self,
        spec: FunctionSpec,
        window_start: usize,
    ) -> Result<Value, VirtualMachineControl> {
        let arguments: ManuallyDrop<[Value; N]> =
            // SAFETY: the surrounding invariant keeps this index in bounds.
            ManuallyDrop::new(array::from_fn(|position| unsafe {
                ptr::read(self.stack.get_unchecked(window_start + position))
            }));
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let arguments = unsafe { &*ptr::from_ref(&arguments).cast::<[Value; N]>() };
        self.invoke_proven_built_in_function_values(spec, arguments)
    }

    fn invoke_built_in_function_values(
        &mut self,
        spec: FunctionSpec,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let outcome = (|| -> Result<Value, VirtualMachineControl> {
            self.validate_built_in_arguments(
                spec.parameters,
                spec.name,
                arguments,
                None,
                TypeEnvironmentId::default(),
            )?;

            let outcome =
                self.dispatch_built_in(spec.handler, arguments, None, TypeEnvironmentId::default());
            if let Some(code) = self.pending_exit.take() {
                return Err(VirtualMachineControl::Exit(code));
            }

            outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value))
        })();
        self.finish_built_in_invocation(outcome, spec.name, spec.parameters, arguments)
    }

    fn invoke_indexed_built_in_function_values(
        &mut self,
        function: BuiltInId,
        spec: FunctionSpec,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let declaration = unsafe {
            self.engine
                .tables
                .built_in_function_declarations
                .get_unchecked(function.0 as usize)
        };
        let pointer = declaration.parameters.as_ptr();
        let length = declaration.parameters.len();
        // SAFETY: the pointer and length share one live allocation.
        let parameters = unsafe { slice::from_raw_parts(pointer, length) };
        let outcome = (|| -> Result<Value, VirtualMachineControl> {
            self.validate_compiled_built_in_arguments(
                spec.parameters,
                parameters,
                spec.name,
                arguments,
            )?;

            let outcome =
                self.dispatch_built_in(spec.handler, arguments, None, TypeEnvironmentId::default());
            if let Some(code) = self.pending_exit.take() {
                return Err(VirtualMachineControl::Exit(code));
            }

            outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value))
        })();
        self.finish_built_in_invocation(outcome, spec.name, spec.parameters, arguments)
    }

    pub(in crate::vm) fn invoke_proven_built_in_function_values(
        &mut self,
        spec: FunctionSpec,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let outcome = (|| -> Result<Value, VirtualMachineControl> {
            let outcome =
                self.dispatch_built_in(spec.handler, arguments, None, TypeEnvironmentId::default());
            if let Some(code) = self.pending_exit.take() {
                return Err(VirtualMachineControl::Exit(code));
            }

            outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value))
        })();
        self.finish_built_in_invocation(outcome, spec.name, spec.parameters, arguments)
    }

    #[inline(always)]
    pub(in crate::vm) fn invoke_proven_built_in_method_from_stack(
        &mut self,
        body: BuiltInMethodBody,
        name: Atom,
        called_class: ClassId,
        environment: TypeEnvironmentId,
        window_start: usize,
        count: usize,
    ) -> Result<Value, VirtualMachineControl> {
        // SAFETY: the pointer and length share one live allocation.
        let window = unsafe { slice::from_raw_parts(self.stack.as_ptr().add(window_start), count) };
        let outcome = self.dispatch_built_in(body.handler, window, Some(called_class), environment);
        if let Some(code) = self.pending_exit.take() {
            return Err(VirtualMachineControl::Exit(code));
        }

        let outcome = outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value));
        self.finish_built_in_callable_invocation(
            outcome,
            BuiltInTraceMetadata::Method {
                body,
                name,
                called_class: Some(called_class),
            },
            &window[1..],
        )
    }

    #[inline(always)]
    fn finish_built_in_invocation(
        &self,
        outcome: Result<Value, VirtualMachineControl>,
        name: &str,
        parameters: &[ParameterSpec],
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        if let Err(VirtualMachineControl::Throw(value)) = &outcome {
            let attributes = self.built_in_function_attributes(name);
            self.record_built_in_trace_frame(value, name, parameters, arguments, attributes);
        }
        outcome
    }

    #[cold]
    fn built_in_function_attributes(&self, name: &str) -> BuiltInCallableAttributes {
        self.engine
            .tables
            .built_in_function_declarations
            .iter()
            .find(|declaration| declaration.name.as_bytes() == name.as_bytes())
            .map_or_else(
                || BuiltInCallableAttributes::for_whim_symbol(name),
                |declaration| declaration.attributes,
            )
    }

    #[inline(always)]
    fn finish_built_in_callable_invocation(
        &self,
        outcome: Result<Value, VirtualMachineControl>,
        metadata: BuiltInTraceMetadata,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let Err(VirtualMachineControl::Throw(value)) = &outcome else {
            return outcome;
        };
        match metadata {
            BuiltInTraceMetadata::Function(spec) => {
                let attributes = self.built_in_function_attributes(spec.name);
                self.record_built_in_trace_frame(
                    value,
                    spec.name,
                    spec.parameters,
                    arguments,
                    attributes,
                )
            }
            BuiltInTraceMetadata::Method {
                body,
                name,
                called_class,
            } => {
                let method = name.to_string_lossy();
                let name = called_class.map_or_else(
                    || method.clone().into_owned(),
                    |class| {
                        let class = &self.engine.tables.classes[class.0 as usize].name;
                        format!("{}::{method}", class.to_string_lossy())
                    },
                );
                self.record_built_in_trace_frame(
                    value,
                    &name,
                    body.parameters,
                    arguments,
                    body.attributes,
                );
            }
        }
        outcome
    }

    pub(in crate::vm) fn built_in_id_for_method(
        &mut self,
        entry: &MethodEntry,
        name: Atom,
    ) -> BuiltInId {
        let body = match entry.body {
            MethodBodyKind::BuiltIn(body) => body,
            // SAFETY: the surrounding invariant makes this path unreachable.
            MethodBodyKind::Bytecode(_) => unsafe {
                unreachable_invariant("bytecode methods bind as user targets")
            },
        };

        self.engine
            .intern_built_in_method(entry.declaring_class, name, body)
    }

    #[inline(always)]
    pub(crate) fn dispatch_built_in(
        &mut self,
        handler: BuiltInHandler,
        window: &[Value],
        called_class: Option<ClassId>,
        environment: TypeEnvironmentId,
    ) -> Result<Value, Throw> {
        let outcome = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let mut context = Context::over_called(self, called_class, environment);
            let arguments = window;

            (handler)(&mut context, arguments)
        }));

        match outcome {
            Ok(result) => result,
            Err(panic) => panic::resume_unwind(panic),
        }
    }

    fn validate_built_in_arity(
        &mut self,
        parameters: &[ParameterSpec],
        name: &str,
        argument_count: usize,
    ) -> Result<(), VirtualMachineControl> {
        let required = parameters
            .iter()
            .filter(|parameter| !parameter.optional)
            .count();
        if argument_count < required {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.argument_count_error,
                format!(
                    "too few arguments to {name}: {argument_count} given, at least {required} expected"
                ),
            ));
        }

        if argument_count > parameters.len() {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.argument_count_error,
                format!(
                    "too many arguments to {name}: {argument_count} given, at most {} accepted",
                    parameters.len()
                ),
            ));
        }

        Ok(())
    }

    fn validate_built_in_arguments(
        &mut self,
        parameters: &[ParameterSpec],
        name: &str,
        arguments: &[Value],
        called_class: Option<ClassId>,
        environment: TypeEnvironmentId,
    ) -> Result<(), VirtualMachineControl> {
        self.validate_built_in_arity(parameters, name, arguments.len())?;
        for (position, argument) in arguments.iter().enumerate() {
            let parameter = if position < parameters.len() {
                &parameters[position]
            } else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("the arity checks bound the positions") }
            };

            if argument.is_uninitialized() && parameter.optional {
                continue;
            }

            if !self.built_in_type_spec_accepts(
                &parameter.type_spec,
                argument,
                called_class,
                environment,
            )? {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "argument {} (${}) must be {}, {} given",
                        position + 1,
                        parameter.name,
                        self.render_built_in_type_spec(&parameter.type_spec),
                        argument.kind_name()
                    ),
                ));
            }
        }

        Ok(())
    }

    fn validate_compiled_built_in_arguments(
        &mut self,
        parameters: &[ParameterSpec],
        compiled: &[CompiledParameter],
        name: &str,
        arguments: &[Value],
    ) -> Result<(), VirtualMachineControl> {
        self.validate_built_in_arity(parameters, name, arguments.len())?;
        for (position, argument) in arguments.iter().enumerate() {
            let parameter = if position < parameters.len() {
                &parameters[position]
            } else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("the arity checks bound the positions") }
            };
            if argument.is_uninitialized() && parameter.optional {
                continue;
            }

            let accepts =
                match Self::trivial_built_in_type_spec_accepts(&parameter.type_spec, argument) {
                    Some(accepts) => accepts,
                    None => {
                        let Some(descriptor) = compiled
                            .get(position)
                            .and_then(|parameter| parameter.declared_type.as_ref())
                        else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "a built-in parameter has a compiled declared type",
                                )
                            }
                        };
                        self.check_descriptor(
                            descriptor,
                            argument,
                            None,
                            TypeEnvironmentId::default(),
                            0,
                        )?
                    }
                };
            if !accepts {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!(
                        "argument {} (${}) must be {}, {} given",
                        position + 1,
                        parameter.name,
                        self.render_built_in_type_spec(&parameter.type_spec),
                        argument.kind_name()
                    ),
                ));
            }
        }

        Ok(())
    }

    pub(in crate::vm) fn invoke_built_in_callable(
        &mut self,
        callable: BuiltInCallable,
        this: Option<&ManagedRef<InstanceObject>>,
        arguments: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let called_class = this.map(|instance| instance.class());
        let environment = this
            .map(|instance| instance.type_environment())
            .unwrap_or_default();
        self.invoke_built_in_callable_called(
            callable,
            this,
            arguments,
            called_class,
            environment,
            false,
            &[],
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hot invocation boundary keeps independent call-shape inputs unbundled"
    )]
    pub(in crate::vm) fn invoke_built_in_callable_called(
        &mut self,
        callable: BuiltInCallable,
        this: Option<&ManagedRef<InstanceObject>>,
        arguments: &[Value],
        called_class: Option<ClassId>,
        outer_environment: TypeEnvironmentId,
        type_arguments_bound: bool,
        captures: &[Value],
    ) -> Result<Value, VirtualMachineControl> {
        let trace = match &callable {
            BuiltInCallable::Function(spec) => BuiltInTraceMetadata::Function(*spec),
            BuiltInCallable::Method { body, name } => BuiltInTraceMetadata::Method {
                body: *body,
                name: name.clone(),
                called_class,
            },
        };
        let outcome = (|| -> Result<Value, VirtualMachineControl> {
            let environment = if type_arguments_bound {
                outer_environment
            } else {
                let parameters = built_in_type_parameters(&self.heap, callable.type_parameters());
                let subject = callable.display_name();
                self.bind_type_parameters(&parameters, None, outer_environment, subject.as_bytes())?
            };

            let outcome = match callable {
                BuiltInCallable::Function(spec) => {
                    self.validate_built_in_arguments(
                        spec.parameters,
                        spec.name,
                        arguments,
                        called_class,
                        environment,
                    )?;

                    if captures.is_empty() {
                        self.dispatch_built_in(spec.handler, arguments, called_class, environment)
                    } else {
                        let mut window = Vec::with_capacity(arguments.len() + captures.len());
                        window.extend(arguments.iter().cloned());
                        window.extend(captures.iter().cloned());
                        self.dispatch_built_in(spec.handler, &window, called_class, environment)
                    }
                }
                BuiltInCallable::Method { body, name } => {
                    let rendered = name.to_string_lossy().into_owned();
                    self.validate_built_in_arguments(
                        body.parameters,
                        &rendered,
                        arguments,
                        called_class,
                        environment,
                    )?;

                    let mut window = Vec::with_capacity(arguments.len() + 1);
                    window.push(match this {
                        Some(instance) => Value::object(instance.clone()),
                        None => Value::null(),
                    });

                    window.extend(arguments.iter().cloned());
                    self.dispatch_built_in(body.handler, &window, called_class, environment)
                }
            };

            if let Some(code) = self.pending_exit.take() {
                return Err(VirtualMachineControl::Exit(code));
            }

            outcome.map_err(|Throw(value)| VirtualMachineControl::Throw(value))
        })();
        self.finish_built_in_callable_invocation(outcome, trace, arguments)
    }
}
