use whim_bytecode::REFERENCE_REGISTER_LIMIT;
use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::CatchEntry;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::Comparison;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::instruction::operands::ShortJumpOffset;
use whim_bytecode::reference_registers;
use whim_bytecode::verify::verify;
use whim_span::Position;
use whim_span::Span;

use super::optimize_chunk;
use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;

const TARGET: Register = Register::new(0);
const TEMPORARY: Register = Register::new(1);
const OTHER: Register = Register::new(2);

fn fixture(kind: IntegerKind, immediate: ImmediateInteger) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.register_count = 3;
    chunk.local_register_count = 1;
    for instruction in [
        Instruction::LoadInteger {
            destination: TEMPORARY,
            immediate,
            kind,
        },
        Instruction::IntegerAddAssign {
            target: TARGET,
            source: TEMPORARY,
            kind,
        },
        Instruction::ReturnScalarUnchecked { source: TARGET },
    ] {
        chunk.emit(instruction, Span::zero());
    }
    chunk
}

fn rewrite(chunk: &mut Chunk) {
    optimize_chunk(
        chunk,
        OptimizationConfiguration::default(),
        &mut OptimizationStatistics::default(),
    );
}

fn assert_unchanged(mut chunk: Chunk) {
    let before = chunk.code.clone();
    rewrite(&mut chunk);
    assert_eq!(chunk.code, before);
}

#[test]
fn signed_and_unsigned_assignments_keep_immediate_bits_and_consumer_spans() {
    for (kind, immediate) in [
        (IntegerKind::I64, ImmediateInteger::signed(i16::MIN)),
        (IntegerKind::I64, ImmediateInteger::signed(i16::MAX)),
        (IntegerKind::U64, ImmediateInteger::unsigned(0)),
        (IntegerKind::U64, ImmediateInteger::unsigned(32768)),
        (IntegerKind::U64, ImmediateInteger::unsigned(u16::MAX)),
    ] {
        let mut chunk = fixture(kind, immediate);
        let span = Span::new(Position::new(10), Position::new(25));
        chunk.spans[1] = span;
        rewrite(&mut chunk);
        assert_eq!(
            chunk.code,
            vec![
                Instruction::AddImmediate {
                    destination: TARGET,
                    source: TARGET,
                    immediate,
                    kind: Some(kind),
                },
                Instruction::ReturnScalarUnchecked { source: TARGET },
            ]
        );
        assert_eq!(chunk.spans[0], span);
    }
}

#[test]
fn mismatched_kinds_sources_and_aliased_targets_keep_the_load() {
    for kind in [IntegerKind::I64, IntegerKind::U64] {
        for (target, source, operand_kind) in [
            (
                TARGET,
                TEMPORARY,
                if kind == IntegerKind::I64 {
                    IntegerKind::U64
                } else {
                    IntegerKind::I64
                },
            ),
            (TARGET, OTHER, kind),
            (TEMPORARY, TEMPORARY, kind),
        ] {
            let mut chunk = fixture(kind, ImmediateInteger::unsigned(5));
            chunk.code[1] = Instruction::IntegerAddAssign {
                target,
                source,
                kind: operand_kind,
            };
            assert_unchanged(chunk);
        }
    }
}

#[test]
fn live_temporaries_and_consumer_branch_targets_keep_the_load() {
    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    chunk.code[2] = Instruction::ReturnScalarUnchecked { source: TEMPORARY };
    assert_unchanged(chunk);

    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    chunk.emit(
        Instruction::Jump {
            offset: JumpOffset::new(-2),
        },
        Span::zero(),
    );
    assert_unchanged(chunk);
}

