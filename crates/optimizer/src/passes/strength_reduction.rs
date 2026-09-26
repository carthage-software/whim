//! Reduction of proven integer arithmetic to smaller immediate operations.

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::rewrite::control_flow_targets;
use whim_value::heap::Heap;

use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;
use crate::analysis::Analysis;
use crate::candidates::CandidateSet;
use crate::cfg::is_block_boundary;
use crate::liveness::effect::effect_on;
use crate::liveness::register_is_dead_after;
use crate::operands::for_each_write_register;
use crate::rewrite::plan::RewritePlan;
use crate::type_flow::ConstantValue;
use crate::type_flow::TypeFlow;

pub(crate) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.strength_reduction {
        return;
    }

    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::ARITHMETIC) {
            continue;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) {
                continue;
            }

            let Some(replacement) = reduced_instruction(&analyzed.flow, index, instruction) else {
                continue;
            };

            if analyzed.write(plan, index, replacement) {
                statistics.operations_specialized += 1;
            }
        }
    }
}

pub(in crate::passes) fn optimize_chunk(
    chunk: &mut Chunk,
    heap: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.strength_reduction || chunk.code.is_empty() {
        return;
    }

    let mut replacements = vec![None; chunk.code.len()];
    let flow = TypeFlow::analyze(chunk, &[], false, None, &[], heap);
    for (index, instruction) in chunk.code.iter().copied().enumerate() {
        replacements[index] = reduced_instruction(&flow, index, instruction);
    }

    for (instruction, replacement) in chunk.code.iter_mut().zip(replacements) {
        if let Some(replacement) = replacement {
            *instruction = replacement;
            statistics.operations_specialized += 1;
        }
    }
}

fn reduced_instruction(
    flow: &TypeFlow<'_>,
    index: usize,
    instruction: Instruction,
) -> Option<Instruction> {
    match instruction {
        Instruction::AddImmediate {
            kind: None | Some(IntegerKind::I64),
            destination,
            source,
            immediate,
        }
        | Instruction::SubtractImmediate {
            kind: None | Some(IntegerKind::I64),
            destination,
            source,
            immediate,
        } if immediate.as_int() == 0 && flow.proves(index, source, &TypeDescriptor::Int) => {
            Some(Instruction::Move {
                destination,
                source,
            })
        }
        Instruction::IntegerMultiplyImmediate {
            kind: IntegerKind::I64,
            destination,
            source,
            immediate,
        } if immediate.as_int() == 1 => Some(Instruction::Move {
            destination,
            source,
        }),
        Instruction::Add {
            kind: Some(IntegerKind::I64),
            destination,
            left,
            right,
        } => immediate(flow, index, right)
            .map(|immediate| Instruction::AddImmediate {
                kind: Some(IntegerKind::I64),
                destination,
                source: left,
                immediate,
            })
            .or_else(|| {
                immediate(flow, index, left).map(|immediate| Instruction::AddImmediate {
                    kind: Some(IntegerKind::I64),
                    destination,
                    source: right,
                    immediate,
                })
            }),
        Instruction::Subtract {
            kind: Some(IntegerKind::I64),
            destination,
            left,
            right,
        } => immediate(flow, index, right).map(|immediate| Instruction::SubtractImmediate {
            kind: Some(IntegerKind::I64),
            destination,
            source: left,
            immediate,
        }),
        _ => None,
    }
}

fn immediate(flow: &TypeFlow<'_>, index: usize, register: Register) -> Option<ImmediateInteger> {
    let ConstantValue::Int(value) = flow.constant_value(index, register)? else {
        return None;
    };

    Some(ImmediateInteger::signed(i16::try_from(value).ok()?))
}

#[derive(Clone, Copy, Default)]
struct KnownBits {
    zero: u64,
    one: u64,
    untagged: bool,
}

impl KnownBits {
    fn constant(value: u64) -> Self {
        Self {
            zero: !value,
            one: value,
            untagged: true,
        }
    }

    fn value(self) -> Option<u64> {
        (self.zero | self.one == u64::MAX).then_some(self.one)
    }
}

