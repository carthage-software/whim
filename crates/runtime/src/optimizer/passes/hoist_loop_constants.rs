//! Hoisting of directly consumed scalar literals from natural loops.

use whim_span::Span;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::rewrite::compact;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::cfg::is_block_boundary;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::register_is_dead_after;
use crate::optimizer::rewrite::splice::can_insert_straight_line_before;
use crate::optimizer::rewrite::splice::insert_straight_line_before;
use crate::unwrap_result_invariant;

pub(in crate::optimizer::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
) {
    if !configuration.licm || chunk.code.len() < 3 || !chunk.catch_table.is_empty() {
        return;
    }

    while hoist_one_loop(chunk) {}
}

fn hoist_one_loop(chunk: &mut Chunk) -> bool {
    for tail in 1..chunk.code.len() {
        let Some(header) = backward_target(chunk.code[tail], tail) else {
            continue;
        };
        if header >= tail
            || (header != 0 && is_block_boundary(chunk.code[header - 1]))
            || has_external_entry(chunk, header, tail)
            || !can_insert_straight_line_before(chunk, header)
        {
            continue;
        }

        let mut candidates = Vec::new();
        let available = usize::from(u16::MAX - chunk.register_count);
        for index in header..tail {
            if candidates.len() == available || index + 1 >= tail {
                break;
            }
            let Some((destination, load)) = scalar_load(chunk, chunk.code[index]) else {
                continue;
            };
            if !register_is_dead_after(chunk, destination, index + 2) {
                continue;
            }

            // SAFETY: register capacity caps the count at `u16`.
            let invariant = Register::new(
                chunk.register_count
                    + unsafe {
                        unwrap_result_invariant(
                            u16::try_from(candidates.len()),
                            "candidate count was bounded by register capacity",
                        )
                    },
            );
            let Some(consumer) = replace_binary_read(chunk.code[index + 1], destination, invariant)
            else {
                continue;
            };
            if is_join_point(chunk, index + 1) {
                continue;
            }
            candidates.push(Candidate {
                index,
                load: load.with_destination(invariant),
                consumer,
                span: chunk.spans[index],
            });
        }
        if candidates.is_empty() {
            continue;
        }

        let mut remove = vec![false; chunk.code.len()];
        let mut preheader = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            chunk.code[candidate.index + 1] = candidate.consumer;
            remove[candidate.index] = true;
            preheader.push((candidate.load, candidate.span));
        }
        // SAFETY: the preheader count is bounded by register capacity, so it fits u16.
        chunk.register_count += unsafe {
            unwrap_result_invariant(
                u16::try_from(preheader.len()),
                "candidate count was bounded by register capacity",
            )
        };

        compact(chunk, &remove);
        assert!(
            insert_straight_line_before(chunk, header, &preheader),
            "natural-loop preheader insertion unexpectedly crossed a control-flow edge"
        );

        return true;
    }

    false
}

fn backward_target(instruction: Instruction, source: usize) -> Option<usize> {
    let target = match instruction {
        Instruction::Jump { offset } | Instruction::NumericRegionJump { offset } => {
            relative_target(source, offset.offset())
        }
        Instruction::IncrementJump { offset, .. }
        | Instruction::CounterLoop { offset, .. }
        | Instruction::IntCounterLoop { offset, .. }
        | Instruction::IntStepLoop { offset, .. } => {
            relative_target(source, i32::from(offset.offset()))
        }
        _ => return None,
    };
    (target < source).then_some(target)
}

fn is_join_point(chunk: &Chunk, position: usize) -> bool {
    let mut targets = Vec::new();
    for source in 0..chunk.code.len() {
        if source + 1 == position {
            continue;
        }
        targets.clear();
        successors(chunk, source, &mut targets);
        if targets.contains(&position) {
            return true;
        }
    }
    false
}

fn has_external_entry(chunk: &Chunk, header: usize, tail: usize) -> bool {
    let mut targets = Vec::new();
    for source in 0..chunk.code.len() {
        if source >= header && source <= tail {
            continue;
        }
        targets.clear();
        successors(chunk, source, &mut targets);
        for &target in &targets {
            let initial_fallthrough =
                source + 1 == header && target == header && !is_block_boundary(chunk.code[source]);
            if target >= header && target <= tail && !initial_fallthrough {
                return true;
            }
        }
    }

    chunk
        .catch_table
        .iter()
        .any(|entry| (entry.handler as usize) >= header && (entry.handler as usize) <= tail)
}

#[derive(Clone, Copy)]
struct Candidate {
    index: usize,
    load: Instruction,
    consumer: Instruction,
    span: Span,
}

#[derive(Clone, Copy)]
enum ScalarLoad {
    Int(ImmediateInt),
    Constant(ConstantIndex),
}

impl ScalarLoad {
    fn with_destination(self, destination: Register) -> Instruction {
        match self {
            Self::Int(immediate) => Instruction::LoadInt {
                destination,
                immediate,
            },
            Self::Constant(constant) => Instruction::LoadConstant {
                destination,
                constant,
            },
        }
    }
}

