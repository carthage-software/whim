//! Hoisting of directly consumed scalar literals from natural loops.

use whim_base::unwrap_result_invariant;
use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ConstantIndex;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::rewrite::compact;
use whim_bytecode::rewrite::relative_target;

use crate::OptimizationConfiguration;
use crate::cfg::is_block_boundary;
use crate::cfg::successors;
use crate::liveness::register_is_dead_after;
use crate::operands::replace_read_register;
use crate::rewrite::splice::can_insert_straight_line_before;
use crate::rewrite::splice::insert_straight_line_before;

pub(in crate::passes) fn optimize_chunk(
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
        let mut loads = Vec::new();
        let available = usize::from(u16::MAX - chunk.register_count);
        for index in header..tail {
            if index + 1 >= tail {
                break;
            }
            let Some((destination, load)) = scalar_load(chunk, chunk.code[index]) else {
                continue;
            };
            if !register_is_dead_after(chunk, destination, index + 2) {
                continue;
            }

            let existing = loads.iter().position(|(previous, _, _)| *previous == load);
            let position = existing.unwrap_or(loads.len());
            if position == available {
                continue;
            }

            // SAFETY: register capacity caps the count at `u16`.
            let invariant = Register::new(
                chunk.register_count
                    + unsafe {
                        unwrap_result_invariant(
                            u16::try_from(position),
                            "candidate count was bounded by register capacity",
                        )
                    },
            );

            let Some(consumer) = replace_scalar_read(chunk.code[index + 1], destination, invariant)
            else {
                continue;
            };
            if is_join_point(chunk, index + 1) {
                continue;
            }
            if existing.is_none() {
                loads.push((load, invariant, chunk.spans[index]));
            }
            candidates.push(Candidate { index, consumer });
        }
        if candidates.is_empty() {
            continue;
        }

        let mut remove = vec![false; chunk.code.len()];
        for candidate in candidates {
            chunk.code[candidate.index + 1] = candidate.consumer;
            remove[candidate.index] = true;
        }
        let preheader = loads
            .into_iter()
            .map(|(load, destination, span)| (load.with_destination(destination), span))
            .collect::<Vec<_>>();
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
        | Instruction::UintCounterLoop { offset, .. }
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
    consumer: Instruction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScalarLoad {
    Integer(ImmediateInteger, IntegerKind),
    Constant(ConstantIndex),
}

impl ScalarLoad {
    fn with_destination(self, destination: Register) -> Instruction {
        match self {
            Self::Integer(immediate, kind) => Instruction::LoadInteger {
                kind,
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
        Instruction::LoadInteger {
            kind,
            destination,
            immediate,
        } => Some((destination, ScalarLoad::Integer(immediate, kind))),
        Instruction::LoadConstant {
            destination,
            constant,
        } if matches!(
            chunk.constants[usize::from(constant.index())],
            Literal::Null
                | Literal::Bool(_)
                | Literal::Int(_)
                | Literal::Uint(_)
                | Literal::Float(_)
                | Literal::String(_)
        ) =>
        {
            Some((destination, ScalarLoad::Constant(constant)))
        }
        _ => None,
    }
}

fn replace_scalar_read(
    instruction: Instruction,
    expected: Register,
    replacement: Register,
) -> Option<Instruction> {
    macro_rules! replace {
        ($variant:ident, $destination:ident, $left:ident, $right:ident $(, $kind:expr)?) => {
            (($left == expected) || ($right == expected)).then_some(Instruction::$variant {
                destination: $destination,
                $(kind: $kind,)?
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
        Instruction::IndexGet { index, .. }
        | Instruction::VecIndexGet { index, .. }
        | Instruction::DictIndexGetIntKey { index, .. }
        | Instruction::DictIndexGetUintKey { index, .. }
        | Instruction::DictIndexGetStringKey { index, .. }
        | Instruction::StringIndexGet { index, .. }
            if index == expected =>
        {
            replace_read_register(instruction, expected, replacement)
        }
        Instruction::Concatenate {
            destination,
            left,
            right,
        } => replace!(Concatenate, destination, left, right),
        Instruction::Divide {
            destination,
            left,
            right,
        } => replace!(Divide, destination, left, right),
        Instruction::Power {
            destination,
            left,
            right,
        } => replace!(Power, destination, left, right),
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
        Instruction::Add {
            destination,
            left,
            right,
            kind,
        } => replace!(Add, destination, left, right, kind),
        Instruction::Subtract {
            destination,
            left,
            right,
            kind,
        } => replace!(Subtract, destination, left, right, kind),
        Instruction::Multiply {
            destination,
            left,
            right,
            kind,
        } => replace!(Multiply, destination, left, right, kind),
        Instruction::Modulo {
            destination,
            left,
            right,
            kind,
        } => replace!(Modulo, destination, left, right, kind),
        Instruction::BitwiseAnd {
            destination,
            left,
            right,
            kind,
        } => replace!(BitwiseAnd, destination, left, right, kind),
        Instruction::BitwiseOr {
            destination,
            left,
            right,
            kind,
        } => replace!(BitwiseOr, destination, left, right, kind),
        Instruction::BitwiseXor {
            destination,
            left,
            right,
            kind,
        } => replace!(BitwiseXor, destination, left, right, kind),
        Instruction::ShiftLeft {
            destination,
            left,
            right,
            kind,
        } => replace!(ShiftLeft, destination, left, right, kind),
        Instruction::ShiftRight {
            destination,
            left,
            right,
            kind,
        } => replace!(ShiftRight, destination, left, right, kind),
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
