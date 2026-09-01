//! Splicing and trampolining of proven reified generic call sites.

use std::cmp::Reverse;

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::passes::inline_leaf_calls::Atom;
use crate::optimizer::passes::inline_leaf_calls::CALLER_CODE_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::Chunk;
use crate::optimizer::passes::inline_leaf_calls::CompiledFunction;
use crate::optimizer::passes::inline_leaf_calls::CompiledParameter;
use crate::optimizer::passes::inline_leaf_calls::CompiledTypeParameter;
use crate::optimizer::passes::inline_leaf_calls::CompiledUnit;
use crate::optimizer::passes::inline_leaf_calls::Heap;
use crate::optimizer::passes::inline_leaf_calls::IcDescriptor;
use crate::optimizer::passes::inline_leaf_calls::InlineCandidates;
use crate::optimizer::passes::inline_leaf_calls::Instruction;

use crate::optimizer::passes::inline_leaf_calls::Location;
use crate::optimizer::passes::inline_leaf_calls::OptimizationStatistics;
use crate::optimizer::passes::inline_leaf_calls::REGISTER_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::TypeFlow;
use crate::optimizer::passes::inline_leaf_calls::Visibility;
use crate::optimizer::passes::inline_leaf_calls::alias_bindings;
use crate::optimizer::passes::inline_leaf_calls::is_always_inline;
use crate::optimizer::passes::inline_leaf_calls::is_external;
use crate::optimizer::passes::inline_leaf_calls::is_never_inline;
use crate::optimizer::passes::inline_leaf_calls::jumping::build_jumping_replacement_bound;
use crate::optimizer::passes::inline_leaf_calls::leaf::owned_register_mask;
use crate::optimizer::passes::inline_leaf_calls::methods::descriptor_references_parameter;
use crate::optimizer::passes::inline_leaf_calls::methods::generic_body_inlinable;
use crate::optimizer::passes::inline_leaf_calls::methods::unchecked_terminal;
use crate::optimizer::passes::inline_leaf_calls::splice_replace;
use crate::optimizer::type_flow::IndexedUnit;
use crate::optimizer::type_flow::World;

/// Finds proven reified generic call sites past `floor` whose callee bodies
/// can be spliced with the site's concrete type arguments substituted into
/// every cloned cache descriptor.
pub(in crate::optimizer) fn generic_call_sites(
    chunk: &Chunk,
    functions: &[CompiledFunction],
    candidates: &InlineCandidates<'_>,
    flow: &TypeFlow<'_>,
    floor: usize,
) -> Vec<(usize, usize)> {
    let mut sites = Vec::new();
    for (index, instruction) in chunk
        .code
        .iter()
        .enumerate()
        .skip(floor.saturating_add(1).min(chunk.code.len()))
    {
        let (Instruction::CallNamed {
            argument_count,
            first_argument,
            cache,
            ..
        }
        | Instruction::CallNamedUnchecked {
            argument_count,
            first_argument,
            cache,
            ..
        }) = *instruction
        else {
            continue;
        };

        let IcDescriptor::Member {
            name,
            type_arguments: Some(arguments),
        } = &chunk.ic_descriptors[usize::from(cache.index())]
        else {
            continue;
        };

        if arguments.iter().any(descriptor_references_parameter) {
            continue;
        }

        let Some(position) = candidates.local_position(name) else {
            continue;
        };

        let function = &functions[position];
        if is_external(&function.attributes) || is_never_inline(&function.attributes) {
            continue;
        }

        let Ok(parameters) = u16::try_from(function.parameters.len()) else {
            continue;
        };

        if function.type_parameters.is_empty()
            || !bounds_proven(function, arguments, flow)
            || function.captures_this
            || function
                .parameters
                .iter()
                .any(|parameter| parameter.has_default)
            || function.parameters.len() != usize::from(argument_count.value())
            || !generic_body_inlinable(
                &function.chunk,
                parameters,
                is_always_inline(&function.attributes),
            )
        {
            continue;
        }

        if !flow.function_arguments_proven(
            index,
            usize::from(first_argument.index()),
            usize::from(argument_count.value()),
        ) {
            continue;
        }

        sites.push((index, position));
    }

    sites
}

