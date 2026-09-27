use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayKind;
use whim_bytecode::instruction::operands::Count;
use whim_bytecode::instruction::operands::DescriptorIndex;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IndexUpdateOperation;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::instruction::operands::ShortJumpOffset;
use whim_bytecode::instruction::word::InstructionWord;
use whim_bytecode::reference_registers;
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
fn indexed_write_paths_verify_their_register_windows() {
    for count in [1, 2, u8::MAX] {
        let instruction = Instruction::IndexSetPath {
            index_count: Count::new(count),
            container: Register::new(0),
            first_index: Register::new(1),
            value: Register::new(0),
        };

        assert_round_trip(instruction);
        let mut chunk = Chunk::new();
        chunk.register_count = 1 + u16::from(count);
        chunk.emit(instruction, Span::zero());
        chunk.emit(Instruction::ReturnNull, Span::zero());
        verify(&chunk).unwrap();
        chunk.register_count -= 1;
        assert!(verify(&chunk).is_err());
    }

    for (count, container) in [(0, 0), (2, 1), (2, 2)] {
        let mut chunk = Chunk::new();
        chunk.register_count = 3;
        chunk.emit(
            Instruction::IndexSetPath {
                index_count: Count::new(count),
                container: Register::new(container),
                first_index: Register::new(1),
                value: Register::new(0),
            },
            Span::zero(),
        );

        chunk.emit(Instruction::ReturnNull, Span::zero());
        assert_eq!(
            verify(&chunk),
            Err(VerifyError::IndexPathInvalid { instruction: 0 })
        );
    }
}

#[test]
fn indexed_read_paths_verify_windows_and_allow_overlapping_destinations() {
    for count in [1, 2, u8::MAX] {
        for destination in [0, 1, u16::from(count)] {
            let instruction = Instruction::IndexGetPath {
                index_count: Count::new(count),
                destination: Register::new(destination),
                container: Register::new(0),
                first_index: Register::new(1),
            };
            assert_round_trip(instruction);
            let mut chunk = Chunk::new();
            chunk.register_count = 1 + u16::from(count);
            chunk.emit(instruction, Span::zero());
            chunk.emit(Instruction::ReturnNull, Span::zero());
            verify(&chunk).unwrap();
            chunk.register_count -= 1;
            assert!(verify(&chunk).is_err());
        }
    }
    let mut chunk = Chunk::new();
    chunk.register_count = 1;
    chunk.emit(
        Instruction::IndexGetPath {
            index_count: Count::new(0),
            destination: Register::new(0),
            container: Register::new(0),
            first_index: Register::new(0),
        },
        Span::zero(),
    );
    chunk.emit(Instruction::ReturnNull, Span::zero());
    assert_eq!(
        verify(&chunk),
        Err(VerifyError::IndexPathInvalid { instruction: 0 })
    );
}

