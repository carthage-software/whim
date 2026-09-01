//! Direct-method inlining and the shared callee-body admissibility checks.

use std::cell::OnceCell;
use std::cmp::Reverse;
use std::ptr::from_ref;

use hashbrown::HashMap;

use crate::bytecode::rewrite::compact;
use crate::bytecode::unit::CompiledMethod;
use crate::bytecode::unit::must_use_note;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::passes::inline_leaf_calls::Atom;
use crate::optimizer::passes::inline_leaf_calls::CALLEE_INSTRUCTION_LIMIT;
use crate::optimizer::passes::inline_leaf_calls::Chunk;
use crate::optimizer::passes::inline_leaf_calls::CompiledParameter;
use crate::optimizer::passes::inline_leaf_calls::CompiledTypeParameter;
use crate::optimizer::passes::inline_leaf_calls::CompiledUnit;
use crate::optimizer::passes::inline_leaf_calls::Heap;
use crate::optimizer::passes::inline_leaf_calls::Instruction;
use crate::optimizer::passes::inline_leaf_calls::Location;
use crate::optimizer::passes::inline_leaf_calls::MethodSite;
use crate::optimizer::passes::inline_leaf_calls::OptimizationStatistics;
use crate::optimizer::passes::inline_leaf_calls::Register;
use crate::optimizer::passes::inline_leaf_calls::TypeDescriptor;
use crate::optimizer::passes::inline_leaf_calls::TypeFlow;
use crate::optimizer::passes::inline_leaf_calls::Visibility;
use crate::optimizer::passes::inline_leaf_calls::effect_on;
use crate::optimizer::passes::inline_leaf_calls::is_always_inline;
use crate::optimizer::passes::inline_leaf_calls::is_external;
use crate::optimizer::passes::inline_leaf_calls::is_never_inline;
use crate::optimizer::passes::inline_leaf_calls::jumping::body_jump_targets_are_forward;
use crate::optimizer::passes::inline_leaf_calls::jumping::build_jumping_replacement;
use crate::optimizer::passes::inline_leaf_calls::leaf::owned_register_mask;
use crate::optimizer::passes::inline_leaf_calls::leaf::straight_line_body_instruction;
use crate::optimizer::passes::inline_leaf_calls::splice_replace;
use crate::optimizer::type_flow::IndexedUnit;
use crate::optimizer::type_flow::World;

