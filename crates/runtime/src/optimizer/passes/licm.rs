//! Loop-invariant code motion for immutable loop bounds.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::liveness::register_is_read_before_write;
use crate::optimizer::liveness::register_is_untouched_between;
use crate::unreachable_invariant;

/// Hoists a class-constant loop bound by giving it a register that the loop
/// body cannot reuse and retargeting back edges past the immutable load.
pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
) {
    if !configuration.licm
        || chunk.code.len() < 3
        || !chunk.catch_table.is_empty()
        || chunk.register_count == u16::MAX
    {
        return;
    }

    for header in 0..chunk.code.len() - 1 {
        let (destination, invariant_load) = match chunk.code[header] {
            Instruction::ClassConstantGet { destination, cache } => {
                (destination, InvariantLoad::ClassConstant { cache })
            }
            Instruction::LoadConstant {
                destination,
                constant,
            } => (destination, InvariantLoad::Constant { constant }),
            Instruction::LoadInt {
                destination,
                immediate,
            } => (destination, InvariantLoad::Int { immediate }),
            _ => continue,
        };

        let Instruction::JumpUnless {
            comparison,
            left,
            right,
            offset,
        } = chunk.code[header + 1]
        else {
            continue;
        };

        if left != destination && right != destination {
            continue;
        }

        let exit = relative_target(header + 1, i32::from(offset.offset()));
        if exit <= header + 1
            || exit > chunk.code.len()
            || register_is_read_before_write(chunk, destination, header + 2, exit)
        {
            continue;
        }

        let mut has_back_edge = false;
        for (index, instruction) in chunk.code[header + 2..exit].iter().enumerate() {
            let index = header + 2 + index;
            let target = match instruction {
                Instruction::Jump { offset } => relative_target(index, offset.offset()),
                Instruction::IncrementJump { offset, .. } => {
                    relative_target(index, i32::from(offset.offset()))
                }
                _ => continue,
            };

            if target == header {
                has_back_edge = true;
                break;
            }
        }

        if !has_back_edge {
            continue;
        }

        let invariant = Register::new(chunk.register_count);
        chunk.register_count += 1;
        chunk.code[header] = match invariant_load {
            InvariantLoad::ClassConstant { cache } => Instruction::ClassConstantGet {
                destination: invariant,
                cache,
            },
            InvariantLoad::Constant { constant } => Instruction::LoadConstant {
                destination: invariant,
                constant,
            },
            InvariantLoad::Int { immediate } => Instruction::LoadInt {
                destination: invariant,
                immediate,
            },
        };

        chunk.code[header + 1] = Instruction::JumpUnless {
            comparison,
            left: if left == destination { invariant } else { left },
            right: if right == destination {
                invariant
            } else {
                right
            },
            offset,
        };

        for index in header + 2..exit {
            match chunk.code[index] {
                Instruction::Jump { offset }
                    if relative_target(index, offset.offset()) == header =>
                {
                    chunk.code[index] = Instruction::Jump {
                        offset: JumpOffset::new(offset.offset() + 1),
                    };
                }
                Instruction::IncrementJump {
                    target,
                    immediate,
                    offset,
                } if relative_target(index, i32::from(offset.offset())) == header => {
                    chunk.code[index] = Instruction::IncrementJump {
                        target,
                        immediate,
                        offset: ShortJumpOffset::new(offset.offset() + 1),
                    };
                }
                _ => {}
            }
        }
    }

    hoist_lengths(chunk);
}

/// Moves an invariant collection length out of a counted loop when its
/// temporary is consumed once at the beginning of every iteration.
fn hoist_lengths(chunk: &mut Chunk) {
    if chunk.code.len() < 5 {
        return;
    }

    for header in 0..chunk.code.len() - 4 {
        let Instruction::JumpUnless { offset, .. } = chunk.code[header] else {
            continue;
        };

        let exit = relative_target(header, i32::from(offset.offset()));
        if exit <= header + 3 || exit > chunk.code.len() {
            continue;
        }

        let tail = exit - 1;
        let Instruction::IncrementJump {
            offset: back_edge, ..
        } = chunk.code[tail]
        else {
            continue;
        };

        if relative_target(tail, i32::from(back_edge.offset())) != header {
            continue;
        }

        let Instruction::StringLength {
            destination,
            source,
        } = chunk.code[header + 1]
        else {
            continue;
        };

        let Instruction::IntAddAssign {
            source: consumed, ..
        } = chunk.code[header + 2]
        else {
            continue;
        };

        if consumed != destination
            || !register_is_untouched_between(chunk, source, header + 2, exit)
            || !register_is_untouched_between(chunk, destination, header + 3, exit)
        {
            continue;
        }

        let guard = chunk.code[header];
        chunk.code[header] = chunk.code[header + 1];
        chunk.code[header + 1] = match guard {
            Instruction::JumpUnless {
                comparison,
                left,
                right,
                offset,
            } => Instruction::JumpUnless {
                comparison,
                left,
                right,
                offset: ShortJumpOffset::new(offset.offset() - 1),
            },
            // SAFETY: the surrounding invariant makes this path unreachable.
            _ => unsafe { unreachable_invariant("the loop guard was matched") },
        };

        chunk.spans.swap(header, header + 1);

        let Instruction::IncrementJump {
            target,
            immediate,
            offset,
        } = chunk.code[tail]
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the loop back edge was matched") }
        };

        chunk.code[tail] = Instruction::IncrementJump {
            target,
            immediate,
            offset: ShortJumpOffset::new(offset.offset() + 1),
        };
    }
}

#[derive(Clone, Copy)]
enum InvariantLoad {
    ClassConstant { cache: IcSlot },
    Constant { constant: ConstantIndex },
    Int { immediate: ImmediateInt },
}