fn scalar_load(chunk: &Chunk, instruction: Instruction) -> Option<(Register, ScalarLoad)> {
    match instruction {
        Instruction::LoadInt {
            destination,
            immediate,
        } => Some((destination, ScalarLoad::Int(immediate))),
        Instruction::LoadConstant {
            destination,
            constant,
        } if matches!(
            chunk.constants[usize::from(constant.index())],
            Literal::Null
                | Literal::Bool(_)
                | Literal::Int(_)
                | Literal::Float(_)
                | Literal::String(_)
        ) =>
        {
            Some((destination, ScalarLoad::Constant(constant)))
        }
        _ => None,
    }
}

fn replace_binary_read(
    instruction: Instruction,
    expected: Register,
    replacement: Register,
) -> Option<Instruction> {
    macro_rules! replace {
        ($variant:ident, $destination:ident, $left:ident, $right:ident) => {
            (($left == expected) || ($right == expected)).then_some(Instruction::$variant {
                destination: $destination,
                left: if $left == expected {
                    replacement
                } else {
                    $left
                },
                right: if $right == expected {
                    replacement
                } else {
                    $right
                },
            })
        };
    }

    match instruction {
        Instruction::Add {
            destination,
            left,
            right,
        } => replace!(Add, destination, left, right),
        Instruction::Concatenate {
            destination,
            left,
            right,
        } => replace!(Concatenate, destination, left, right),
        Instruction::Subtract {
            destination,
            left,
            right,
        } => replace!(Subtract, destination, left, right),
        Instruction::Multiply {
            destination,
            left,
            right,
        } => replace!(Multiply, destination, left, right),
        Instruction::Divide {
            destination,
            left,
            right,
        } => replace!(Divide, destination, left, right),
        Instruction::Modulo {
            destination,
            left,
            right,
        } => replace!(Modulo, destination, left, right),
        Instruction::Power {
            destination,
            left,
            right,
        } => replace!(Power, destination, left, right),
        Instruction::BitwiseAnd {
            destination,
            left,
            right,
        } => replace!(BitwiseAnd, destination, left, right),
        Instruction::BitwiseOr {
            destination,
            left,
            right,
        } => replace!(BitwiseOr, destination, left, right),
        Instruction::BitwiseXor {
            destination,
            left,
            right,
        } => replace!(BitwiseXor, destination, left, right),
        Instruction::ShiftLeft {
            destination,
            left,
            right,
        } => replace!(ShiftLeft, destination, left, right),
        Instruction::ShiftRight {
            destination,
            left,
            right,
        } => replace!(ShiftRight, destination, left, right),
        Instruction::Equal {
            destination,
            left,
            right,
        } => replace!(Equal, destination, left, right),
        Instruction::NotEqual {
            destination,
            left,
            right,
        } => replace!(NotEqual, destination, left, right),
        Instruction::LessThan {
            destination,
            left,
            right,
        } => replace!(LessThan, destination, left, right),
        Instruction::LessThanOrEqual {
            destination,
            left,
            right,
        } => replace!(LessThanOrEqual, destination, left, right),
        Instruction::GreaterThan {
            destination,
            left,
            right,
        } => replace!(GreaterThan, destination, left, right),
        Instruction::GreaterThanOrEqual {
            destination,
            left,
            right,
        } => replace!(GreaterThanOrEqual, destination, left, right),
        Instruction::Compare {
            destination,
            left,
            right,
        } => replace!(Compare, destination, left, right),
        Instruction::IntAdd {
            destination,
            left,
            right,
        } => replace!(IntAdd, destination, left, right),
        Instruction::IntSubtract {
            destination,
            left,
            right,
        } => replace!(IntSubtract, destination, left, right),
        Instruction::IntMultiply {
            destination,
            left,
            right,
        } => replace!(IntMultiply, destination, left, right),
        Instruction::IntModulo {
            destination,
            left,
            right,
        } => replace!(IntModulo, destination, left, right),
        Instruction::IntBitwiseAnd {
            destination,
            left,
            right,
        } => replace!(IntBitwiseAnd, destination, left, right),
        Instruction::IntBitwiseOr {
            destination,
            left,
            right,
        } => replace!(IntBitwiseOr, destination, left, right),
        Instruction::IntBitwiseXor {
            destination,
            left,
            right,
        } => replace!(IntBitwiseXor, destination, left, right),
        Instruction::IntShiftLeft {
            destination,
            left,
            right,
        } => replace!(IntShiftLeft, destination, left, right),
        Instruction::IntShiftRight {
            destination,
            left,
            right,
        } => replace!(IntShiftRight, destination, left, right),
        Instruction::FloatAdd {
            destination,
            left,
            right,
        } => replace!(FloatAdd, destination, left, right),
        Instruction::FloatSubtract {
            destination,
            left,
            right,
        } => replace!(FloatSubtract, destination, left, right),
        Instruction::FloatMultiply {
            destination,
            left,
            right,
        } => replace!(FloatMultiply, destination, left, right),
        _ => None,
    }
}
