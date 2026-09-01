//! The verifier soundness gate, fuzzed.

use std::rc::Rc;

use proptest::collection::vec as vec_strategy;
use proptest::option::of;
use proptest::prelude::*;
use whim_span::Span;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::CatchEntry;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::verify::VerifyError;
use crate::bytecode::verify::verify;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::unwrap_result_invariant;

/// A register index drawn from `0..bound`, so some indices exceed the frame's
/// `register_count` and must be rejected by the verifier.
fn register(bound: u16) -> impl Strategy<Value = Register> {
    (0..bound).prop_map(Register::new)
}

fn literal() -> impl Strategy<Value = Literal> {
    prop_oneof![
        Just(Literal::Null),
        any::<bool>().prop_map(Literal::Bool),
        any::<i64>().prop_map(Literal::Int),
    ]
}

fn instruction(register_bound: u16, constant_bound: u16) -> impl Strategy<Value = Instruction> {
    let reg = move || register(register_bound);
    let constant = (0..constant_bound).prop_map(ConstantIndex::new);
    let forward = (1i32..=10).prop_map(JumpOffset::new);
    prop_oneof![
        (reg(), reg()).prop_map(|(destination, source)| Instruction::Move {
            destination,
            source
        }),
        (reg(), constant).prop_map(|(destination, constant)| Instruction::LoadConstant {
            destination,
            constant
        }),
        reg().prop_map(|destination| Instruction::LoadNull { destination }),
        reg().prop_map(|destination| Instruction::LoadTrue { destination }),
        reg().prop_map(|destination| Instruction::LoadFalse { destination }),
        (reg(), reg(), reg()).prop_map(|(destination, left, right)| Instruction::Add {
            destination,
            left,
            right
        }),
        (reg(), reg(), reg()).prop_map(|(destination, left, right)| Instruction::Subtract {
            destination,
            left,
            right
        }),
        (reg(), reg()).prop_map(|(destination, source)| Instruction::Negate {
            destination,
            source
        }),
        (reg(), reg()).prop_map(|(destination, source)| Instruction::Not {
            destination,
            source
        }),
        (reg(), forward.clone())
            .prop_map(|(condition, offset)| Instruction::JumpIfFalse { condition, offset }),
        (reg(), forward.clone())
            .prop_map(|(condition, offset)| Instruction::JumpIfTrue { condition, offset }),
        forward.prop_map(|offset| Instruction::Jump { offset }),
        reg().prop_map(|source| Instruction::Throw { source }),
        reg().prop_map(|source| Instruction::Return { source }),
        Just(Instruction::ReturnNull),
    ]
}

fn catch_entry(
    register_bound: u16,
    descriptor_bound: u16,
    code_bound: u32,
) -> impl Strategy<Value = CatchEntry> {
    (
        0..code_bound,
        0..code_bound,
        0..code_bound,
        0..descriptor_bound,
        0..register_bound,
        of(register(register_bound)),
    )
        .prop_map(
            move |(start, end, handler, descriptor, temporary_floor, binding)| CatchEntry {
                start,
                end,
                handler,
                type_descriptor: DescriptorIndex::new(descriptor),
                temporary_floor,
                binding,
            },
        )
}

fn chunk() -> impl Strategy<Value = Chunk> {
    (
        1u16..=6,
        vec_strategy(literal(), 0..4),
        vec_strategy(
            prop_oneof![Just(TypeDescriptor::Int), Just(TypeDescriptor::Null)],
            0..3,
        ),
    )
        .prop_flat_map(|(register_count, constants, type_descriptors)| {
            let register_bound = register_count + 3;
            // SAFETY: the surrounding invariant proves this result is successful.
            let constant_bound = unsafe {
                unwrap_result_invariant(
                    u16::try_from(constants.len()),
                    "the fuzz strategy emits at most three constants",
                )
            } + 2;
            // SAFETY: the surrounding invariant proves this result is successful.
            let descriptor_bound = unsafe {
                unwrap_result_invariant(
                    u16::try_from(type_descriptors.len()),
                    "the fuzz strategy emits at most two descriptors",
                )
            } + 2;
            let body = vec_strategy(instruction(register_bound, constant_bound), 0..14);
            (
                Just(register_count),
                Just(constants),
                Just(type_descriptors),
                body,
                Just(register_bound),
                Just(descriptor_bound),
            )
        })
        .prop_flat_map(
            |(
                register_count,
                constants,
                type_descriptors,
                mut code,
                register_bound,
                descriptor_bound,
            )| {
                code.push(Instruction::ReturnNull);
                // SAFETY: the surrounding invariant proves this result is successful.
                let code_bound = unsafe {
                    unwrap_result_invariant(
                        u32::try_from(code.len()),
                        "the fuzz strategy emits at most fifteen instructions",
                    )
                } + 2;
                let catches = vec_strategy(
                    catch_entry(register_bound, descriptor_bound, code_bound),
                    0..3,
                );
                (
                    Just(register_count),
                    Just(constants),
                    Just(type_descriptors),
                    Just(code),
                    catches,
                )
            },
        )
        .prop_map(
            |(register_count, constants, type_descriptors, code, catch_table)| {
                let spans = vec![Span::zero(); code.len()];
                let mut chunk = Chunk::new();
                chunk.code = code;
                chunk.spans = spans;
                chunk.constants = constants;
                chunk.type_descriptors = type_descriptors;
                chunk.catch_table = catch_table;
                chunk.register_count = register_count;
                chunk
            },
        )
}

fn execute(chunk: Chunk) {
    let mut engine = Engine::new(EngineConfiguration::default());
    let unit = Rc::new(CompiledUnit {
        path: engine.heap.intern(b"/fuzz/main.whim"),
        main: chunk,
        functions: Vec::new(),
        classes: Vec::new(),
        constants: Vec::new(),
        type_aliases: Vec::new(),
        newtypes: Vec::new(),
    });
    let _ = engine.run_unit(&unit);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn a_verified_chunk_never_causes_undefined_behavior(chunk in chunk()) {
        if verify(&chunk).is_ok() {
            execute(chunk);
        }
    }
}

#[test]
fn panic_requires_a_string_constant() {
    let mut chunk = Chunk::new();
    chunk.constants.push(Literal::Int(1));
    chunk.code.push(Instruction::Panic {
        message: ConstantIndex::new(0),
    });
    chunk.spans.push(Span::zero());

    assert_eq!(
        verify(&chunk),
        Err(VerifyError::ConstantKindInvalid {
            instruction: 0,
            constant: 0,
        })
    );
}

#[test]
fn catch_temporary_floor_must_fit_the_frame() {
    let mut chunk = Chunk::new();
    chunk.type_descriptors.push(TypeDescriptor::Int);
    chunk.code.push(Instruction::ReturnNull);
    chunk.spans.push(Span::zero());
    chunk.catch_table.push(CatchEntry {
        start: 0,
        end: 1,
        handler: 0,
        type_descriptor: DescriptorIndex::new(0),
        temporary_floor: 2,
        binding: None,
    });

    chunk.register_count = 1;

    assert_eq!(
        verify(&chunk),
        Err(VerifyError::CatchTemporaryFloorOutOfRange {
            entry: 0,
            register: 2,
        })
    );
}
