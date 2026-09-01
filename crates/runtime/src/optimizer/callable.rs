//! Exactly-once optimization of one callable against its declared world.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::World;
use crate::optimizer::optimize_unit_with_world;
use crate::value::heap::Heap;

/// Optimizes one top-level function while every other declaration is read-only context.
pub(crate) fn optimize_function(
    owner: &CompiledUnit,
    position: usize,
    mut contexts: Vec<CompiledClassLike>,
    world: &World<'_>,
    heap: &Heap,
    mut configuration: OptimizationConfiguration,
) -> (Chunk, Vec<CompiledClassLike>) {
    let mut synthetic = context_unit(owner);
    synthetic.functions.push(owner.functions[position].clone());
    synthetic.classes.append(&mut contexts);
    configuration.immutable_function_floor = 0;
    configuration.immutable_class_floor = synthetic.classes.len();
    configuration.immutable_constant_floor = 0;
    optimize_unit_with_world(&mut synthetic, world, heap, configuration);
    (synthetic.functions.remove(0).chunk, synthetic.classes)
}

/// Optimizes one method while every other declaration is read-only context.
pub(crate) fn optimize_method(
    owner: &CompiledUnit,
    class_position: usize,
    method_position: usize,
    mut contexts: Vec<CompiledClassLike>,
    world: &World<'_>,
    heap: &Heap,
    mut configuration: OptimizationConfiguration,
) -> (Chunk, Vec<CompiledClassLike>) {
    let mut synthetic = context_unit(owner);
    let owner_class = &owner.classes[class_position];
    debug_assert_eq!(
        contexts.first().map(|class| &class.name),
        Some(&owner_class.name)
    );
    let method = owner_class.methods[method_position].clone();
    synthetic.classes.append(&mut contexts);
    let target = owner_class.clone_with_methods(vec![method]);
    synthetic.classes.push(target);

    configuration.immutable_function_floor = 0;
    configuration.immutable_class_floor = synthetic.classes.len() - 1;
    configuration.immutable_constant_floor = 0;
    optimize_unit_with_world(&mut synthetic, world, heap, configuration);

    let mut target = synthetic
        .classes
        .pop()
        .expect("the target class is present");
    let chunk = target.methods.remove(0).function.chunk;
    (chunk, synthetic.classes)
}

fn context_unit(owner: &CompiledUnit) -> CompiledUnit {
    CompiledUnit {
        path: owner.path.clone(),
        main: Chunk::new(),
        functions: Vec::new(),
        classes: Vec::<CompiledClassLike>::new(),
        constants: Vec::new(),
        type_aliases: owner.type_aliases.clone(),
        newtypes: owner.newtypes.clone(),
    }
}
