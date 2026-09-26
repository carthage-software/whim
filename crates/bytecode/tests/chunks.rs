use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayKind;
use whim_bytecode::instruction::operands::Count;
use whim_bytecode::instruction::operands::DescriptorIndex;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::instruction::operands::ShortJumpOffset;
use whim_bytecode::instruction::word::InstructionWord;
use whim_bytecode::rewrite::control_flow_targets;
use whim_bytecode::verify::VerifyError;
use whim_bytecode::verify::verify;
use whim_span::Span;

#[test]
#[should_panic(expected = "only a jump instruction can be patched")]
fn patching_a_non_jump_panics() {
    let mut chunk = Chunk::new();
    chunk.emit(Instruction::ReturnNull, Span::zero());
    chunk.patch_jump(0, 0);
}

#[test]
#[should_panic(expected = "a jump offset must fit in i32")]
fn patching_an_overflowing_offset_panics() {
    let mut chunk = Chunk::new();
    chunk.emit(
        Instruction::Jump {
            offset: JumpOffset::new(0),
        },
        Span::zero(),
    );
    chunk.patch_jump(0, u32::MAX);
}

#[test]
#[should_panic(expected = "a bytecode branch target must be non-negative")]
fn reading_a_negative_branch_target_panics() {
    let mut chunk = Chunk::new();
    chunk.emit(
        Instruction::Jump {
            offset: JumpOffset::new(-1),
        },
        Span::zero(),
    );
    let _ = control_flow_targets(&chunk);
}

#[test]
fn integer_instructions_preserve_kind_and_operand_bits() {
    for kind in [IntegerKind::I64, IntegerKind::U64] {
        for instruction in [
            Instruction::Add {
                destination: Register::new(0x1234),
                left: Register::new(0x5678),
                right: Register::new(0xabcd),
                kind: Some(kind),
            },
            Instruction::Add {
                destination: Register::new(0x1234),
                left: Register::new(0x5678),
                right: Register::new(0xabcd),
                kind: None,
            },
            Instruction::LoadInteger {
                destination: Register::new(0x1234),
                immediate: ImmediateInteger::unsigned(u16::MAX),
                kind,
            },
            Instruction::IntegerRangeJumpUnless {
                subject: Register::new(0x1234),
                descriptor: DescriptorIndex::new(0x5678),
                offset: ShortJumpOffset::new(i16::MIN),
                kind,
            },
        ] {
            assert_round_trip(instruction);
        }
    }
    assert_eq!(ImmediateInteger::signed(-1).as_uint(), u16::MAX);
    assert_eq!(ImmediateInteger::unsigned(u16::MAX).as_int(), -1);
}

#[test]
fn integer_range_branches_require_a_descriptor_of_their_kind() {
    let mut chunk = Chunk::new();
    chunk.register_count = 1;
    chunk.type_descriptors = vec![
        TypeDescriptor::IntRange {
            min: Some(-1),
            max: None,
        },
        TypeDescriptor::UintRange {
            min: Some(1),
            max: None,
        },
    ];
    for (expected, kind) in [IntegerKind::I64, IntegerKind::U64].into_iter().enumerate() {
        for descriptor in 0..2 {
            for jump_if in [false, true] {
                chunk.code.clear();
                chunk.spans.clear();
                let subject = Register::new(0);
                let offset = ShortJumpOffset::new(1);
                let descriptor = DescriptorIndex::new(descriptor);
                chunk.emit(
                    if jump_if {
                        Instruction::IntegerRangeJumpIf {
                            subject,
                            descriptor,
                            offset,
                            kind,
                        }
                    } else {
                        Instruction::IntegerRangeJumpUnless {
                            subject,
                            descriptor,
                            offset,
                            kind,
                        }
                    },
                    Span::zero(),
                );
                chunk.emit(Instruction::ReturnNull, Span::zero());
                if usize::from(descriptor.index()) == expected {
                    verify(&chunk).unwrap();
                } else {
                    assert!(matches!(
                        verify(&chunk),
                        Err(VerifyError::TypeDescriptorKindInvalid { .. })
                    ));
                }
            }
        }
    }
}

#[test]
fn array_operands_preserve_kind_and_register_windows() {
    for (kind, width) in [
        (ArrayKind::Vec, 1),
        (ArrayKind::Dict, 2),
        (ArrayKind::Tuple, 1),
    ] {
        for count in [0, 1, u8::MAX] {
            let mut chunk = Chunk::new();
            chunk.register_count = 1 + u16::from(count) * width;
            let instruction = Instruction::NewArray {
                count: Count::new(count),
                destination: Register::new(0),
                first_element: Register::new(1),
                kind,
            };
            assert_round_trip(instruction);
            chunk.emit(instruction, Span::zero());
            chunk.emit(Instruction::ReturnNull, Span::zero());
            verify(&chunk).unwrap();
            if count != 0 {
                chunk.register_count -= 1;
                assert!(matches!(
                    verify(&chunk),
                    Err(VerifyError::RegisterWindowOutOfRange { .. })
                ));
            }
        }
    }
}

#[test]
fn write_operands_preserve_flags_and_register_windows() {
    for (new_line, stderr) in [(false, false), (true, false), (false, true), (true, true)] {
        for count in [0, 1, u8::MAX] {
            let mut chunk = Chunk::new();
            chunk.register_count = 1 + u16::from(count);
            let instruction = Instruction::Write {
                count: Count::new(count),
                register: Register::new(1),
                new_line,
                stderr,
            };
            assert_round_trip(instruction);
            chunk.emit(instruction, Span::zero());
            chunk.emit(Instruction::ReturnNull, Span::zero());
            verify(&chunk).unwrap();
            if count != 0 {
                chunk.register_count -= 1;
                assert!(matches!(
                    verify(&chunk),
                    Err(VerifyError::RegisterWindowOutOfRange { .. })
                ));
            }
        }
    }
}

fn assert_round_trip(instruction: Instruction) {
    // SAFETY: the word comes from a live instruction and retains its tag.
    let decoded = unsafe { InstructionWord::read(&instruction).decode() };
    assert_eq!(instruction, decoded);
    assert_eq!(instruction.kind(), decoded.kind());
    let encoded = bincode::serialize(&instruction).unwrap();
    assert_eq!(instruction, bincode::deserialize(&encoded).unwrap());
}
