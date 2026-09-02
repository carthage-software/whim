//! Resolving a callee, binding its arguments, and pushing its frame.

use std::ops::Range;
use std::ptr;
use std::rc::Rc;

use crate::builtin::spec::FunctionSpec;
use crate::bytecode::aliases::expand_aliases;
use crate::bytecode::chunk::descriptors::CallDescriptor;
use crate::bytecode::chunk::descriptors::PresetDescriptor;
use crate::bytecode::chunk::descriptors::PresetSlot;
use crate::bytecode::chunk::descriptors::check_trivial_descriptor;
use crate::bytecode::unit::literal_value;
use crate::engine::builtins::BuiltInCallable;
use crate::engine::builtins::built_in_type_parameters;
use crate::symbols::ArgumentGuardWays;
use crate::value::function::BuiltInId;
use crate::value::function::PresetArg;
use crate::vm::ArgumentGuard;
use crate::vm::ArgumentSlot;
use crate::vm::Atom;
use crate::vm::CacheEntry;
use crate::vm::CachedArgumentGuards;
use crate::vm::CachedBoundCallable;
use crate::vm::CachedCallEnvironment;
use crate::vm::CachedParameterGuard;
use crate::vm::CallTarget;
use crate::vm::CalleeShape;
use crate::vm::Chunk;
use crate::vm::ClassId;
use crate::vm::ExactFunctionEntry;
use crate::vm::ExactMethodEntry;
use crate::vm::Frame;
use crate::vm::FrameFlags;
use crate::vm::FuncId;
use crate::vm::FunctionObject;
use crate::vm::IcDescriptor;
use crate::vm::InlineCache;
use crate::vm::InstanceObject;
use crate::vm::Literal;
use crate::vm::ManagedRef;
use crate::vm::MethodBodyKind;
use crate::vm::MethodContext;
use crate::vm::NonNull;
use crate::vm::OptionalClassId;
use crate::vm::OptionalFuncId;
use crate::vm::TypeDescriptor;
use crate::vm::TypeEnvironmentId;
use crate::vm::UserCallContext;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::find_double_colon;
use crate::vm::frame_argument_count;
use crate::vm::frame_stack_floor_offset;
use crate::vm::reduce_signature;
use crate::vm::unreachable_invariant;
use crate::vm::visibility_allows;
use crate::vm::visibility_name;

fn live_parameter_mask(
    stack: &[Value],
    base: usize,
    offset: usize,
    argc: usize,
    mut candidates: u64,
) -> u64 {
    let mut mask = 0u64;
    while candidates != 0 {
        let position = candidates.trailing_zeros() as usize;
        if position < argc && stack[base + offset + position].is_reference_counted() {
            mask |= 1u64 << (offset + position);
        }
        candidates &= candidates - 1;
    }
    mask
}

/// Whether a cheap runtime fact still holds for `value`.
#[inline(always)]
pub(in crate::vm) fn guard_allows(guard: &ArgumentGuard, value: &Value) -> bool {
    match guard {
        ArgumentGuard::Any => true,
        ArgumentGuard::Null => value.is_null(),
        ArgumentGuard::Bool => value.is_bool(),
        ArgumentGuard::Int => value.is_int(),
        ArgumentGuard::Float => value.is_float(),
        ArgumentGuard::String => value.is_string(),
        ArgumentGuard::Object => value.is_object(),
        ArgumentGuard::ExactBool(expected) => value.as_bool() == Some(*expected),
        ArgumentGuard::ExactInt(expected) => value.as_int() == Some(*expected),
        ArgumentGuard::IntRange { min, max } => value.as_int().is_some_and(|value| {
            min.is_none_or(|min| value >= min) && max.is_none_or(|max| value <= max)
        }),
        ArgumentGuard::ExactFloat(expected) => value
            .as_float()
            .is_some_and(|value| value.to_bits() == *expected),
        ArgumentGuard::ExactObject { class, environment } => {
            value.as_object().is_some_and(|value| {
                value.class() == *class && value.type_environment() == *environment
            })
        }
        ArgumentGuard::Callable {
            target,
            environment,
        } => value.as_function().is_some_and(|function| {
            function.target() == *target
                && function.type_environment() == *environment
                && function.this().is_none()
                && function.presets().is_empty()
        }),
    }
}

