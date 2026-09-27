use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::CatchEntry;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::Comparison;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::instruction::operands::ShortJumpOffset;
use whim_bytecode::verify::verify;
use whim_span::Span;

use super::optimize_chunk;
use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;

const SOURCE: Register = Register::new(0);
const TEMPORARY: Register = Register::new(1);
const RESULT: Register = Register::new(2);

fn branch_fixture(reversed: bool, live_target: bool) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.register_count = 3;
    chunk.local_register_count = 1;
    chunk.parameter_register_count = 1;
    let constant = chunk.add_constant(Literal::Float(5.5)).unwrap();
    let (left, right) = if reversed {
        (TEMPORARY, SOURCE)
    } else {
        (SOURCE, TEMPORARY)
    };
    for instruction in [
        Instruction::LoadConstant {
            destination: TEMPORARY,
            constant,
        },
        Instruction::JumpUnless {
            comparison: Comparison::LessThan,
            left,
            right,
            offset: ShortJumpOffset::new(2),
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

fn rewrite(chunk: &mut Chunk) {
    optimize_chunk(
        chunk,
        OptimizationConfiguration::default(),
        &mut OptimizationStatistics::default(),
    );
    verify(chunk).unwrap();
}

fn assert_unchanged(mut chunk: Chunk) {
    verify(&chunk).unwrap();
    let before = chunk.code.clone();
    rewrite(&mut chunk);
    assert_eq!(chunk.code, before);
}

#[test]
fn branch_literals_live_on_either_edge_keep_the_load() {
    for reversed in [false, true] {
        assert_unchanged(branch_fixture(reversed, true));
        let mut chunk = branch_fixture(reversed, false);
        chunk.code[2] = Instruction::ReturnScalarUnchecked { source: TEMPORARY };
        assert_unchanged(chunk);
    }
}

#[test]
fn branch_literals_dead_on_both_edges_still_fuse() {
    for reversed in [false, true] {
        let mut chunk = branch_fixture(reversed, false);
        rewrite(&mut chunk);
        assert_eq!(chunk.code.len(), 3);
        let Instruction::JumpUnlessConstant {
            comparison,
            source: SOURCE,
            constant,
            offset,
        } = chunk.code[0]
        else {
            panic!("unexpected fused branch: {:?}", chunk.code);
        };
        assert_eq!(
            comparison,
            if reversed {
                Comparison::LessThan.reversed()
            } else {
                Comparison::LessThan
            }
        );
        assert!(matches!(
            chunk.constants[usize::from(constant.index())],
            Literal::Float(value) if value == 5.5
        ));
        assert_eq!(offset.offset(), 2);
    }
}

#[test]
fn branch_literals_live_only_in_catch_handlers_keep_the_load() {
    let mut chunk = branch_fixture(false, false);
    chunk.local_register_count = 2;
    chunk.emit(
        Instruction::ReturnScalarUnchecked { source: TEMPORARY },
        Span::zero(),
    );
    let type_descriptor = chunk.add_type_descriptor(TypeDescriptor::Mixed).unwrap();
    chunk.catch_table.push(CatchEntry {
        start: 1,
        end: 2,
        handler: 4,
        type_descriptor,
        temporary_floor: 2,
        binding: None,
    });
    assert_unchanged(chunk);
}

#[test]
fn external_entries_to_float_consumers_keep_the_load() {
    for branch in [false, true] {
        let mut chunk = Chunk::new();
        chunk.register_count = 3;
        chunk.local_register_count = 1;
        chunk.parameter_register_count = 1;
        let original = chunk.add_constant(Literal::Float(9.0)).unwrap();
        let replacement = chunk.add_constant(Literal::Float(5.5)).unwrap();
        for instruction in [
            Instruction::LoadConstant {
                destination: TEMPORARY,
                constant: original,
            },
            Instruction::Jump {
                offset: JumpOffset::new(2),
            },
            Instruction::LoadConstant {
                destination: TEMPORARY,
                constant: replacement,
            },
            if branch {
                Instruction::JumpUnless {
                    comparison: Comparison::LessThan,
                    left: SOURCE,
                    right: TEMPORARY,
                    offset: ShortJumpOffset::new(2),
                }
            } else {
                Instruction::FloatMultiply {
                    destination: RESULT,
                    left: SOURCE,
                    right: TEMPORARY,
                }
            },
            if branch {
                Instruction::ReturnNull
            } else {
                Instruction::ReturnScalarUnchecked { source: RESULT }
            },
            Instruction::ReturnNull,
        ] {
            chunk.emit(instruction, Span::zero());
        }
        assert_unchanged(chunk);
    }
}

#[test]
fn a_float_consumer_can_overwrite_its_literal_register() {
    for value in [2.0, 5.5] {
        let mut chunk = branch_fixture(false, true);
        chunk.constants[0] = Literal::Float(value);
        chunk.code[1] = Instruction::FloatMultiply {
            destination: TEMPORARY,
            left: SOURCE,
            right: TEMPORARY,
        };
        chunk.code[2] = Instruction::ReturnScalarUnchecked { source: TEMPORARY };
        rewrite(&mut chunk);
        assert_eq!(chunk.code.len(), 3);
        assert!(matches!(
            chunk.code[0],
            Instruction::FloatAdd {
                destination: TEMPORARY,
                left: SOURCE,
                right: SOURCE,
            } | Instruction::FloatMultiplyConstant {
                destination: TEMPORARY,
                source: SOURCE,
                ..
            }
        ));
    }
}