#[test]
fn locals_trace_registers_and_reference_temporaries_keep_the_load() {
    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    chunk.local_register_count = 2;
    assert_unchanged(chunk);

    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    chunk.trace_argument_registers = vec![TEMPORARY];
    assert_unchanged(chunk);

    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    chunk.reference_register_mask = 1 << TEMPORARY.index();
    assert_unchanged(chunk);

    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    chunk.code.insert(
        0,
        Instruction::Move {
            destination: TEMPORARY,
            source: OTHER,
        },
    );
    chunk.spans.insert(0, Span::zero());
    assert_eq!(chunk.reference_register_mask, 0);
    assert_unchanged(chunk);

    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    let high = Register::new(REFERENCE_REGISTER_LIMIT);
    chunk.register_count = REFERENCE_REGISTER_LIMIT + 1;
    let Instruction::LoadInteger { destination, .. } = &mut chunk.code[0] else {
        unreachable!()
    };
    *destination = high;
    let Instruction::IntegerAddAssign { source, .. } = &mut chunk.code[1] else {
        unreachable!()
    };
    *source = high;
    assert_unchanged(chunk);
}

#[test]
fn owned_moves_from_incoming_captures_keep_the_load() {
    for chained in [false, true] {
        let capture = Register::new(1);
        let temporary = Register::new(2);
        let intermediary = Register::new(3);
        let mut chunk = Chunk::new();
        chunk.register_count = 4;
        chunk.local_register_count = 2;
        chunk.parameter_register_count = 1;
        chunk.emit(
            Instruction::MoveOwned {
                destination: if chained { intermediary } else { temporary },
                source: capture,
            },
            Span::zero(),
        );
        if chained {
            chunk.emit(
                Instruction::MoveOwned {
                    destination: temporary,
                    source: intermediary,
                },
                Span::zero(),
            );
        }
        for instruction in [
            Instruction::LoadInteger {
                destination: temporary,
                immediate: ImmediateInteger::unsigned(5),
                kind: IntegerKind::U64,
            },
            Instruction::IntegerAddAssign {
                target: TARGET,
                source: temporary,
                kind: IntegerKind::U64,
            },
            Instruction::ReturnScalarUnchecked { source: TARGET },
        ] {
            chunk.emit(instruction, Span::zero());
        }
        assert_eq!(chunk.reference_register_mask, 0);
        assert_ne!(
            reference_registers::mask(&chunk) & (1u64 << temporary.index()),
            0
        );
        assert_unchanged(chunk);
    }
}

#[test]
fn protected_consumers_keep_the_load_even_when_the_normal_path_is_dead() {
    for returned in [TARGET, TEMPORARY] {
        let mut chunk = fixture(IntegerKind::I64, ImmediateInteger::signed(5));
        chunk.emit(
            Instruction::ReturnScalarUnchecked { source: returned },
            Span::zero(),
        );
        let type_descriptor = chunk.add_type_descriptor(TypeDescriptor::Mixed).unwrap();
        chunk.catch_table.push(CatchEntry {
            start: 0,
            end: 2,
            handler: 3,
            type_descriptor,
            temporary_floor: 3,
            binding: None,
        });
        assert_unchanged(chunk);
    }
}

#[test]
fn a_loop_body_fuses_after_its_reference_mask_is_refined() {
    let mut chunk = fixture(IntegerKind::U64, ImmediateInteger::unsigned(5));
    chunk.code[2] = Instruction::Jump {
        offset: JumpOffset::new(-2),
    };
    chunk.reference_register_mask = 1 << TEMPORARY.index();
    let before = chunk.code.clone();
    rewrite(&mut chunk);
    assert_eq!(chunk.code, before);

    chunk.reference_register_mask = 0;
    rewrite(&mut chunk);
    assert_eq!(chunk.code.len(), 2);
    assert!(matches!(chunk.code[0], Instruction::AddImmediate {
        destination: TARGET,
        source: TARGET,
        kind: Some(IntegerKind::U64),
        immediate,
    } if immediate.as_uint() == 5));
    assert_eq!(
        chunk.code[1],
        Instruction::Jump {
            offset: JumpOffset::new(-1)
        }
    );
}

