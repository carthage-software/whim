use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::passes::for_each_mutable_chunk;
use crate::optimizer::passes::prune_unreachable;

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
) {
    if !configuration.jump_threading {
        return;
    }

    for_each_mutable_chunk(unit, configuration, optimize_and_prune);
}

fn optimize_and_prune(chunk: &mut Chunk) {
    if optimize_chunk(chunk) {
        prune_unreachable::optimize_chunk(chunk);
    }
}

fn optimize_chunk(chunk: &mut Chunk) -> bool {
    let mut changed = false;
    for index in 0..chunk.code.len() {
        let Instruction::Jump { offset } = chunk.code[index] else {
            continue;
        };
        let follows_foreach = index.checked_sub(1).is_some_and(|previous| {
            matches!(
                chunk.code[previous],
                Instruction::ForeachNext { .. }
                    | Instruction::VecForeachNext { .. }
                    | Instruction::DictForeachNext { .. }
            )
        }) || index.checked_sub(2).is_some_and(|previous| {
            matches!(
                (chunk.code[previous], chunk.code[previous + 1]),
                (
                    Instruction::ForeachNext { .. }
                        | Instruction::VecForeachNext { .. }
                        | Instruction::DictForeachNext { .. },
                    Instruction::DrainFinalizers,
                )
            )
        });
        if follows_foreach {
            continue;
        }
        let target = i64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(i64::from(offset.offset())))
            .and_then(|target| usize::try_from(target).ok());
        let Some(target) = target else {
            continue;
        };
        let Some(instruction) = chunk.code.get(target).copied() else {
            continue;
        };
        if matches!(
            instruction,
            Instruction::Return { .. }
                | Instruction::ReturnUnchecked { .. }
                | Instruction::ReturnReferenceUnchecked { .. }
                | Instruction::ReturnScalarUnchecked { .. }
                | Instruction::ReturnPairUnchecked { .. }
                | Instruction::ReturnIntUnchecked { .. }
                | Instruction::ReturnNull
                | Instruction::ReturnNullUnchecked
        ) {
            chunk.code[index] = instruction;
            chunk.spans[index] = chunk.spans[target];
            changed = true;
        }
    }
    changed
}
