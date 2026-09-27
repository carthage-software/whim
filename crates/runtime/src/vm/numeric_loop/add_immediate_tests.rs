use whim_bytecode::chunk::Chunk;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayValueMode;
use whim_bytecode::instruction::operands::ImmediateInt;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_span::Span;
use whim_value::Value;
use whim_value::heap::Heap;
use whim_value::newtype::NewtypeValueId;
use whim_value::string::ByteStringObject;
use whim_value::vec::VecObject;

use super::NumericLoopOutcome;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::vm::Fault;
use crate::vm::VirtualMachine;

const SOURCE: Register = Register::new(0);
const DESTINATION: Register = Register::new(1);
const ELEMENT: Register = Register::new(2);
const INDEX: Register = Register::new(3);

fn addition(destination: Register, immediate: u16) -> Instruction {
    Instruction::AddImmediate {
        destination,
        source: SOURCE,
        immediate: ImmediateInteger::unsigned(immediate),
        kind: Some(IntegerKind::U64),
    }
}

fn execute(code: &[Instruction], registers: &mut [Value]) -> NumericLoopOutcome {
    let mut chunk = Chunk::new();
    chunk.register_count = u16::try_from(registers.len()).unwrap();
    for instruction in code {
        chunk.emit(*instruction, Span::zero());
    }
    chunk.emit(
        Instruction::Jump {
            offset: JumpOffset::new(1),
        },
        Span::zero(),
    );
    let mut engine = Engine::new(EngineConfiguration::default());
    let mut vm = VirtualMachine::new(&mut engine);
    // SAFETY: every operand names a live register, each unchecked read has its
    // required kind, and the final jump exits this bounded straight-line body.
    // The fallback cases stop at a guarded kind or tag check.
    unsafe {
        vm.run_numeric_loop::<false>(&chunk, registers.as_mut_ptr(), 0, chunk.code.len(), 0, 0)
    }
}

#[test]
fn uint_add_immediate_keeps_high_bits_and_in_place_or_distinct_destinations() {
    for immediate in [0, 32768, u16::MAX] {
        for destination in [SOURCE, DESTINATION] {
            let start = u64::MAX - u64::from(immediate);
            let mut registers = [Value::uint(start), Value::int(-1)];
            assert!(matches!(
                execute(&[addition(destination, immediate)], &mut registers),
                NumericLoopOutcome::Completed
            ));
            assert_eq!(
                registers[usize::from(destination.index())].as_uint(),
                Some(u64::MAX)
            );
            if destination != SOURCE {
                assert_eq!(registers[0].as_uint(), Some(start));
            }
        }
    }
}

#[test]
fn uint_add_immediate_faults_keep_destinations_and_flush_prior_writes() {
    for destination in [SOURCE, DESTINATION] {
        let heap = Heap::new();
        let child = ByteStringObject::from_bytes(&heap, b"an unchanged destination on overflow");
        let mut registers = [
            Value::uint(u64::MAX),
            Value::string(child.clone()),
            Value::int(0),
        ];
        let outcome = execute(
            &[
                Instruction::LoadInteger {
                    destination: ELEMENT,
                    immediate: ImmediateInteger::signed(7),
                    kind: IntegerKind::I64,
                },
                addition(destination, u16::MAX),
            ],
            &mut registers,
        );
        assert!(matches!(
            outcome,
            NumericLoopOutcome::Fault {
                resume_ip: 2,
                fault: Fault::Overflow,
                operator: "+",
                left: SOURCE,
                right: None,
            }
        ));
        assert_eq!(registers[0].as_uint(), Some(u64::MAX));
        assert_eq!(
            registers[1].as_string_bytes(),
            Some(b"an unchanged destination on overflow".as_slice())
        );
        assert_eq!(registers[2].as_int(), Some(7));
        assert!(!child.is_unique());
    }
}

#[test]
fn uint_add_immediate_releases_and_invalidates_a_pinned_destination() {
    let heap = Heap::new();
    let child = ByteStringObject::from_bytes(&heap, b"a child owned by the overwritten vector");
    let vector = VecObject::with_elements(&heap, [Value::int(7), Value::string(child.clone())]);
    let mut registers = [
        Value::uint(2),
        Value::vec(vector),
        Value::int(0),
        Value::int(0),
    ];
    let outcome = execute(
        &[
            Instruction::VecIndexGet {
                destination: ELEMENT,
                container: DESTINATION,
                index: INDEX,
                value_mode: ArrayValueMode::Int,
            },
            addition(DESTINATION, u16::MAX),
            Instruction::IndexGet {
                destination: ELEMENT,
                container: DESTINATION,
                index: INDEX,
            },
        ],
        &mut registers,
    );
    assert!(matches!(outcome, NumericLoopOutcome::Deoptimize(2)));
    assert_eq!(registers[0].as_uint(), Some(2));
    assert_eq!(registers[1].as_uint(), Some(65537));
    assert_eq!(registers[2].as_int(), Some(7));
    assert!(child.is_unique());
}

#[test]
fn uint_add_immediate_keeps_tag_and_kind_fallbacks_and_other_unsigned_steps() {
    let mut registers = [Value::int(5), Value::int(9)];
    assert!(matches!(
        execute(&[addition(DESTINATION, 1)], &mut registers),
        NumericLoopOutcome::Deoptimize(0)
    ));
    assert_eq!(registers[0].as_int(), Some(5));
    assert_eq!(registers[1].as_int(), Some(9));

    let tag = NewtypeValueId(17);
    let mut registers = [Value::uint(5).with_newtype(Some(tag)), Value::int(9)];
    assert!(matches!(
        execute(&[addition(DESTINATION, 1)], &mut registers),
        NumericLoopOutcome::Deoptimize(0)
    ));
    assert_eq!(registers[0].newtype_id(), Some(tag));
    assert_eq!(registers[1].as_int(), Some(9));

    for instruction in [
        Instruction::SubtractImmediate {
            destination: SOURCE,
            source: SOURCE,
            immediate: ImmediateInteger::unsigned(1),
            kind: Some(IntegerKind::U64),
        },
        Instruction::Step {
            destination: SOURCE,
            source: SOURCE,
            immediate: ImmediateInt::new(1),
            kind: Some(IntegerKind::U64),
        },
    ] {
        let mut registers = [Value::uint(5)];
        assert!(matches!(
            execute(&[instruction], &mut registers),
            NumericLoopOutcome::Deoptimize(0)
        ));
        assert_eq!(registers[0].as_uint(), Some(5));
    }
}
