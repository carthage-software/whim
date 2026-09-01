//! Selection of closed, side-effect-free numeric counted loops for unboxed
//! execution by the VM.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::NUMERIC_LOOP_REGISTER_LIMIT;
use crate::bytecode::instruction::operands::Comparison as BytecodeComparison;
use crate::bytecode::instruction::operands::Register;

use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;

mod shape;

use crate::optimizer::passes::fuse_numeric_loop::shape::closed_numeric_body;
use crate::optimizer::passes::fuse_numeric_loop::shape::float_write_mask;
use crate::optimizer::passes::fuse_numeric_loop::shape::has_external_entry;
use crate::optimizer::passes::fuse_numeric_loop::shape::ordered;
use crate::optimizer::passes::fuse_numeric_loop::shape::region_profits;
use crate::optimizer::passes::fuse_numeric_loop::shape::writes_pinned_container;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
) {
    if !configuration.fuse_numeric_loop
        || chunk.register_count > NUMERIC_LOOP_REGISTER_LIMIT
        || chunk.code.len() < 3
    {
        return;
    }

    for header in 0..chunk.code.len() - 2 {
        let (comparison, left, right, offset) = match chunk.code[header] {
            Instruction::JumpUnless {
                comparison,
                left,
                right,
                offset,
            }
            | Instruction::IntJumpUnless {
                comparison,
                left,
                right,
                offset,
            }
            | Instruction::IntNumericLoop {
                comparison,
                left,
                right,
                offset,
            } => (comparison, left, right, offset),
            _ => continue,
        };

        if !ordered(comparison) {
            continue;
        }

        let exit = relative_target(header, i32::from(offset.offset()));
        if exit <= header + 1 || exit > chunk.code.len() {
            continue;
        }
        let tail = exit - 1;
        let shape = match chunk.code[tail] {
            Instruction::CounterLoop {
                comparison: tail_comparison,
                counter,
                limit,
                offset: back_edge,
            } => {
                if tail_comparison != comparison
                    || counter != left
                    || limit != right
                    || relative_target(tail, i32::from(back_edge.offset())) != header + 1
                {
                    continue;
                }

                TailShape::Counted {
                    integer_only: false,
                }
            }
            Instruction::IntCounterLoop {
                comparison: tail_comparison,
                counter,
                limit,
                offset: back_edge,
            } => {
                if tail_comparison != comparison
                    || counter != left
                    || limit != right
                    || relative_target(tail, i32::from(back_edge.offset())) != header + 1
                {
                    continue;
                }

                TailShape::Counted { integer_only: true }
            }
            Instruction::Jump { offset: back_edge } => {
                if relative_target(tail, back_edge.offset()) != header {
                    continue;
                }

                TailShape::While {
                    int_header: matches!(
                        chunk.code[header],
                        Instruction::IntJumpUnless { .. } | Instruction::IntNumericLoop { .. }
                    ),
                }
            }
            _ => continue,
        };

        if !closed_numeric_body(chunk, header, tail, exit)
            || !region_profits(chunk, header, tail)
            || has_external_entry(chunk, header, tail)
            || writes_pinned_container(chunk, header, tail)
            || (matches!(shape, TailShape::Counted { .. })
                && chunk.code[header + 1..tail]
                    .iter()
                    .any(|instruction| effect_on(chunk, *instruction, left).writes()))
        {
            continue;
        }

        chunk.code[header] = match shape {
            TailShape::Counted { integer_only: true } => {
                let float_registers = float_write_mask(chunk, header + 1, tail);
                if float_registers != 0
                    && let Ok(descriptor) =
                        chunk.add_prepared_int_loop_descriptor(PreparedIntLoopDescriptor {
                            comparison,
                            counter: left,
                            limit: right,
                            float_registers,
                        })
                {
                    Instruction::PreparedIntNumericLoop { descriptor, offset }
                } else {
                    Instruction::IntNumericLoop {
                        comparison,
                        left,
                        right,
                        offset,
                    }
                }
            }
            TailShape::While { int_header: true } => Instruction::IntNumericLoop {
                comparison,
                left,
                right,
                offset,
            },
            _ => Instruction::NumericLoop {
                comparison,
                left,
                right,
                offset,
            },
        };
    }

    for tail in 2..chunk.code.len() {
        let Instruction::Jump { offset } = chunk.code[tail] else {
            continue;
        };

        let relative = offset.offset();
        if relative >= 0 {
            continue;
        }

        let header = relative_target(tail, relative);
        if header == 0
            || matches!(
                chunk.code[header],
                Instruction::JumpUnless { .. }
                    | Instruction::IntJumpUnless { .. }
                    | Instruction::NumericLoop { .. }
                    | Instruction::IntNumericLoop { .. }
                    | Instruction::PreparedIntNumericLoop { .. }
            )
        {
            continue;
        }

        if !closed_numeric_body(chunk, header - 1, tail, tail + 1)
            || !region_profits(chunk, header - 1, tail)
            || writes_pinned_container(chunk, header - 1, tail)
        {
            continue;
        }

        chunk.code[tail] = Instruction::NumericRegionJump { offset };
    }
}

#[derive(Clone, Copy)]
enum TailShape {
    Counted { integer_only: bool },
    While { int_header: bool },
}