/// Splices exact-class-proven direct method calls whose bodies are tiny
/// slot-typed leaves into their callers.
pub(super) fn inline_direct_methods(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    heap: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> Vec<Location> {
    let mut sites: Vec<MethodSite>;
    {
        let method_locations = OnceCell::new();
        let indexed = IndexedUnit::with_world(unit, world);
        let analyze = |chunk: &Chunk,
                       parameters: &[CompiledParameter],
                       has_receiver: bool,
                       class_name: Option<&Atom>,
                       class_type_parameters: &[CompiledTypeParameter],
                       location: Location,
                       sites: &mut Vec<MethodSite>| {
            if chunk.code.is_empty()
                || !chunk.code.iter().any(|instruction| {
                    matches!(
                        instruction,
                        Instruction::CallMethodDirect { .. }
                            | Instruction::CallMethodUnchecked { .. }
                            | Instruction::CallMethodDiscarded { .. }
                    )
                })
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
                if !matches!(
                    instruction,
                    Instruction::CallMethodDirect { .. }
                        | Instruction::CallMethodUnchecked { .. }
                        | Instruction::CallMethodDiscarded { .. }
                ) {
                    continue;
                }

                let Some(method) = flow.resolved_method_at(index, 0) else {
                    continue;
                };

                let locations = method_locations.get_or_init(|| local_method_locations(unit));
                let Some(&(class_position, method_position)) = locations.get(&from_ref(method))
                else {
                    continue;
                };

                let permitted = match method.visibility {
                    Visibility::Public => true,
                    _ => matches!(
                        location,
                        Location::Method { class, .. } if class == class_position
                    ),
                };

                if !permitted {
                    continue;
                }

                if is_never_inline(&method.function.attributes) {
                    continue;
                }

                let discarded_destination = match instruction {
                    Instruction::CallMethodDiscarded { destination, .. } => Some(*destination),
                    _ => None,
                };
                if let Some(destination) = discarded_destination {
                    if must_use_note(&method.function.attributes).is_some() {
                        continue;
                    }

                    if !matches!(
                        chunk.code.get(index + 1),
                        Some(Instruction::CheckDiscardedResult { source }) if *source == destination
                    ) {
                        continue;
                    }
                }

                sites.push(MethodSite {
                    location,
                    index,
                    class: class_position,
                    method: method_position,
                    discarded: discarded_destination.is_some(),
                });
            }
        };

        let mut collected = Vec::new();
        analyze(
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
            analyze(
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
                analyze(
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

        sites = collected;
    }

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

        let callee = unit.classes[site.class].methods[site.method]
            .function
            .clone();
        let snapshot = callee.chunk.clone();
        let declared = callee.parameters.len();
        let Ok(parameters) = u16::try_from(declared + 1) else {
            continue;
        };

        let force = is_always_inline(&callee.attributes);
        if !method_body_inlinable(&snapshot, parameters, force) {
            continue;
        }

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

        let (argument_count, destination, first_argument) = match chunk.code[site.index] {
            Instruction::CallMethodDirect {
                argument_count,
                destination,
                first_argument,
                ..
            }
            | Instruction::CallMethodUnchecked {
                argument_count,
                destination,
                first_argument,
                ..
            }
            | Instruction::CallMethodDiscarded {
                argument_count,
                destination,
                first_argument,
                ..
            } => (argument_count, destination, first_argument),
            _ => continue,
        };

        if usize::from(argument_count.value()) != usize::from(parameters) {
            continue;
        }

        let owned =
            owned_register_mask(chunk, site.index, first_argument, &callee, true, parameters);
        if let Some(replacement) = build_jumping_replacement(
            chunk,
            &snapshot,
            terminal,
            parameters,
            destination,
            first_argument,
            owned,
        ) {
            let replacement_len = replacement.len();
            splice_replace(chunk, site.index, &replacement);
            if site.discarded {
                let check = site.index + replacement_len;
                let mut remove = vec![false; chunk.code.len()];
                remove[check] = true;
                compact(chunk, &remove);
                statistics.instructions_removed += 1;
            }
            statistics.calls_inlined += 1;
            if !changed.contains(&site.location) {
                changed.push(site.location);
            }
        }
    }

    changed
}

fn local_method_locations(unit: &CompiledUnit) -> HashMap<*const CompiledMethod, (usize, usize)> {
    unit.classes
        .iter()
        .enumerate()
        .filter(|(_, class)| !is_external(&class.attributes))
        .flat_map(|(class, declaration)| {
            declaration
                .methods
                .iter()
                .enumerate()
                .map(move |(method, declaration)| (from_ref(declaration), (class, method)))
        })
        .collect()
}

pub(super) fn unchecked_terminal(chunk: &Chunk) -> Option<usize> {
    let mut terminal = chunk.code.len().checked_sub(1)?;
    if terminal > 0 && matches!(chunk.code[terminal], Instruction::ReturnNull) {
        terminal -= 1;
    }

    matches!(
        chunk.code[terminal],
        Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::ReturnNullUnchecked
    )
    .then_some(terminal)
}

/// Whether a method body may be spliced: bounded, table-free apart from
/// class-relative property slots, with no writes to the receiver window.
pub(super) fn method_body_inlinable(chunk: &Chunk, parameters: u16, force: bool) -> bool {
    if chunk.code.is_empty()
        || (!force && chunk.code.len() > CALLEE_INSTRUCTION_LIMIT)
        || !chunk.catch_table.is_empty()
        || !chunk.switch_tables.is_empty()
    {
        return false;
    }

    let Some(terminal) = unchecked_terminal(chunk) else {
        return false;
    };

    for (index, instruction) in chunk.code[..terminal].iter().copied().enumerate() {
        let allowed = straight_line_body_instruction(instruction)
            || matches!(
                instruction,
                Instruction::PropertyGetUnchecked { .. }
                    | Instruction::PropertySetUnchecked { .. }
                    | Instruction::ReturnScalarUnchecked { .. }
                    | Instruction::ReturnReferenceUnchecked { .. }
                    | Instruction::ReturnUnchecked { .. }
                    | Instruction::ReturnIntUnchecked { .. }
                    | Instruction::ReturnNullUnchecked
            )
            || body_jump_targets_are_forward(instruction, index, terminal);

        if !allowed {
            return false;
        }

        for parameter in 0..parameters {
            if effect_on(chunk, instruction, Register::new(parameter)).writes() {
                return false;
            }
        }
    }

    true
}

/// Whether a reified generic function body may be spliced: the method-body
/// shape plus instantiation and unchecked-method-call sites whose caches can
/// be cloned with the call site's type arguments substituted in.
pub(super) fn generic_body_inlinable(chunk: &Chunk, parameters: u16, force: bool) -> bool {
    if chunk.code.is_empty()
        || (!force && chunk.code.len() > CALLEE_INSTRUCTION_LIMIT)
        || !chunk.catch_table.is_empty()
        || !chunk.switch_tables.is_empty()
    {
        return false;
    }

    let Some(terminal) = unchecked_terminal(chunk) else {
        return false;
    };

    for (index, instruction) in chunk.code[..terminal].iter().copied().enumerate() {
        let allowed = straight_line_body_instruction(instruction)
            || matches!(
                instruction,
                Instruction::PropertyGetUnchecked { .. }
                    | Instruction::PropertySetUnchecked { .. }
                    | Instruction::NewStatic { .. }
                    | Instruction::ReturnScalarUnchecked { .. }
                    | Instruction::ReturnReferenceUnchecked { .. }
                    | Instruction::ReturnUnchecked { .. }
                    | Instruction::ReturnIntUnchecked { .. }
                    | Instruction::ReturnNullUnchecked
            )
            || matches!(
                instruction,
                Instruction::CallMethodUnchecked { first_argument, .. }
                    if first_argument.index() >= parameters
            )
            || body_jump_targets_are_forward(instruction, index, terminal);
        if !allowed {
            return false;
        }

        for parameter in 0..parameters {
            if effect_on(chunk, instruction, Register::new(parameter)).writes() {
                return false;
            }
        }
    }

    true
}

pub(super) fn descriptor_references_parameter(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Parameter(_) => true,
        TypeDescriptor::Named { arguments, .. } => arguments
            .as_ref()
            .is_some_and(|arguments| arguments.iter().any(descriptor_references_parameter)),
        TypeDescriptor::Array(arguments) => arguments.as_ref().is_some_and(|(key, value)| {
            descriptor_references_parameter(key) || descriptor_references_parameter(value)
        }),
        TypeDescriptor::Vector(element) => element
            .as_ref()
            .is_some_and(|element| descriptor_references_parameter(element)),
        TypeDescriptor::Dictionary(arguments) => arguments.as_ref().is_some_and(|(key, value)| {
            descriptor_references_parameter(key) || descriptor_references_parameter(value)
        }),
        TypeDescriptor::Callable(callable) => callable.as_ref().is_some_and(|callable| {
            callable
                .parameters
                .iter()
                .any(|parameter| descriptor_references_parameter(&parameter.r#type))
                || descriptor_references_parameter(&callable.return_type)
        }),
        TypeDescriptor::Classname(inner) | TypeDescriptor::Negated(inner) => {
            descriptor_references_parameter(inner)
        }
        TypeDescriptor::Tuple(members)
        | TypeDescriptor::Union(members)
        | TypeDescriptor::Intersection(members) => {
            members.iter().any(descriptor_references_parameter)
        }
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.iter().any(descriptor_references_parameter)
                || descriptor_references_parameter(rest)
        }
        _ => false,
    }
}
