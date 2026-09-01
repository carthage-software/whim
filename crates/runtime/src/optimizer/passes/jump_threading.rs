//! Retargeting of unconditional jumps through chains of unconditional jumps.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::cfg::relative_target;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
) {
    if !configuration.jump_threading || chunk.code.len() < 2 {
        return;
    }

    let targets = resolve_targets(chunk);
    for (source, target) in targets.into_iter().enumerate() {
        let Instruction::Jump { offset } = chunk.code[source] else {
            continue;
        };
        let first = relative_target(source, offset.offset());
        let Some(target) = target else {
            continue;
        };
        if target == first {
            continue;
        }

        let relative = target as i64 - source as i64;
        let Ok(relative) = i32::try_from(relative) else {
            continue;
        };
        chunk.code[source] = Instruction::Jump {
            offset: JumpOffset::new(relative),
        };
    }
}

#[derive(Clone, Copy)]
enum TargetState {
    Unknown,
    Visiting,
    Resolved(Option<usize>),
}

fn resolve_targets(chunk: &Chunk) -> Vec<Option<usize>> {
    let mut states = vec![TargetState::Unknown; chunk.code.len()];
    for source in 0..chunk.code.len() {
        if !matches!(states[source], TargetState::Unknown) {
            continue;
        }

        let mut path = Vec::new();
        let mut current = source;
        let target = loop {
            let Some(state) = states.get(current).copied() else {
                break None;
            };
            match state {
                TargetState::Resolved(target) => break target,
                TargetState::Visiting => break None,
                TargetState::Unknown => {}
            }

            states[current] = TargetState::Visiting;
            path.push(current);
            let Instruction::Jump { offset } = chunk.code[current] else {
                break Some(current);
            };
            current = relative_target(current, offset.offset());
        };

        for index in path {
            states[index] = TargetState::Resolved(target);
        }
    }

    states
        .into_iter()
        .map(|state| match state {
            TargetState::Resolved(target) => target,
            TargetState::Unknown | TargetState::Visiting => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use whim_span::Span;

    use crate::bytecode::chunk::Chunk;
    use crate::bytecode::instruction::Instruction;
    use crate::bytecode::instruction::operands::JumpOffset;
    use crate::optimizer::OptimizationConfiguration;
    use crate::optimizer::passes::jump_threading::optimize_chunk;

    fn emit(chunk: &mut Chunk, instruction: Instruction) {
        chunk.emit(instruction, Span::zero());
    }

    #[test]
    fn threads_an_unconditional_jump_chain() {
        let mut chunk = Chunk::new();
        emit(
            &mut chunk,
            Instruction::Jump {
                offset: JumpOffset::new(2),
            },
        );
        emit(&mut chunk, Instruction::ReturnNull);
        emit(
            &mut chunk,
            Instruction::Jump {
                offset: JumpOffset::new(1),
            },
        );
        emit(&mut chunk, Instruction::ReturnNull);

        optimize_chunk(&mut chunk, OptimizationConfiguration::default());

        assert_eq!(
            chunk.code[0],
            Instruction::Jump {
                offset: JumpOffset::new(3),
            }
        );
    }

    #[test]
    fn threads_a_long_shared_jump_chain() {
        let mut chunk = Chunk::new();
        for _ in 0..1_000 {
            emit(
                &mut chunk,
                Instruction::Jump {
                    offset: JumpOffset::new(1),
                },
            );
        }
        emit(&mut chunk, Instruction::ReturnNull);

        optimize_chunk(&mut chunk, OptimizationConfiguration::default());

        assert_eq!(
            chunk.code[0],
            Instruction::Jump {
                offset: JumpOffset::new(1_000),
            }
        );
    }

    #[test]
    fn leaves_a_jump_cycle_unchanged() {
        let mut chunk = Chunk::new();
        emit(
            &mut chunk,
            Instruction::Jump {
                offset: JumpOffset::new(1),
            },
        );
        emit(
            &mut chunk,
            Instruction::Jump {
                offset: JumpOffset::new(-1),
            },
        );
        let before = chunk.code.clone();

        optimize_chunk(&mut chunk, OptimizationConfiguration::default());

        assert_eq!(chunk.code, before);
    }
}