pub(in crate::passes) fn reduce_bitwise_masks(
    chunk: &mut Chunk,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.strength_reduction
        || !chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::BitwiseAnd { kind: Some(_), .. }))
    {
        return;
    }

    reduce_nested_masks(chunk, statistics);
    let targets = control_flow_targets(chunk);
    let mut bits = vec![KnownBits::default(); usize::from(chunk.register_count)];
    for index in 0..chunk.code.len() {
        if targets.contains(&index) {
            bits.fill(KnownBits::default());
        }
        let instruction = chunk.code[index];
        let read = |register: Register| bits[usize::from(register.index())];
        let result = match instruction {
            Instruction::LoadInteger {
                destination,
                immediate,
                kind,
            } => Some((
                destination,
                KnownBits::constant(match kind {
                    IntegerKind::I64 => i64::from(immediate.as_int()) as u64,
                    IntegerKind::U64 => u64::from(immediate.as_uint()),
                }),
            )),
            Instruction::LoadConstant {
                destination,
                constant,
            } => match &chunk.constants[usize::from(constant.index())] {
                Literal::Int(value) => Some((destination, KnownBits::constant(*value as u64))),
                Literal::Uint(value) => Some((destination, KnownBits::constant(*value))),
                _ => None,
            },
            Instruction::Move {
                destination,
                source,
            }
            | Instruction::MoveOwned {
                destination,
                source,
            } => Some((destination, read(source))),
            Instruction::BitwiseAnd {
                kind: Some(_),
                destination,
                left,
                right,
            } => {
                let a = read(left);
                let b = read(right);
                let source = if a.untagged && a.zero | b.one == u64::MAX {
                    Some(left)
                } else if b.untagged && b.zero | a.one == u64::MAX {
                    Some(right)
                } else {
                    None
                };
                if let Some(source) = source {
                    chunk.code[index] = Instruction::Move {
                        destination,
                        source,
                    };
                    statistics.operations_specialized += 1;
                }
                Some((
                    destination,
                    KnownBits {
                        zero: a.zero | b.zero,
                        one: a.one & b.one,
                        untagged: true,
                    },
                ))
            }
            Instruction::BitwiseOr {
                kind: Some(_),
                destination,
                left,
                right,
            } => Some((
                destination,
                KnownBits {
                    zero: read(left).zero & read(right).zero,
                    one: read(left).one | read(right).one,
                    untagged: true,
                },
            )),
            Instruction::BitwiseXor {
                kind: Some(_),
                destination,
                left,
                right,
            } => {
                let a = read(left);
                let b = read(right);
                Some((
                    destination,
                    KnownBits {
                        zero: (a.zero & b.zero) | (a.one & b.one),
                        one: (a.zero & b.one) | (a.one & b.zero),
                        untagged: true,
                    },
                ))
            }
            Instruction::ShiftLeft {
                kind: Some(_),
                destination,
                left,
                right,
            } => read(right)
                .value()
                .filter(|shift| *shift < 64)
                .map(|shift| {
                    let a = read(left);
                    (
                        destination,
                        KnownBits {
                            zero: (a.zero << shift) | !(u64::MAX << shift),
                            one: a.one << shift,
                            untagged: true,
                        },
                    )
                }),
            Instruction::ShiftRight {
                kind: Some(kind),
                destination,
                left,
                right,
            } => read(right)
                .value()
                .filter(|shift| *shift < 64)
                .map(|shift| {
                    let a = read(left);
                    let high = !(u64::MAX >> shift);
                    let mut result = KnownBits {
                        zero: a.zero >> shift,
                        one: a.one >> shift,
                        untagged: true,
                    };
                    if kind == IntegerKind::U64 || a.zero & (1 << 63) != 0 {
                        result.zero |= high;
                    } else if a.one & (1 << 63) != 0 {
                        result.one |= high;
                    }
                    (destination, result)
                }),
            _ => None,
        };
        if !preserves_integer_facts(instruction)
            || !for_each_write_register(instruction, |register| {
                bits[usize::from(register.index())] = KnownBits::default()
            })
        {
            bits.fill(KnownBits::default());
        }
        if let Some((destination, result)) = result {
            bits[usize::from(destination.index())] = result;
        }
        if is_block_boundary(instruction) {
            bits.fill(KnownBits::default());
        }
    }
}

fn preserves_integer_facts(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::LoadInteger { .. }
            | Instruction::LoadConstant { .. }
            | Instruction::Move { .. }
            | Instruction::MoveOwned { .. }
            | Instruction::BitwiseAnd { .. }
            | Instruction::BitwiseOr { .. }
            | Instruction::BitwiseXor { .. }
            | Instruction::BitwiseNot { .. }
            | Instruction::ShiftLeft { .. }
            | Instruction::ShiftRight { .. }
            | Instruction::Add { .. }
            | Instruction::Subtract { .. }
            | Instruction::Multiply { .. }
            | Instruction::Modulo { .. }
            | Instruction::AddImmediate { .. }
            | Instruction::SubtractImmediate { .. }
            | Instruction::IntegerMultiplyImmediate { .. }
            | Instruction::IntegerModuloImmediate { .. }
            | Instruction::IntegerAddAssign { .. }
            | Instruction::Step { .. }
    )
}

