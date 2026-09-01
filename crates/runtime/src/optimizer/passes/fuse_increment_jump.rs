//! Fusion of an in-place immediate increment and unconditional jump.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::cfg::control_flow_targets;
use crate::optimizer::passes::compact_removed_instructions;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.fuse_increment_jump || chunk.code.len() < 2 {
        return;
    }

    let targets = control_flow_targets(chunk);
    let mut remove = vec![false; chunk.code.len()];
    for index in 0..chunk.code.len() - 1 {
        let Instruction::AddImmediate {
            destination,
            source,
            immediate,
        } = chunk.code[index]
        else {
            continue;
        };

        let Instruction::Jump { offset } = chunk.code[index + 1] else {
            continue;
        };

        if destination != source || targets.contains(&(index + 1)) {
            continue;
        }

        let target = index as i64 + 1 + i64::from(offset.offset());
        let relative = target - index as i64;
        let Ok(relative) = i16::try_from(relative) else {
            continue;
        };

        chunk.code[index] = Instruction::IncrementJump {
            target: destination,
            immediate,
            offset: ShortJumpOffset::new(relative),
        };

        remove[index + 1] = true;
    }

    compact_removed_instructions(chunk, &remove, statistics);
}