/// Splices collected generic call sites bottom-up so earlier site indexes
/// stay valid, binding each callee's type parameters to the site arguments.
pub(in crate::optimizer) fn splice_generic_sites(
    chunk: &mut Chunk,
    functions: &[CompiledFunction],
    sites: &[(usize, usize)],
    register_cap: u16,
    statistics: &mut OptimizationStatistics,
) -> bool {
    let mut changed = false;
    for &(index, position) in sites.iter().rev() {
        if chunk.code.len() >= CALLER_CODE_LIMIT {
            break;
        }

        let (Instruction::CallNamed {
            destination,
            first_argument,
            cache,
            ..
        }
        | Instruction::CallNamedUnchecked {
            destination,
            first_argument,
            cache,
            ..
        }) = chunk.code[index]
        else {
            continue;
        };

        let IcDescriptor::Member {
            type_arguments: Some(arguments),
            ..
        } = &chunk.ic_descriptors[usize::from(cache.index())]
        else {
            continue;
        };

        let function = &functions[position];
        let Some(bindings) = alias_bindings(&function.type_parameters, Some(arguments)) else {
            continue;
        };

        let Ok(parameters) = u16::try_from(function.parameters.len()) else {
            continue;
        };

        let snapshot = function.chunk.clone();
        let Some(terminal) = unchecked_terminal(&snapshot) else {
            continue;
        };

        if let Some(replacement) = build_jumping_replacement_bound(
            chunk,
            &snapshot,
            terminal,
            parameters,
            destination,
            first_argument,
            register_cap,
            Some(&bindings),
            owned_register_mask(chunk, index, first_argument, function, false, parameters),
        ) {
            splice_replace(chunk, index, &replacement);
            statistics.calls_inlined += 1;
            changed = true;
        }
    }

    changed
}

/// One proven reified generic static-call site scheduled for splicing.
pub(in crate::optimizer) struct StaticSite {
    location: Location,
    index: usize,
    class: usize,
    method: usize,
}