pub(in crate::vm) fn argument_guard(
    descriptor: &TypeDescriptor,
    value: &Value,
) -> Option<ArgumentGuard> {
    Some(match descriptor {
        TypeDescriptor::Wildcard | TypeDescriptor::Mixed => ArgumentGuard::Any,
        TypeDescriptor::Null => ArgumentGuard::Null,
        TypeDescriptor::Bool => ArgumentGuard::Bool,
        TypeDescriptor::Int => ArgumentGuard::Int,
        TypeDescriptor::Float => ArgumentGuard::Float,
        TypeDescriptor::String => ArgumentGuard::String,
        TypeDescriptor::Object => ArgumentGuard::Object,
        TypeDescriptor::TrueLiteral => ArgumentGuard::ExactBool(true),
        TypeDescriptor::FalseLiteral => ArgumentGuard::ExactBool(false),
        TypeDescriptor::IntLiteral(expected) => ArgumentGuard::ExactInt(*expected),
        TypeDescriptor::IntRange { min, max } => ArgumentGuard::IntRange {
            min: *min,
            max: *max,
        },
        TypeDescriptor::FloatLiteral(expected) => ArgumentGuard::ExactFloat(expected.to_bits()),
        TypeDescriptor::Named { .. } | TypeDescriptor::StaticClass => {
            let value = value.as_object()?;
            ArgumentGuard::ExactObject {
                class: value.class(),
                environment: value.type_environment(),
            }
        }
        TypeDescriptor::Callable(Some(_)) => {
            let function = value.as_function()?;
            if function.this().is_some() || !function.presets().is_empty() {
                return None;
            }
            ArgumentGuard::Callable {
                target: function.target(),
                environment: function.type_environment(),
            }
        }
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::Array(_)
        | TypeDescriptor::Vector(_)
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::Dictionary(_)
        | TypeDescriptor::DictionaryShape { .. }
        | TypeDescriptor::Callable(None)
        | TypeDescriptor::Classname(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::TupleAny
        | TypeDescriptor::Union(_)
        | TypeDescriptor::Intersection(_)
        | TypeDescriptor::Negated(_) => return None,
    })
}

mod frames;
mod shape;
mod sites;
mod user;

impl VirtualMachine<'_> {
    /// Whether cached argument facts from a previous call at this site still
    /// hold for this invocation.
    #[inline(always)]
    pub(in crate::vm) fn cached_argument_guards_match(
        &mut self,
        cache: NonNull<InlineCache>,
        site: usize,
        function: FuncId,
        environment: TypeEnvironmentId,
        window: Range<usize>,
        called: Option<ClassId>,
    ) -> Result<bool, VirtualMachineControl> {
        let count = window.len();
        {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let guards = unsafe { &*cache.as_ref().argument_guards() };
            if site >= guards.len() {
                return Ok(false);
            }
            // SAFETY: the surrounding invariant keeps this index in bounds.
            let Some(entry) = unsafe { guards.get_unchecked(site) }.get(function, environment)
            else {
                return Ok(false);
            };
            if entry.guards.len() != count {
                return Ok(false);
            }
        }

        for position in 0..count {
            let guard = {
                // SAFETY: the surrounding invariant keeps this index in bounds.
                let guards = unsafe { &*cache.as_ref().argument_guards() };
                // SAFETY: the surrounding invariant keeps this index in bounds.
                let Some(entry) = unsafe { guards.get_unchecked(site) }.get(function, environment)
                else {
                    return Ok(false);
                };
                // SAFETY: the surrounding invariant keeps this index in bounds.
                unsafe { entry.guards.get_unchecked(position) }.clone()
            };
            match guard {
                CachedParameterGuard::Cheap(guard) => {
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    if !guard_allows(&guard, unsafe {
                        self.stack.get_unchecked(window.start + position)
                    }) {
                        return Ok(false);
                    }
                }
                CachedParameterGuard::Descriptor {
                    descriptor,
                    array_id,
                } => {
                    let value =
                        // SAFETY: the surrounding invariant keeps this index in bounds.
                        unsafe { self.stack.get_unchecked(window.start + position).clone() };
                    if !self.check_descriptor_with_array_id(
                        &descriptor,
                        &value,
                        called,
                        TypeEnvironmentId::default(),
                        array_id,
                        0,
                    )? {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    #[inline(always)]
    pub(in crate::vm) fn cached_single_argument_guard(
        cache: NonNull<InlineCache>,
        site: usize,
        function: FuncId,
        environment: TypeEnvironmentId,
    ) -> Option<ArgumentGuard> {
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let guards = unsafe { &*cache.as_ref().argument_guards() };
        let entry = guards.get(site)?.get(function, environment)?;
        let [CachedParameterGuard::Cheap(guard)] = entry.guards.as_ref() else {
            return None;
        };
        Some(*guard)
    }

    /// Call after the ordinary parameter checks succeed.
    pub(in crate::vm) fn cache_argument_guards(
        &mut self,
        cache: NonNull<InlineCache>,
        site: usize,
        function: FuncId,
        environment: TypeEnvironmentId,
        frame_start: usize,
        count: usize,
    ) {
        let mut guards = {
            let parameters = self.engine.tables.functions[function.0 as usize].parameters();
            let mut guards = Vec::with_capacity(count);
            for (position, parameter) in parameters.iter().enumerate().take(count) {
                // SAFETY: the surrounding invariant keeps this index in bounds.
                let value = unsafe { self.stack.get_unchecked(frame_start + position) };
                let guard = match parameter.declared_type.as_ref() {
                    None => CachedParameterGuard::Cheap(ArgumentGuard::Any),
                    Some(descriptor) => {
                        let concrete = expand_aliases(
                            &self.substitute_descriptor(descriptor, environment, 0),
                            &self.engine.tables.type_aliases,
                        );
                        match argument_guard(&concrete, value) {
                            Some(guard) => CachedParameterGuard::Cheap(guard),
                            None => CachedParameterGuard::Descriptor {
                                descriptor: Rc::new(concrete),
                                array_id: None,
                            },
                        }
                    }
                };
                guards.push(guard);
            }
            guards
        };
        for guard in &mut guards {
            if let CachedParameterGuard::Descriptor {
                descriptor,
                array_id,
            } = guard
            {
                *array_id = self.array_type_check_id(descriptor);
            }
        }

        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let entries = unsafe { &mut *cache.as_ref().argument_guards() };
        if entries.len() <= site {
            entries.resize(site + 1, ArgumentGuardWays::EMPTY);
        }
        entries[site].record(CachedArgumentGuards {
            function,
            environment,
            guards: guards.into_boxed_slice(),
        });
    }
}