#[test]
fn indexed_update_paths_verify_windows_and_operations() {
    for operation in [
        IndexUpdateOperation::Add,
        IndexUpdateOperation::Subtract,
        IndexUpdateOperation::Multiply,
        IndexUpdateOperation::Divide,
        IndexUpdateOperation::Modulo,
        IndexUpdateOperation::Power,
        IndexUpdateOperation::BitwiseAnd,
        IndexUpdateOperation::BitwiseOr,
        IndexUpdateOperation::BitwiseXor,
        IndexUpdateOperation::ShiftLeft,
        IndexUpdateOperation::ShiftRight,
    ] {
        for count in [1, 2, u8::MAX] {
            let instruction = Instruction::IndexUpdatePath {
                index_count: Count::new(count),
                operation,
                container: Register::new(0),
                operand: Register::new(1),
            };
            assert_round_trip(instruction);
            let mut chunk = Chunk::new();
            chunk.register_count = 2 + u16::from(count);
            chunk.emit(instruction, Span::zero());
            chunk.emit(Instruction::ReturnNull, Span::zero());
            verify(&chunk).unwrap();
            chunk.register_count -= 1;
            assert!(verify(&chunk).is_err());
        }
    }
    for (count, container) in [(0, 0), (2, 1), (2, 2), (2, 3)] {
        let mut chunk = Chunk::new();
        chunk.register_count = 4;
        chunk.emit(
            Instruction::IndexUpdatePath {
                index_count: Count::new(count),
                operation: IndexUpdateOperation::Add,
                container: Register::new(container),
                operand: Register::new(1),
            },
            Span::zero(),
        );
        chunk.emit(Instruction::ReturnNull, Span::zero());
        assert_eq!(
            verify(&chunk),
            Err(VerifyError::IndexPathInvalid { instruction: 0 })
        );
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

#[test]
fn short_instructions_preserve_tags_and_operands() {
    for instruction in [
        Instruction::ReturnNull,
        Instruction::Clear {
            target: Register::new(u16::MAX),
        },
        Instruction::Jump {
            offset: JumpOffset::new(i32::MIN),
        },
    ] {
        assert_round_trip(instruction);
    }
}

fn assert_round_trip(instruction: Instruction) {
    // SAFETY: the word comes from a live instruction and retains its tag.
    let word = unsafe { InstructionWord::read(&instruction) };
    assert_eq!(instruction.kind(), word.kind());
    // SAFETY: the word's tag matches this instruction.
    let decoded = unsafe { word.decode() };
    // SAFETY: the decoded instruction is live.
    assert_eq!(word, unsafe { InstructionWord::read(&decoded) });
    assert_eq!(
        format!("{word:?}"),
        format!("InstructionWord({instruction:?})")
    );
    assert_eq!(instruction, decoded);
    assert_eq!(instruction.kind(), decoded.kind());
    let encoded = bincode::serialize(&instruction).unwrap();
    assert_eq!(instruction, bincode::deserialize(&encoded).unwrap());
}

#[test]
fn owned_move_destinations_keep_incoming_references_in_the_mask() {
    let mut chunk = Chunk::new();
    chunk.register_count = 4;
    chunk.local_register_count = 2;
    chunk.parameter_register_count = 1;
    for instruction in [
        Instruction::MoveOwned {
            destination: Register::new(2),
            source: Register::new(1),
        },
        Instruction::MoveOwned {
            destination: Register::new(3),
            source: Register::new(2),
        },
        Instruction::ReturnNull,
    ] {
        chunk.emit(instruction, Span::zero());
    }
    verify(&chunk).unwrap();
    assert_eq!(reference_registers::mask(&chunk), 0b1100);
    assert_eq!(
        reference_registers::mask_with_classification(&chunk, 2, |_| false),
        0b1100
    );
    chunk.refresh_runtime_metadata();
    assert_eq!(chunk.reference_register_mask, 0b1100);
}

#[test]
fn scalar_result_classification_keeps_its_existing_scope() {
    let mut chunk = Chunk::new();
    chunk.register_count = 2;
    chunk.emit(
        Instruction::CallSelfUnchecked {
            argument_count: Count::new(0),
            destination: Register::new(0),
            first_argument: Register::new(0),
        },
        Span::zero(),
    );
    chunk.emit(Instruction::ReturnNull, Span::zero());
    verify(&chunk).unwrap();
    assert_eq!(reference_registers::mask(&chunk), 1);
    assert_eq!(
        reference_registers::mask_with_classification(&chunk, 0, |_| false),
        0
    );
    chunk.code.insert(
        1,
        Instruction::MoveOwned {
            destination: Register::new(1),
            source: Register::new(0),
        },
    );
    chunk.spans.insert(1, Span::zero());
    verify(&chunk).unwrap();
    assert_eq!(
        reference_registers::mask_with_classification(&chunk, 0, |_| false),
        0
    );
}

#[test]
fn scalar_local_owned_moves_need_no_reference_teardown() {
    let mut chunk = Chunk::new();
    chunk.register_count = 4;
    chunk.local_register_count = 2;
    for instruction in [
        Instruction::LoadInteger {
            destination: Register::new(1),
            immediate: ImmediateInteger::signed(7),
            kind: IntegerKind::I64,
        },
        Instruction::MoveOwned {
            destination: Register::new(2),
            source: Register::new(1),
        },
        Instruction::MoveOwned {
            destination: Register::new(3),
            source: Register::new(2),
        },
        Instruction::ReturnNull,
    ] {
        chunk.emit(instruction, Span::zero());
    }
    verify(&chunk).unwrap();
    assert_eq!(reference_registers::mask(&chunk), 0b1100);
    assert_eq!(
        reference_registers::mask_with_classification(&chunk, 0, |_| true),
        0
    );
}

#[test]
fn owned_moves_from_trace_slots_keep_reference_teardown() {
    let mut chunk = Chunk::new();
    chunk.register_count = 5;
    chunk.local_register_count = 3;
    chunk.parameter_register_count = 1;
    chunk.trace_argument_registers = vec![Register::new(2)];
    for instruction in [
        Instruction::MoveOwned {
            destination: Register::new(3),
            source: Register::new(2),
        },
        Instruction::MoveOwned {
            destination: Register::new(4),
            source: Register::new(3),
        },
        Instruction::ReturnNull,
    ] {
        chunk.emit(instruction, Span::zero());
    }
    verify(&chunk).unwrap();
    assert_eq!(
        reference_registers::mask_with_classification(&chunk, 1, |_| false),
        0b1_1000
    );
}
