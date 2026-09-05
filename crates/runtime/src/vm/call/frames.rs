//! Pushing call frames for proven call shapes.

use std::mem::ManuallyDrop;
use std::slice;

use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::value::ValueView;
use crate::value::heap::metadata::HeapBox;
use crate::vm::call::BuiltInCallable;
use crate::vm::call::ClassId;
use crate::vm::call::ExactFunctionEntry;
use crate::vm::call::ExactMethodEntry;
use crate::vm::call::Frame;
use crate::vm::call::FrameFlags;
use crate::vm::call::FuncId;
use crate::vm::call::InstanceObject;
use crate::vm::call::Literal;
use crate::vm::call::ManagedRef;
use crate::vm::call::NonNull;
use crate::vm::call::OptionalClassId;
use crate::vm::call::OptionalFuncId;
use crate::vm::call::TypeEnvironmentId;
use crate::vm::call::Value;
use crate::vm::call::VirtualMachine;
use crate::vm::call::VirtualMachineControl;
use crate::vm::call::frame_argument_count;
use crate::vm::call::literal_value;
use crate::vm::call::live_parameter_mask;
use crate::vm::call::ptr;
use crate::vm::call::unreachable_invariant;

impl VirtualMachine<'_> {
    #[inline(always)]
    fn finalized_function_entry(
        &mut self,
        function: FuncId,
        scalar_return_target: bool,
    ) -> Result<ExactFunctionEntry, VirtualMachineControl> {
        self.engine.optimize_callable_once(function)?;
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let runtime = unsafe {
            self.engine
                .tables
                .functions
                .get_unchecked(function.0 as usize)
        };
        Ok(ExactFunctionEntry::from_runtime(
            function,
            runtime,
            scalar_return_target,
        ))
    }

    #[inline(always)]
    fn ensure_prelinked_function_site_finalized(
        &mut self,
        site: usize,
        entry: ExactFunctionEntry,
    ) -> Result<ExactFunctionEntry, VirtualMachineControl> {
        if entry.finalized {
            return Ok(entry);
        }

        self.finalize_prelinked_function_site(site, entry)
    }

    #[inline(always)]
    pub(in crate::vm) fn ensure_function_entry_finalized(
        &mut self,
        entry: ExactFunctionEntry,
    ) -> Result<ExactFunctionEntry, VirtualMachineControl> {
        if entry.finalized {
            return Ok(entry);
        }

        self.finalized_function_entry(entry.function, entry.scalar_return_target)
    }

    #[cold]
    #[inline(never)]
    fn finalize_prelinked_function_site(
        &mut self,
        site: usize,
        entry: ExactFunctionEntry,
    ) -> Result<ExactFunctionEntry, VirtualMachineControl> {
        let entry = self.finalized_function_entry(entry.function, entry.scalar_return_target)?;
        let cache_pointer = self.current_frame().cache;
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let cache = unsafe { &mut *cache_pointer.as_ref().exact_functions() };
        // SAFETY: the surrounding invariant keeps this index in bounds.
        unsafe {
            *cache.get_unchecked_mut(site) = Some(entry);
        }
        Ok(entry)
    }

    /// Pushes a checked object-iterator frame without general dispatch.
    pub(in crate::vm) fn push_object_iterator_frame(
        &mut self,
        function: FuncId,
        receiver: NonNull<HeapBox<InstanceObject>>,
        declaring_class: ClassId,
        type_environment: TypeEnvironmentId,
        return_register: u16,
    ) -> Result<(), VirtualMachineControl> {
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let receiver_class = unsafe { receiver.as_ref() }.state_ref().class();
        let frameless = self.engine.tables.functions[function.0 as usize]
            .frameless_literal
            .is_some();
        if !frameless && self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }
        self.engine.optimize_callable_once(function)?;
        let (chunk, cache, unit) = {
            let runtime = &self.engine.tables.functions[function.0 as usize];
            (
                runtime.chunk,
                NonNull::from(&*runtime.cache),
                NonNull::from(&*runtime.unit),
            )
        };
        if frameless {
            return self.execute_frameless_call(
                function,
                return_register,
                self.stack.len(),
                0,
                None,
                Some(receiver_class),
                type_environment,
                false,
                false,
            );
        }

        let base = self.stack.len();
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        self.resize_frame_stack(base + usize::from(unsafe { chunk.as_ref() }.register_count));
        self.reset_uninitialized_locals(base, chunk);
        // The cursor owns `receiver` for the whole call. Register zero borrows
        // that handle and frame teardown clears it without decrementing the
        // cursor's reference count.
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            self.stack
                .as_mut_ptr()
                .add(base)
                .write(Value::object(ManagedRef::from_raw(receiver)));
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        if unsafe { chunk.as_ref() }.register_count <= REFERENCE_REGISTER_LIMIT {
            reference_register_mask &= !1;
        }

        // SAFETY: the call-entry guard reserved a frame within the depth limit.
        unsafe {
            self.push_frame_unchecked(Frame {
                chunk,
                cache,
                unit,
                function: OptionalFuncId::some(function),
                ip: 0,
                base: base as u32,
                argc: frame_argument_count(0),
                called_class: OptionalClassId::some(receiver_class),
                class_scope: OptionalClassId::some(declaring_class),
                stack_floor_offset: 0,
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(true, true, false).with_iterator_step(),
                type_environment,
            });
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    pub(in crate::vm) fn call_depth_exceeded(&mut self) -> VirtualMachineControl {
        self.throw_well_known(
            self.engine.tables.well_known.stack_overflow_error,
            "the call depth limit was exceeded".to_string(),
        )
    }

    /// Pushes the narrow frame shape emitted for a whole-unit-proven exact
    /// method call. The call-site proof has already established a
    /// non-generic bytecode target, valid arity, and every argument type.
    #[inline(always)]
    #[expect(
        clippy::too_many_arguments,
        reason = "the exact frame shape is passed without allocating a context"
    )]
    pub(in crate::vm) fn push_exact_method_frame(
        &mut self,
        entry: ExactMethodEntry,
        return_register: u16,
        this: ManagedRef<InstanceObject>,
        window_start: usize,
        argc: usize,
        type_environment: TypeEnvironmentId,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        let frameless = entry.function.frameless;
        if !frameless && self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }

        let function = self.ensure_function_entry_finalized(entry.function)?;
        let ExactFunctionEntry {
            chunk,
            cache,
            unit,
            function,
            reference_parameter_mask,
            declared_parameters,
            scalar_return_target,
            ..
        } = function;
        if frameless {
            self.execute_frameless_call(
                function,
                return_register,
                window_start,
                argc,
                None,
                Some(entry.called),
                type_environment,
                discard_result,
                false,
            )?;
            return Ok(());
        }
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.reset_omitted_parameters(base, 1, argc, usize::from(declared_parameters));
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            self.stack.as_mut_ptr().add(base).write(Value::object(this));
        }
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            let destination = self.stack.as_mut_ptr().add(base + 1);
            let source = self.stack.as_mut_ptr().add(window_start);
            if argc == 1 {
                ptr::swap_nonoverlapping(destination, source, 1);
            } else {
                ptr::swap_nonoverlapping(destination, source, argc);
            }
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            reference_register_mask |= 1;
            if reference_parameter_mask != 0 {
                reference_register_mask |=
                    live_parameter_mask(&self.stack, base, 1, argc, reference_parameter_mask);
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
                called_class: OptionalClassId::some(entry.called),
                class_scope: OptionalClassId::some(entry.scope),
                stack_floor_offset: 0,
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(true, false, entry.is_constructor)
                    .with_scalar_return_target(scalar_return_target)
                    .with_discard_result(discard_result),
                type_environment,
            });
        }
        Ok(())
    }

    /// Pushes a static bytecode method whose target, arity, argument types,
    /// and reified environment were established by its call-site caches.
    #[inline(always)]
    pub(in crate::vm) fn push_exact_static_method_frame(
        &mut self,
        entry: ExactMethodEntry,
        return_register: u16,
        window_start: usize,
        argc: usize,
        type_environment: TypeEnvironmentId,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        let frameless = entry.function.frameless;
        if !frameless && self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }

        let function = self.ensure_function_entry_finalized(entry.function)?;
        let ExactFunctionEntry {
            chunk,
            cache,
            unit,
            function,
            reference_parameter_mask,
            declared_parameters,
            scalar_return_target,
            ..
        } = function;
        if frameless {
            self.execute_frameless_call(
                function,
                return_register,
                window_start,
                argc,
                None,
                Some(entry.called),
                type_environment,
                discard_result,
                false,
            )?;
            return Ok(());
        }
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.reset_omitted_parameters(base, 0, argc, usize::from(declared_parameters));
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            let destination = self.stack.as_mut_ptr().add(base);
            let source = self.stack.as_mut_ptr().add(window_start);
            if argc == 1 {
                ptr::swap_nonoverlapping(destination, source, 1);
            } else {
                ptr::swap_nonoverlapping(destination, source, argc);
            }
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            reference_register_mask |=
                live_parameter_mask(&self.stack, base, 0, argc, reference_parameter_mask);
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
                called_class: OptionalClassId::some(entry.called),
                class_scope: OptionalClassId::some(entry.scope),
                stack_floor_offset: 0,
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(false, false, false)
                    .with_scalar_return_target(scalar_return_target)
                    .with_discard_result(discard_result),
                type_environment,
            });
        }
        Ok(())
    }

    /// Pushes an exact method frame directly from caller registers. Scalar
    /// parameters are copied as immediates, reference parameters are cloned,
    /// and narrow frames borrow the receiver from the suspended caller.
    pub(in crate::vm) fn push_direct_method_frame(
        &mut self,
        entry: ExactMethodEntry,
        return_register: u16,
        window_start: usize,
        argc: usize,
        type_environment: TypeEnvironmentId,
    ) -> Result<(), VirtualMachineControl> {
        let frameless = entry.function.frameless;
        if !frameless && self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }

        let function = self.ensure_function_entry_finalized(entry.function)?;
        let ExactFunctionEntry {
            chunk,
            cache,
            unit,
            function,
            reference_parameter_mask,
            declared_parameters,
            scalar_return_target,
            ..
        } = function;
        if frameless {
            self.execute_frameless_call(
                function,
                return_register,
                window_start,
                argc,
                None,
                Some(entry.called),
                type_environment,
                false,
                false,
            )?;
            return Ok(());
        }
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.reset_omitted_parameters(base, 1, argc, usize::from(declared_parameters));

        let receiver = match self.stack[window_start].transparent() {
            ValueView::Object(receiver) => {
                if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
                    // SAFETY: the source and target ranges are valid.
                    unsafe { ptr::read(receiver) }
                } else {
                    receiver.clone()
                }
            }
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe {
                unreachable_invariant("a direct method call has a proven object receiver")
            },
        };
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            self.stack
                .as_mut_ptr()
                .add(base)
                .write(Value::object(receiver));
        }
        for position in 0..argc {
            let source = &self.stack[window_start + 1 + position];
            let value = if position >= usize::from(REFERENCE_REGISTER_LIMIT)
                || reference_parameter_mask & (1u64 << position) != 0
            {
                source.clone()
            } else {
                debug_assert!(!source.is_reference_counted());
                // SAFETY: the source and target ranges are valid.
                unsafe { ptr::read(source) }
            };
            // SAFETY: call setup keeps the chunk and reserved stack window live.
            unsafe {
                self.stack
                    .as_mut_ptr()
                    .add(base + 1 + position)
                    .write(value);
            }
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            debug_assert_eq!(reference_register_mask & 1, 0);
            reference_register_mask &= !1;
            reference_register_mask |=
                live_parameter_mask(&self.stack, base, 1, argc, reference_parameter_mask);
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
                called_class: OptionalClassId::some(entry.called),
                class_scope: OptionalClassId::some(entry.scope),
                stack_floor_offset: 0,
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(true, true, entry.is_constructor)
                    .with_scalar_return_target(scalar_return_target),
                type_environment,
            });
        }
        Ok(())
    }

    /// Pushes the narrow frame shape emitted for a whole-unit-proven exact
    /// named-function call.
    pub(in crate::vm) fn push_exact_function_frame(
        &mut self,
        site: usize,
        entry: ExactFunctionEntry,
        return_register: u16,
        window_start: usize,
        argc: usize,
    ) -> Result<(), VirtualMachineControl> {
        let function = entry.function;
        let frameless = entry.frameless;
        if !frameless && self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }

        if cfg!(debug_assertions) {
            let runtime = &self.engine.tables.functions[function.0 as usize];
            debug_assert!(runtime.type_parameters().is_empty());
            debug_assert!(!runtime.captures_this);
            debug_assert!(usize::from(runtime.required_parameters) <= argc);
            debug_assert!(usize::from(runtime.declared_parameters) >= argc);
        }

        let entry = self.ensure_prelinked_function_site_finalized(site, entry)?;
        let chunk = entry.chunk;
        let cache = entry.cache;
        let unit = entry.unit;
        let reference_parameter_mask = entry.reference_parameter_mask;
        let declared_parameters = usize::from(entry.declared_parameters);
        if frameless {
            self.execute_frameless_call(
                function,
                return_register,
                window_start,
                argc,
                None,
                None,
                TypeEnvironmentId::default(),
                false,
                false,
            )?;
            return Ok(());
        }
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.reset_omitted_parameters(base, 0, argc, declared_parameters);
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            let destination = self.stack.as_mut_ptr().add(base);
            let source = self.stack.as_mut_ptr().add(window_start);
            if argc == 1 {
                ptr::swap_nonoverlapping(destination, source, 1);
            } else {
                ptr::swap_nonoverlapping(destination, source, argc);
            }
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            reference_register_mask |=
                live_parameter_mask(&self.stack, base, 0, argc, reference_parameter_mask);
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
                called_class: OptionalClassId::NONE,
                class_scope: OptionalClassId::NONE,
                stack_floor_offset: 0,
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(false, false, false)
                    .with_scalar_return_target(entry.scalar_return_target),
                type_environment: TypeEnvironmentId::default(),
            });
        }
        Ok(())
    }

    /// Pushes a proven generic named call with its already-bound reified
    /// environment while retaining a normal user frame for traces.
    #[inline(always)]
    pub(in crate::vm) fn push_exact_generic_function_frame(
        &mut self,
        function: FuncId,
        return_register: u16,
        window_start: usize,
        argc: usize,
        type_environment: TypeEnvironmentId,
        discard_result: bool,
    ) -> Result<(), VirtualMachineControl> {
        let frameless = self.engine.tables.functions[function.0 as usize]
            .frameless_literal
            .is_some();
        if !frameless && self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }

        if cfg!(debug_assertions) {
            let runtime = &self.engine.tables.functions[function.0 as usize];
            debug_assert!(!runtime.captures_this);
            debug_assert!(usize::from(runtime.required_parameters) <= argc);
            debug_assert!(usize::from(runtime.declared_parameters) >= argc);
        }
        let entry = self.finalized_function_entry(function, false)?;

        let chunk = entry.chunk;
        let cache = entry.cache;
        let unit = entry.unit;
        let function = entry.function;
        let reference_parameter_mask = entry.reference_parameter_mask;
        let declared = usize::from(
            // SAFETY: call setup keeps the chunk and reserved stack window live.
            unsafe {
                self.engine
                    .tables
                    .functions
                    .get_unchecked(function.0 as usize)
            }
            .declared_parameters,
        );
        if frameless {
            self.execute_frameless_call(
                function,
                return_register,
                window_start,
                argc,
                None,
                None,
                type_environment,
                discard_result,
                false,
            )?;
            return Ok(());
        }
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.reset_omitted_parameters(base, 0, argc, declared);
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            let destination = self.stack.as_mut_ptr().add(base);
            let source = self.stack.as_mut_ptr().add(window_start);
            if argc == 1 {
                ptr::swap_nonoverlapping(destination, source, 1);
            } else {
                ptr::swap_nonoverlapping(destination, source, argc);
            }
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            reference_register_mask |=
                live_parameter_mask(&self.stack, base, 0, argc, reference_parameter_mask);
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
                called_class: OptionalClassId::NONE,
                class_scope: OptionalClassId::NONE,
                stack_floor_offset: 0,
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(false, false, false)
                    .with_scalar_return_target(entry.scalar_return_target)
                    .with_discard_result(discard_result),
                type_environment,
            });
        }
        Ok(())
    }

    /// Pushes a normal exact-function frame whose sole argument came directly
    /// from the caller's constant pool.
    pub(in crate::vm) fn call_exact_constant_function_site(
        &mut self,
        site: usize,
        destination: u16,
        literal: &Literal,
        borrowed: bool,
    ) -> Result<(), VirtualMachineControl> {
        if let Some(entry) = self.prelinked_built_in_function_site(site) {
            let function = entry.function;
            // SAFETY: call setup keeps the chunk and reserved stack window live.
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
            let value = if borrowed {
                let Literal::String(atom) = literal else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("only a string constant may be borrowed") }
                };
                let argument =
                    // SAFETY: the source and target ranges are valid.
                    ManuallyDrop::new(Value::string(unsafe { ptr::read(atom.as_handle()) }));
                // SAFETY: call setup keeps the chunk and reserved stack window live.
                let argument = unsafe { &*ptr::from_ref(&argument).cast::<Value>() };
                self.invoke_proven_built_in_function_values(spec, slice::from_ref(argument))?
            } else {
                let argument = literal_value(literal);
                self.invoke_proven_built_in_function_values(spec, slice::from_ref(&argument))?
            };
            let target = self.current_base() + usize::from(destination);
            self.stack[target] = value;
            return Ok(());
        }
        let entry = self.prelinked_function_site(site);
        if borrowed {
            self.push_exact_constant_function_frame::<true>(site, entry, destination, literal)
        } else {
            self.push_exact_constant_function_frame::<false>(site, entry, destination, literal)
        }
    }

    pub(in crate::vm) fn push_exact_constant_function_frame<const BORROWED: bool>(
        &mut self,
        site: usize,
        entry: ExactFunctionEntry,
        return_register: u16,
        literal: &Literal,
    ) -> Result<(), VirtualMachineControl> {
        if self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }

        let function = entry.function;
        if cfg!(debug_assertions) {
            // SAFETY: call setup keeps the chunk and reserved stack window live.
            let runtime = unsafe {
                self.engine
                    .tables
                    .functions
                    .get_unchecked(function.0 as usize)
            };
            debug_assert!(runtime.type_parameters().is_empty());
            debug_assert!(!runtime.captures_this);
            debug_assert!(usize::from(runtime.required_parameters) <= 1);
            debug_assert!(runtime.declared_parameters >= 1);
        }

        let entry = self.ensure_prelinked_function_site_finalized(site, entry)?;
        let chunk = entry.chunk;
        let cache = entry.cache;
        let unit = entry.unit;
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.reset_omitted_parameters(base, 0, 1, usize::from(entry.declared_parameters));
        debug_assert!(!self.stack[base].is_reference_counted());
        let argument = if BORROWED {
            let Literal::String(atom) = literal else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("only a string constant may be borrowed") }
            };
            // SAFETY: the source and target ranges are valid.
            Value::string(unsafe { ptr::read(atom.as_handle()) })
        } else {
            literal_value(literal)
        };
        let argument_is_reference = !BORROWED && argument.is_reference_counted();
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            self.stack.as_mut_ptr().add(base).write(argument);
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            if BORROWED {
                reference_register_mask &= !1;
            } else if argument_is_reference {
                reference_register_mask |= 1;
            }
        }
        self.snapshot_trace_arguments(base, chunk, 1, &mut reference_register_mask);
        // SAFETY: the call-entry guard reserved a frame within the depth limit.
        unsafe {
            self.push_frame_unchecked(Frame {
                chunk,
                cache,
                unit,
                function: OptionalFuncId::some(function),
                ip: 0,
                base: base as u32,
                argc: 1,
                called_class: OptionalClassId::NONE,
                class_scope: OptionalClassId::NONE,
                stack_floor_offset: 0,
                reference_register_mask,
                return_register,
                flags: FrameFlags::new(false, BORROWED, false)
                    .with_scalar_return_target(entry.scalar_return_target),
                type_environment: TypeEnvironmentId::default(),
            });
        }
        Ok(())
    }

    #[inline(always)]
    pub(in crate::vm) fn prelinked_function_site(&self, site: usize) -> ExactFunctionEntry {
        let cache_pointer = self.current_frame().cache;
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let cache_cell = unsafe { cache_pointer.as_ref() };
        // SAFETY: this call site uses the cache's exact-function storage.
        let cache = unsafe { &*cache_cell.exact_functions() };
        // SAFETY: linking populated this verified call site's entry.
        unsafe { cache.get_unchecked(site).unwrap_unchecked() }
    }

    /// Pushes an exact recursive frame without consulting a symbol or inline
    /// cache.
    pub(in crate::vm) fn call_exact_self(
        &mut self,
        destination: u16,
        window_start: usize,
        argc: usize,
    ) -> Result<(), VirtualMachineControl> {
        if self.frames.len() >= self.call_frame_limit() {
            self.grow_call_frames()?;
        }

        let (chunk, cache, unit, function, type_environment, declared) = {
            let frame = self.current_frame();
            let Some(function) = frame.function.get() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant("a self-call instruction executes inside a user function")
                }
            };
            (
                frame.chunk,
                frame.cache,
                frame.unit,
                function,
                frame.type_environment,
                usize::from(
                    // SAFETY: call setup keeps the chunk and reserved stack window live.
                    unsafe {
                        self.engine
                            .tables
                            .functions
                            .get_unchecked(function.0 as usize)
                    }
                    .declared_parameters,
                ),
            )
        };
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.reset_omitted_parameters(base, 0, argc, declared);
        // SAFETY: call setup keeps the chunk and reserved stack window live.
        unsafe {
            let destination = self.stack.as_mut_ptr().add(base);
            let source = self.stack.as_mut_ptr().add(window_start);
            if argc == 1 {
                ptr::swap_nonoverlapping(destination, source, 1);
            } else {
                ptr::swap_nonoverlapping(destination, source, argc);
            }
        }

        // SAFETY: call setup keeps the chunk and reserved stack window live.
        let mut reference_register_mask = unsafe { chunk.as_ref() }.reference_register_mask;
        if reference_register_mask != 0 && register_count <= usize::from(REFERENCE_REGISTER_LIMIT) {
            // SAFETY: call setup keeps the chunk and reserved stack window live.
            let reference_parameter_mask = unsafe {
                self.engine
                    .tables
                    .functions
                    .get_unchecked(function.0 as usize)
                    .reference_parameter_mask
            };
            if reference_parameter_mask != 0 {
                reference_register_mask |=
                    live_parameter_mask(&self.stack, base, 0, argc, reference_parameter_mask);
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
                called_class: OptionalClassId::NONE,
                class_scope: OptionalClassId::NONE,
                stack_floor_offset: 0,
                reference_register_mask,
                return_register: destination,
                flags: FrameFlags::new(false, false, false),
                type_environment,
            });
        }
        Ok(())
    }
}
