//! Resolving a callee, binding its arguments, and pushing its frame.

use std::ops::Range;
use std::ptr;
use std::rc::Rc;

use whim_bytecode::aliases::expand_aliases_using as expand_aliases;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::chunk::descriptors::string_length_matches;
use whim_value::Value;
use whim_value::ValueKind;
use whim_value::array::ArrayTypeCheck;
use whim_value::function::FuncId;
use whim_value::object::ClassId;
use whim_value::object::TypeEnvironmentId;

use crate::builtin::spec::FunctionSpec;
use crate::engine::builtins::BuiltInCallable;
use crate::engine::builtins::built_in_type_parameters;
use crate::symbols::ArgumentGuardWays;
use crate::symbols::SymbolKind;
use crate::vm::ArgumentGuard;
use crate::vm::ArgumentSlot;
use crate::vm::CacheEntry;
use crate::vm::CachedArgumentGuards;
use crate::vm::CachedBoundCallable;
use crate::vm::CachedCallEnvironment;
use crate::vm::CachedParameterGuard;
use crate::vm::CalleeShape;
use crate::vm::ExactFunctionEntry;
use crate::vm::ExactMethodEntry;
use crate::vm::Frame;
use crate::vm::FrameFlags;
use crate::vm::InlineCache;
use crate::vm::MethodBodyKind;
use crate::vm::MethodContext;
use crate::vm::NonNull;
use crate::vm::OptionalClassId;
use crate::vm::OptionalFuncId;
use crate::vm::UserCallContext;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::frame_argument_count;
use crate::vm::frame_stack_floor_offset;
use crate::vm::is_instance_of;
use crate::vm::reduce_signature;
use crate::vm::unreachable_invariant;
use crate::vm::visibility_allows;
use crate::vm::visibility_name;

#[inline]
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
        ArgumentGuard::ScalarUnion(mask) => value.kind_bit() & mask != 0,
        ArgumentGuard::Null => value.is_null(),
        ArgumentGuard::Bool => value.is_bool(),
        ArgumentGuard::Int => value.is_int(),
        ArgumentGuard::Uint => value.is_uint(),
        ArgumentGuard::Float => value.is_float(),
        ArgumentGuard::String => value.is_string(),
        ArgumentGuard::Object => value.is_object(),
        ArgumentGuard::ExactBool(expected) => value.as_bool() == Some(*expected),
        ArgumentGuard::ExactInt(expected) => value.as_int() == Some(*expected),
        ArgumentGuard::ExactUint(expected) => value.as_uint() == Some(*expected),
        ArgumentGuard::UintRange { min, max } => value.as_uint().is_some_and(|value| {
            min.is_none_or(|min| value >= min) && max.is_none_or(|max| value <= max)
        }),
        ArgumentGuard::IntRange { min, max } => value.as_int().is_some_and(|value| {
            min.is_none_or(|min| value >= min) && max.is_none_or(|max| value <= max)
        }),
        ArgumentGuard::StringLength { min, max } => value
            .as_string_len()
            .is_some_and(|length| string_length_matches(length, *min, *max)),
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
                && function.scope().is_none()
                && function.this().is_none()
                && function.presets().is_empty()
        }),
    }
}

fn scalar_union_mask(descriptor: &TypeDescriptor) -> Option<u16> {
    Some(match descriptor {
        TypeDescriptor::Null => 1 << ValueKind::Null as u16,
        TypeDescriptor::Bool => 1 << ValueKind::Bool as u16,
        TypeDescriptor::Int => 1 << ValueKind::Int as u16,
        TypeDescriptor::Uint => 1 << ValueKind::Uint as u16,
        TypeDescriptor::Float => 1 << ValueKind::Float as u16,
        TypeDescriptor::String => {
            (1 << ValueKind::String as u16) | (1 << ValueKind::ShortString as u16)
        }
        TypeDescriptor::Union(members) => members
            .iter()
            .try_fold(0, |mask, member| Some(mask | scalar_union_mask(member)?))?,
        TypeDescriptor::Array(_)
        | TypeDescriptor::Vector(_)
        | TypeDescriptor::Dictionary(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::TupleAny => 0,
        _ => return None,
    })
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
        TypeDescriptor::Uint => ArgumentGuard::Uint,
        TypeDescriptor::Float => ArgumentGuard::Float,
        TypeDescriptor::String => ArgumentGuard::String,
        TypeDescriptor::StringLength { min, max } => ArgumentGuard::StringLength {
            min: *min,
            max: *max,
        },
        TypeDescriptor::Object => ArgumentGuard::Object,
        TypeDescriptor::TrueLiteral => ArgumentGuard::ExactBool(true),
        TypeDescriptor::FalseLiteral => ArgumentGuard::ExactBool(false),
        TypeDescriptor::IntLiteral(expected) => ArgumentGuard::ExactInt(*expected),
        TypeDescriptor::UintLiteral(expected) => ArgumentGuard::ExactUint(*expected),
        TypeDescriptor::UintRange { min, max } => ArgumentGuard::UintRange {
            min: *min,
            max: *max,
        },
        TypeDescriptor::IntRange { min, max } => ArgumentGuard::IntRange {
            min: *min,
            max: *max,
        },
        TypeDescriptor::FloatLiteral(expected) => ArgumentGuard::ExactFloat(expected.to_bits()),
        TypeDescriptor::Named { .. } => {
            if !descriptor.is_resolved() || value.newtype_id().is_some() {
                return None;
            }
            let value = value.as_object()?;
            ArgumentGuard::ExactObject {
                class: value.class(),
                environment: value.type_environment(),
            }
        }
        TypeDescriptor::Callable(Some(_)) => {
            if !descriptor.is_resolved() {
                return None;
            }
            let function = value.as_function()?;
            if function.scope().is_some()
                || function.this().is_some()
                || !function.presets().is_empty()
            {
                return None;
            }
            ArgumentGuard::Callable {
                target: function.target(),
                environment: function.type_environment(),
            }
        }
        TypeDescriptor::Union(_) => {
            let guard = ArgumentGuard::ScalarUnion(scalar_union_mask(descriptor)?);
            return guard_allows(&guard, value).then_some(guard);
        }
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::StaticClass
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::Array(_)
        | TypeDescriptor::Vector(_)
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::Dictionary(_)
        | TypeDescriptor::DictionaryShape { .. }
        | TypeDescriptor::ObjectShape { .. }
        | TypeDescriptor::Callable(None)
        | TypeDescriptor::Classname(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::TupleAny
        | TypeDescriptor::Intersection(_)
        | TypeDescriptor::Negated(_) => return None,
    })
}

