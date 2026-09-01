//! Elision of unused keys from specialized collection iteration.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::passes::for_each_mutable_chunk;

pub(in crate::optimizer) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.elide_foreach_keys {
        return;
    }

    for index in 0..chunk.code.len() {
        let key_destination = match chunk.code[index] {
            Instruction::VecForeachNext {
                key_destination, ..
            }
            | Instruction::DictForeachNext {
                key_destination, ..
            } if key_destination != Register::NONE => key_destination,
            _ => continue,
        };

        if !register_is_dead_after(chunk, key_destination, index + 2) {
            continue;
        }

        match &mut chunk.code[index] {
            Instruction::VecForeachNext {
                key_destination, ..
            }
            | Instruction::DictForeachNext {
                key_destination, ..
            } => *key_destination = Register::NONE,
            _ => unreachable!("the instruction was matched above"),
        }

        statistics.foreach_keys_elided += 1;
    }
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    for_each_mutable_chunk(unit, configuration, |chunk| {
        optimize_chunk(chunk, configuration, statistics);
    });
}
