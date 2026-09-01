//! Precise frame ownership metadata for whole-unit exact calls.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::reference_registers::mask_with_classification;
use crate::bytecode::unit::CompiledProperty;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::value::atom::Atom;

struct FunctionReturn {
    name: Atom,
    may_reference: bool,
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
) {
    let returns = unit
        .functions
        .iter()
        .map(|function| FunctionReturn {
            name: function.name.clone(),
            may_reference: function
                .return_type
                .as_ref()
                .is_none_or(TypeDescriptor::may_hold_reference),
        })
        .collect::<Vec<_>>();

    refresh_chunk(&mut unit.main, None, None, &returns);
    let function_floor = configuration.function_floor(unit.functions.len());
    for function in &mut unit.functions[function_floor..] {
        let current = function
            .return_type
            .as_ref()
            .is_none_or(TypeDescriptor::may_hold_reference);
        refresh_chunk(&mut function.chunk, Some(current), None, &returns);
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for class in &mut unit.classes[class_floor..] {
        for method in &mut class.methods {
            let current = method
                .function
                .return_type
                .as_ref()
                .is_none_or(TypeDescriptor::may_hold_reference);

            let properties = (!method.is_static).then_some(class.properties.as_slice());
            refresh_chunk(
                &mut method.function.chunk,
                Some(current),
                properties,
                &returns,
            );
        }
    }
}

fn refresh_chunk(
    chunk: &mut Chunk,
    current: Option<bool>,
    properties: Option<&[CompiledProperty]>,
    returns: &[FunctionReturn],
) {
    chunk.reference_register_mask = mask_with_classification(chunk, |instruction| {
        result_may_reference(chunk, instruction, current, properties, returns)
    });
}

fn result_may_reference(
    chunk: &Chunk,
    instruction: Instruction,
    current: Option<bool>,
    properties: Option<&[CompiledProperty]>,
    returns: &[FunctionReturn],
) -> bool {
    match instruction {
        Instruction::PropertyGetUnchecked { object, slot, .. } if object.index() == 0 => properties
            .and_then(|properties| {
                properties
                    .iter()
                    .filter(|property| !property.is_static)
                    .nth(usize::from(slot.index()))
            })
            .and_then(|property| property.declared_type.as_ref())
            .is_none_or(TypeDescriptor::may_hold_reference),
        Instruction::CallSelfUnchecked { .. } => current.unwrap_or(true),
        Instruction::CallNamedUnchecked { cache, .. }
        | Instruction::CallNamedConstantUnchecked { cache, .. } => {
            let Some(IcDescriptor::Member { name, .. }) =
                chunk.ic_descriptors.get(cache.index() as usize)
            else {
                return true;
            };
            returns
                .iter()
                .find(|function| function.name == *name)
                .is_none_or(|function| function.may_reference)
        }
        _ => true,
    }
}
