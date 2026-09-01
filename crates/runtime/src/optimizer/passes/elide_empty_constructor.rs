//! Elision of constructor dispatch for exact classes without constructors.

use hashbrown::HashSet;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::compact_removed_instructions;
use crate::value::atom::Atom;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.elide_empty_constructor {
        return;
    }

    let classes_with_constructors: HashSet<Atom> = unit
        .classes
        .iter()
        .filter(|class| {
            class
                .methods
                .iter()
                .any(|method| method.name.as_bytes() == b"__construct")
        })
        .map(|class| class.name.clone())
        .collect();
    let classes: Vec<Atom> = unit
        .classes
        .iter()
        .filter(|class| {
            has_no_constructor(class) && !classes_with_constructors.contains(&class.name)
        })
        .map(|class| class.name.clone())
        .collect();
    if classes.is_empty() {
        return;
    }

    optimize_chunk(&mut unit.main, &classes, statistics);
    let function_floor = configuration.function_floor(unit.functions.len());
    for function in &mut unit.functions[function_floor..] {
        optimize_chunk(&mut function.chunk, &classes, statistics);
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for class in &mut unit.classes[class_floor..] {
        for constant in &mut class.constants {
            optimize_initializer(&mut constant.initializer, &classes, statistics);
        }

        for property in &mut class.properties {
            if let Some(default) = &mut property.default {
                optimize_initializer(default, &classes, statistics);
            }
        }

        for method in &mut class.methods {
            optimize_chunk(&mut method.function.chunk, &classes, statistics);
        }

        for case in &mut class.cases {
            if let Some(value) = &mut case.value {
                optimize_initializer(value, &classes, statistics);
            }
        }
    }

    let constant_floor = configuration.constant_floor(unit.constants.len());
    for constant in &mut unit.constants[constant_floor..] {
        optimize_initializer(&mut constant.initializer, &classes, statistics);
    }
}

fn has_no_constructor(class: &CompiledClassLike) -> bool {
    class.kind == ClassLikeKind::Class
        && class.parent.is_none()
        && class
            .methods
            .iter()
            .all(|method| method.name.as_bytes() != b"__construct")
}

fn optimize_initializer(
    initializer: &mut ConstantInitializer,
    classes: &[Atom],
    statistics: &mut OptimizationStatistics,
) {
    if let ConstantInitializer::Thunk(chunk) = initializer {
        optimize_chunk(chunk, classes, statistics);
    }
}

fn optimize_chunk(chunk: &mut Chunk, classes: &[Atom], statistics: &mut OptimizationStatistics) {
    if chunk.code.len() < 3 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 2 {
        let Instruction::NewStatic {
            destination: instance,
            cache: class_cache,
        } = chunk.code[index]
        else {
            continue;
        };

        let Instruction::Move {
            destination: receiver,
            source,
        } = chunk.code[index + 1]
        else {
            continue;
        };

        let Instruction::CallMethod {
            argument_count,
            destination: result,
            first_argument,
            cache: constructor_cache,
        } = chunk.code[index + 2]
        else {
            continue;
        };

        if source != instance
            || first_argument != receiver
            || argument_count.value() != 1
            || targets.contains(&(index + 1))
            || targets.contains(&(index + 2))
            || !register_is_dead_after(chunk, receiver, index + 3)
            || !register_is_dead_after(chunk, result, index + 3)
            || !cache_names_class(chunk, class_cache, classes)
            || !cache_names_constructor(chunk, constructor_cache)
        {
            continue;
        }

        remove[index + 1] = true;
        remove[index + 2] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}

fn cache_names_class(chunk: &Chunk, cache: IcSlot, classes: &[Atom]) -> bool {
    let Some(IcDescriptor::Member { name, .. }) =
        chunk.ic_descriptors.get(usize::from(cache.index()))
    else {
        return false;
    };

    classes.iter().any(|class| class == name)
}

fn cache_names_constructor(chunk: &Chunk, cache: IcSlot) -> bool {
    matches!(
        chunk.ic_descriptors.get(usize::from(cache.index())),
        Some(IcDescriptor::Member { name, type_arguments: None })
            if name.as_bytes() == b"__construct"
    )
}
