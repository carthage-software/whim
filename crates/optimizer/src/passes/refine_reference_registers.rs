//! Precise frame ownership metadata for whole-unit exact calls.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::IcDescriptor;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::reference_registers::mask_with_classification;
use whim_bytecode::unit::CompiledProperty;
use whim_bytecode::unit::CompiledUnit;
use whim_value::atom::Atom;

use crate::OptimizationConfiguration;

struct FunctionReturn {
    name: Atom,
    may_reference: bool,
}

pub(crate) fn optimize_unit(unit: &mut CompiledUnit, configuration: OptimizationConfiguration) {
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

    refresh_chunk(&mut unit.main, 0, None, None, &returns);
    let function_floor = configuration.function_floor(unit.functions.len());
    for function in &mut unit.functions[function_floor..] {
        let current = function
            .return_type
            .as_ref()
            .is_none_or(TypeDescriptor::may_hold_reference);
        let incoming = function.incoming_register_count(function.captures_this);
        refresh_chunk(&mut function.chunk, incoming, Some(current), None, &returns);
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for class in &mut unit.classes[class_floor..] {
        for method in &mut class.methods {
            let current = method
                .function
                .return_type
                .as_ref()
                .is_none_or(TypeDescriptor::may_hold_reference);

            let incoming = method
                .function
                .incoming_register_count(!method.is_static || method.function.captures_this);
            let properties = (!method.is_static).then_some(class.properties.as_slice());
            refresh_chunk(
                &mut method.function.chunk,
                incoming,
                Some(current),
                properties,
                &returns,
            );
        }
    }
}

fn refresh_chunk(
    chunk: &mut Chunk,
    incoming_register_count: u16,
    current: Option<bool>,
    properties: Option<&[CompiledProperty]>,
    returns: &[FunctionReturn],
) {
    chunk.reference_register_mask =
        mask_with_classification(chunk, incoming_register_count, |instruction| {
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
        Instruction::CallNamedDirect { cache, .. }
        | Instruction::CallNamedUnchecked { cache, .. }
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