/// Splices proven calls to small generic *static* methods into their callers,
/// with the site's written type arguments substituted into the body.
pub(in crate::optimizer) fn inline_generic_statics(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    heap: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> Vec<Location> {
    let mut sites: Vec<StaticSite> = {
        let indexed = IndexedUnit::with_world(unit, world);
        let collect = |chunk: &Chunk,
                       parameters: &[CompiledParameter],
                       has_receiver: bool,
                       class_name: Option<&Atom>,
                       class_type_parameters: &[CompiledTypeParameter],
                       location: Location,
                       sites: &mut Vec<StaticSite>| {
            if !chunk
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CallStatic { .. }))
            {
                return;
            }

            let flow = TypeFlow::analyze_with_unit(
                chunk,
                parameters,
                has_receiver,
                class_name,
                class_type_parameters,
                &indexed,
                heap,
            );

            for (index, instruction) in chunk.code.iter().enumerate() {
                let Instruction::CallStatic {
                    argument_count,
                    first_argument,
                    cache,
                    ..
                } = *instruction
                else {
                    continue;
                };

                let IcDescriptor::ClassMember {
                    class: class_name,
                    member,
                    type_arguments: Some(arguments),
                } = &chunk.ic_descriptors[usize::from(cache.index())]
                else {
                    continue;
                };

                if arguments.iter().any(descriptor_references_parameter) {
                    continue;
                }

                let Some(class_position) = unit
                    .classes
                    .iter()
                    .position(|candidate| candidate.name.as_bytes() == class_name.as_bytes())
                else {
                    continue;
                };

                let class = &unit.classes[class_position];
                if is_external(&class.attributes) {
                    continue;
                }

                let Some(method_position) = class.methods.iter().position(|candidate| {
                    candidate.is_static && candidate.name.as_bytes() == member.as_bytes()
                }) else {
                    continue;
                };

                let method = &class.methods[method_position];
                if method.visibility != Visibility::Public
                    || is_never_inline(&method.function.attributes)
                {
                    continue;
                }

                let Ok(parameters) = u16::try_from(method.function.parameters.len()) else {
                    continue;
                };

                if method.function.type_parameters.is_empty()
                    || !bounds_proven(&method.function, arguments, &flow)
                    || !class.type_parameters.is_empty()
                    || method.function.captures_this
                    || method
                        .function
                        .parameters
                        .iter()
                        .any(|parameter| parameter.has_default)
                    || method.function.parameters.len() != usize::from(argument_count.value())
                    || !generic_body_inlinable(
                        &method.function.chunk,
                        parameters,
                        is_always_inline(&method.function.attributes),
                    )
                {
                    continue;
                }

                if !flow.callee_arguments_proven(
                    index,
                    usize::from(first_argument.index()),
                    usize::from(argument_count.value()),
                    &method.function.parameters,
                    &method.function.type_parameters,
                    Some(arguments),
                ) {
                    continue;
                }

                sites.push(StaticSite {
                    location,
                    index,
                    class: class_position,
                    method: method_position,
                });
            }
        };

        let mut collected = Vec::new();
        collect(
            &unit.main,
            &[],
            false,
            None,
            &[],
            Location::Main,
            &mut collected,
        );

        for (position, function) in unit
            .functions
            .iter()
            .enumerate()
            .skip(configuration.function_floor(unit.functions.len()))
        {
            collect(
                &function.chunk,
                &function.parameters,
                function.captures_this,
                None,
                &[],
                Location::Function(position),
                &mut collected,
            );
        }

        for (class_position, class) in unit
            .classes
            .iter()
            .enumerate()
            .skip(configuration.class_floor(unit.classes.len()))
        {
            for (method_position, method) in class.methods.iter().enumerate() {
                collect(
                    &method.function.chunk,
                    &method.function.parameters,
                    !method.is_static || method.function.captures_this,
                    Some(&class.name),
                    &class.type_parameters,
                    Location::Method {
                        class: class_position,
                        method: method_position,
                    },
                    &mut collected,
                );
            }
        }

        collected
    };

    if sites.is_empty() {
        return Vec::new();
    }

    sites.sort_by_key(|site| Reverse(site.index));
    let mut changed: Vec<Location> = Vec::new();
    for site in sites {
        if site.location
            == (Location::Method {
                class: site.class,
                method: site.method,
            })
        {
            continue;
        }

        let callee = &unit.classes[site.class].methods[site.method].function;
        let snapshot = callee.chunk.clone();
        let type_parameters = callee.type_parameters.clone();
        let Ok(parameters) = u16::try_from(callee.parameters.len()) else {
            continue;
        };

        let callee = callee.clone();

        let Some(terminal) = unchecked_terminal(&snapshot) else {
            continue;
        };

        let chunk = match site.location {
            Location::Main => &mut unit.main,
            Location::Function(position) => &mut unit.functions[position].chunk,
            Location::Method { class, method } => {
                &mut unit.classes[class].methods[method].function.chunk
            }
        };

        if chunk.code.len() >= CALLER_CODE_LIMIT {
            continue;
        }

        let Instruction::CallStatic {
            destination,
            first_argument,
            cache,
            ..
        } = chunk.code[site.index]
        else {
            continue;
        };

        let IcDescriptor::ClassMember {
            type_arguments: Some(arguments),
            ..
        } = &chunk.ic_descriptors[usize::from(cache.index())]
        else {
            continue;
        };

        let Some(bindings) = alias_bindings(&type_parameters, Some(arguments)) else {
            continue;
        };

        let owned = owned_register_mask(
            chunk,
            site.index,
            first_argument,
            &callee,
            false,
            parameters,
        );

        if let Some(replacement) = build_jumping_replacement_bound(
            chunk,
            &snapshot,
            terminal,
            parameters,
            destination,
            first_argument,
            REGISTER_LIMIT,
            Some(&bindings),
            owned,
        ) {
            splice_replace(chunk, site.index, &replacement);
            statistics.calls_inlined += 1;
            if !changed.contains(&site.location) {
                changed.push(site.location);
            }
        }
    }

    changed
}

/// Returns whether every explicit type argument is proven to satisfy the
/// callee's declared bounds. Inlining can omit the generic binding check only
/// when this proof is available; unresolved nominal bounds stay on the normal
/// call path.
fn bounds_proven(
    function: &CompiledFunction,
    arguments: &[TypeDescriptor],
    flow: &TypeFlow<'_>,
) -> bool {
    function
        .type_parameters
        .iter()
        .zip(arguments)
        .all(|(parameter, argument)| {
            parameter
                .bounds
                .iter()
                .all(|bound| flow.descriptor_proves(argument, bound, 0))
        })
}