fn branch_fixture(kind: IntegerKind, reversed: bool, live_target: bool) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.register_count = 2;
    chunk.local_register_count = 1;
    chunk.parameter_register_count = 1;
    let (left, right) = if reversed {
        (TEMPORARY, TARGET)
    } else {
        (TARGET, TEMPORARY)
    };
    let comparison = Comparison::LessThan;
    let offset = ShortJumpOffset::new(2);
    for instruction in [
        Instruction::LoadInteger {
            destination: TEMPORARY,
            immediate: ImmediateInteger::unsigned(5),
            kind,
        },
        if kind == IntegerKind::I64 {
            Instruction::IntJumpUnless {
                comparison,
                left,
                right,
                offset,
            }
        } else {
            Instruction::UintJumpUnless {
                comparison,
                left,
                right,
                offset,
            }
        },
        Instruction::ReturnNull,
        if live_target {
            Instruction::ReturnScalarUnchecked { source: TEMPORARY }
        } else {
            Instruction::ReturnNull
        },
    ] {
        chunk.emit(instruction, Span::zero());
    }
    verify(&chunk).unwrap();
    chunk
}

#[test]
fn branch_literals_live_on_either_edge_keep_the_load() {
    for kind in [IntegerKind::I64, IntegerKind::U64] {
        for reversed in [false, true] {
            let chunk = branch_fixture(kind, reversed, true);
            assert_unchanged(chunk);

            let mut chunk = branch_fixture(kind, reversed, false);
            chunk.code[2] = Instruction::ReturnScalarUnchecked { source: TEMPORARY };
            verify(&chunk).unwrap();
            assert_unchanged(chunk);
        }
    }
}

#[test]
fn branch_literals_dead_on_both_edges_still_fuse() {
    for kind in [IntegerKind::I64, IntegerKind::U64] {
        for reversed in [false, true] {
            let mut chunk = branch_fixture(kind, reversed, false);
            rewrite(&mut chunk);
            verify(&chunk).unwrap();
            assert_eq!(chunk.code.len(), 3);
            let expected = if reversed {
                Comparison::LessThan.reversed()
            } else {
                Comparison::LessThan
            };
            match chunk.code[0] {
                Instruction::IntJumpUnlessImmediate {
                    comparison,
                    source: TARGET,
                    immediate,
                    offset,
                } if kind == IntegerKind::I64 => {
                    assert_eq!(comparison, expected);
                    assert_eq!(immediate.value(), 5);
                    assert_eq!(offset.offset(), 2);
                }
                Instruction::UintJumpUnlessImmediate {
                    comparison,
                    source: TARGET,
                    immediate,
                    offset,
                } if kind == IntegerKind::U64 => {
                    assert_eq!(comparison, expected);
                    assert_eq!(immediate.value(), 5);
                    assert_eq!(offset.offset(), 2);
                }
                instruction => panic!("unexpected fused branch: {instruction:?}"),
            }
        }
    }
}

#[test]
fn overwriting_consumers_keep_literals_read_by_catch_handlers() {
    let mut chunk = branch_fixture(IntegerKind::I64, false, true);
    chunk.local_register_count = 2;
    chunk.code[1] = Instruction::Add {
        destination: TEMPORARY,
        left: TARGET,
        right: TEMPORARY,
        kind: Some(IntegerKind::I64),
    };
    let type_descriptor = chunk.add_type_descriptor(TypeDescriptor::Mixed).unwrap();
    chunk.catch_table.push(CatchEntry {
        start: 1,
        end: 2,
        handler: 3,
        type_descriptor,
        temporary_floor: 2,
        binding: None,
    });
    verify(&chunk).unwrap();
    assert_unchanged(chunk);
}

#[test]
fn integer_consumers_can_overwrite_their_literal_registers() {
    for kind in [IntegerKind::I64, IntegerKind::U64] {
        let mut chunk = branch_fixture(kind, false, false);
        chunk.code[1] = Instruction::Add {
            destination: TEMPORARY,
            left: TARGET,
            right: TEMPORARY,
            kind: Some(kind),
        };
        chunk.code[2] = Instruction::ReturnScalarUnchecked { source: TEMPORARY };
        verify(&chunk).unwrap();
        rewrite(&mut chunk);
        verify(&chunk).unwrap();
        assert_eq!(chunk.code.len(), 3);
        assert!(matches!(
            chunk.code[0],
            Instruction::AddImmediate {
                destination: TEMPORARY,
                source: TARGET,
                kind: Some(actual),
                immediate,
            } if actual == kind && immediate.as_uint() == 5
        ));
    }
}