mod frames;
mod reflection;
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

            let mut complete = true;
            for (guard, value) in entry.guards.iter().zip(&self.stack[window.clone()]) {
                if let CachedParameterGuard::NominalClass(class) = guard {
                    if !value.as_object().is_some_and(|object| {
                        is_instance_of(&self.engine.tables.classes, object.class(), *class)
                    }) {
                        return Ok(false);
                    }
                    continue;
                }
                let CachedParameterGuard::Cheap(guard) = guard else {
                    if let CachedParameterGuard::Descriptor {
                        scalar_mask,
                        array_id,
                        ..
                    } = guard
                        && (value.kind_bit() & scalar_mask != 0
                            || array_id.is_some_and(|id| {
                                value
                                    .as_vec()
                                    .map(|array| array.type_check(id))
                                    .or_else(|| value.as_dict().map(|array| array.type_check(id)))
                                    .or_else(|| value.as_tuple().map(|array| array.type_check(id)))
                                    == Some(ArrayTypeCheck::Clean(id))
                            }))
                    {
                        continue;
                    }
                    complete = false;
                    break;
                };

                if !guard_allows(guard, value) {
                    return Ok(false);
                }
            }

            if complete {
                return Ok(true);
            }
        }

        self.check_cached_argument_guards(cache, site, function, environment, window, called)
    }

    #[inline(never)]
    fn check_cached_argument_guards(
        &mut self,
        cache: NonNull<InlineCache>,
        site: usize,
        function: FuncId,
        environment: TypeEnvironmentId,
        window: Range<usize>,
        called: Option<ClassId>,
    ) -> Result<bool, VirtualMachineControl> {
        for position in 0..window.len() {
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
                CachedParameterGuard::NominalClass(class) => {
                    if !self.stack[window.start + position]
                        .as_object()
                        .is_some_and(|object| {
                            is_instance_of(&self.engine.tables.classes, object.class(), class)
                        })
                    {
                        return Ok(false);
                    }
                }
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
                    scalar_mask,
                    array_id,
                } => {
                    let value = &self.stack[window.start + position];
                    if value.kind_bit() & scalar_mask != 0 {
                        continue;
                    }
                    let value = value.clone();
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
        if self.engine.declaration_depth != 0 {
            return;
        }
        // SAFETY: the caller's retained chunk owns this cache across call setup.
        if unsafe { &*cache.as_ref().argument_guards() }
            .get(site)
            .is_some_and(|ways| !ways.can_record(function, environment))
        {
            return;
        }

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
                            self.engine.tables.type_aliases.as_slice(),
                        );
                        if let TypeDescriptor::Named {
                            name,
                            arguments: None,
                            ..
                        } = &concrete
                            && let Some(entry) = self.engine.tables.symbols.get(name)
                            && matches!(entry.kind, SymbolKind::Class | SymbolKind::Interface)
                            && self.engine.tables.classes[entry.index as usize]
                                .type_parameters
                                .is_empty()
                        {
                            CachedParameterGuard::NominalClass(ClassId(entry.index))
                        } else {
                            match argument_guard(&concrete, value) {
                                Some(guard) => CachedParameterGuard::Cheap(guard),
                                None => CachedParameterGuard::Descriptor {
                                    scalar_mask: scalar_union_mask(&concrete).unwrap_or(0),
                                    descriptor: Rc::new(concrete),
                                    array_id: None,
                                },
                            }
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
                ..
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