fn reduce_nested_masks(chunk: &mut Chunk, statistics: &mut OptimizationStatistics) {
    if !chunk.catch_table.is_empty() {
        return;
    }
    let targets = control_flow_targets(chunk);
    let mut start = 0;
    for index in 0..chunk.code.len() {
        if targets.contains(&index) {
            start = index;
        }
        if !preserves_integer_facts(chunk.code[index]) {
            start = index + 1;
            continue;
        }
        let Instruction::BitwiseAnd {
            kind: Some(kind),
            left,
            right,
            ..
        } = chunk.code[index]
        else {
            continue;
        };
        let start = start.max(index.saturating_sub(64));
        let Some((subject, mask)) = literal_mask(chunk, start, index, kind, left, right) else {
            continue;
        };
        let mut pending = vec![(subject, index)];
        for _ in 0..8 {
            let Some((register, consumer)) = pending.pop() else {
                break;
            };
            if register
                .index()
                .wrapping_sub(chunk.parameter_register_start)
                < chunk.parameter_register_count
                || chunk.trace_argument_registers.contains(&register)
            {
                continue;
            }
            let Some(producer) = last_write(chunk, start, consumer, register) else {
                continue;
            };
            if chunk.code[producer + 1..consumer]
                .iter()
                .any(|instruction| effect_on(chunk, *instruction, register).reads())
            {
                continue;
            }
            let Some((producer_kind, left, right)) = bitwise_operands(chunk.code[producer]) else {
                continue;
            };
            if producer_kind != kind {
                continue;
            }
            if !effect_on(chunk, chunk.code[consumer], register).writes()
                && !register_is_dead_after(chunk, register, consumer + 1)
            {
                continue;
            }
            if matches!(chunk.code[producer], Instruction::BitwiseAnd { .. })
                && let Some((source, inner_mask)) =
                    literal_mask(chunk, start, producer, kind, left, right)
                && mask & !inner_mask == 0
                && last_write(chunk, start, producer, source)
                    .and_then(|at| bitwise_operands(chunk.code[at]))
                    .is_some_and(|(source_kind, _, _)| source_kind == kind)
            {
                chunk.code[producer] = Instruction::Move {
                    destination: register,
                    source,
                };
                statistics.operations_specialized += 1;
            }
            pending.push((left, producer));
            pending.push((right, producer));
        }
    }
}

fn last_write(chunk: &Chunk, start: usize, end: usize, register: Register) -> Option<usize> {
    (start..end)
        .rev()
        .find(|index| effect_on(chunk, chunk.code[*index], register).writes())
}

fn bitwise_operands(instruction: Instruction) -> Option<(IntegerKind, Register, Register)> {
    match instruction {
        Instruction::BitwiseAnd {
            kind: Some(kind),
            left,
            right,
            ..
        }
        | Instruction::BitwiseOr {
            kind: Some(kind),
            left,
            right,
            ..
        }
        | Instruction::BitwiseXor {
            kind: Some(kind),
            left,
            right,
            ..
        } => Some((kind, left, right)),
        _ => None,
    }
}

fn literal_mask(
    chunk: &Chunk,
    start: usize,
    index: usize,
    kind: IntegerKind,
    left: Register,
    right: Register,
) -> Option<(Register, u64)> {
    for (source, register) in [(left, right), (right, left)] {
        let Some(at) = last_write(chunk, start, index, register) else {
            continue;
        };
        let value = match chunk.code[at] {
            Instruction::LoadInteger {
                kind: loaded_kind,
                immediate,
                ..
            } if loaded_kind == kind => match kind {
                IntegerKind::I64 => i64::from(immediate.as_int()) as u64,
                IntegerKind::U64 => u64::from(immediate.as_uint()),
            },
            Instruction::LoadConstant { constant, .. } => {
                match (&chunk.constants[usize::from(constant.index())], kind) {
                    (Literal::Int(value), IntegerKind::I64) => *value as u64,
                    (Literal::Uint(value), IntegerKind::U64) => *value,
                    _ => continue,
                }
            }
            _ => continue,
        };
        return Some((source, value));
    }
    None
}
