use std::rc::Rc;

use whim_span::Span;

use super::canonicalize_string_chains;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::verify::verify;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::optimizer::passes::prune_unreachable;
use crate::value::heap::Heap;

const SUBJECT: Register = Register::new(0);
const TEMPORARY: Register = Register::new(1);
const RESULT: Register = Register::new(2);

fn string_chain(heap: &Heap, input: &[u8], literals: &[&[u8]]) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.register_count = 3;
    chunk.local_register_count = 1;
    let input = chunk
        .add_constant(Literal::String(heap.intern(input)))
        .unwrap();
    chunk.emit(
        Instruction::LoadConstant {
            destination: SUBJECT,
            constant: input,
        },
        Span::zero(),
    );
    for (position, literal) in literals.iter().enumerate() {
        let constant = chunk
            .add_constant(Literal::String(heap.intern(literal)))
            .unwrap();
        chunk.emit(
            Instruction::LoadConstant {
                destination: TEMPORARY,
                constant,
            },
            Span::zero(),
        );
        chunk.emit(
            Instruction::StringJumpUnless {
                comparison: Comparison::Equal,
                left: SUBJECT,
                right: TEMPORARY,
                offset: ShortJumpOffset::new(3),
            },
            Span::zero(),
        );
        emit_exit(&mut chunk, i16::try_from(position + 1).unwrap());
    }
    emit_exit(&mut chunk, 90);
    verify(&chunk).expect("the equality chain verifies");
    chunk
}

fn emit_exit(chunk: &mut Chunk, code: i16) {
    chunk.emit(
        Instruction::LoadInt {
            destination: RESULT,
            immediate: ImmediateInt::new(code),
        },
        Span::zero(),
    );
    chunk.emit(Instruction::Exit { code: RESULT }, Span::zero());
}

fn entry_string_chain(heap: &Heap) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.register_count = 2;
    chunk.local_register_count = 1;
    chunk.parameter_register_count = 1;
    for (literal, value) in [(b"alpha".as_slice(), 1), (b"beta", 2)] {
        let constant = chunk
            .add_constant(Literal::String(heap.intern(literal)))
            .unwrap();
        for instruction in [
            Instruction::LoadConstant {
                destination: TEMPORARY,
                constant,
            },
            Instruction::StringJumpUnless {
                comparison: Comparison::Equal,
                left: SUBJECT,
                right: TEMPORARY,
                offset: ShortJumpOffset::new(2),
            },
            Instruction::ReturnIntUnchecked {
                immediate: ImmediateInt::new(value),
            },
        ] {
            chunk.emit(instruction, Span::zero());
        }
    }
    chunk.emit(
        Instruction::ReturnIntUnchecked {
            immediate: ImmediateInt::new(90),
        },
        Span::zero(),
    );
    verify(&chunk).expect("the function-entry equality chain verifies");
    chunk
}

fn assert_exit(engine: &mut Engine, mut chunk: Chunk, expected: u8) {
    chunk.refresh_runtime_metadata();
    let unit = Rc::new(CompiledUnit {
        path: engine.heap.intern(b"/optimizer/string-chain.whim"),
        main: chunk,
        functions: Vec::new(),
        classes: Vec::new(),
        constants: Vec::new(),
        type_aliases: Vec::new(),
        newtypes: Vec::new(),
    });
    let outcome = engine.run_unit(&unit);
    assert_eq!(outcome.exit_code(), expected, "{outcome:?}");
}

#[test]
fn string_chain_refuses_external_entry_into_comparison() {
    let heap = Heap::new();
    let mut chunk = string_chain(&heap, b"alpha", &[b"alpha", b"beta"]);
    chunk.code.splice(
        1..1,
        [
            Instruction::Move {
                destination: TEMPORARY,
                source: SUBJECT,
            },
            Instruction::Jump {
                offset: JumpOffset::new(2),
            },
        ],
    );
    chunk.spans.splice(1..1, [Span::zero(); 2]);
    verify(&chunk).expect("the externally entered comparison verifies");
    let original = chunk.code.clone();

    assert!(!canonicalize_string_chains(&mut chunk).changed);
    assert_eq!(chunk.code, original);
    assert!(chunk.switch_tables.is_empty());
}

#[test]
fn string_chain_refuses_live_temporary_in_any_body_or_default() {
    let heap = Heap::new();
    for live_read in [3, 7, 9] {
        let mut chunk = string_chain(&heap, b"alpha", &[b"alpha", b"beta"]);
        chunk.code[live_read] = Instruction::ReturnUnchecked { source: TEMPORARY };
        verify(&chunk).expect("the temporary-reading branch verifies");
        let original = chunk.code.clone();

        assert!(
            !canonicalize_string_chains(&mut chunk).changed,
            "read at {live_read}"
        );
        assert_eq!(chunk.code, original);
        assert!(chunk.switch_tables.is_empty());
    }
}

#[test]
fn string_chain_keeps_first_duplicate_target() {
    let mut engine = Engine::new(EngineConfiguration::default());
    for (input, expected) in [(b"alpha".as_slice(), 1), (b"beta", 3), (b"missing", 90)] {
        let mut chunk = string_chain(&engine.heap, input, &[b"alpha", b"alpha", b"beta"]);
        assert_exit(&mut engine, chunk.clone(), expected);

        assert!(canonicalize_string_chains(&mut chunk).changed);
        let SwitchTable::String { arms, default, .. } = &chunk.switch_tables[0] else {
            panic!("the equality chain becomes a string switch");
        };
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].0.as_bytes(), b"alpha");
        assert_eq!(arms[0].1, 1);
        assert_eq!(arms[1].0.as_bytes(), b"beta");
        assert_eq!(arms[1].1, 9);
        assert_eq!(*default, 11);
        prune_unreachable::optimize_chunk(&mut chunk);
        assert_exit(&mut engine, chunk, expected);
    }
}

#[test]
fn string_chain_keeps_tests_reached_after_selected_body_changes_subject() {
    let mut engine = Engine::new(EngineConfiguration::default());
    for (input, expected) in [(b"alpha".as_slice(), 2), (b"beta", 2), (b"missing", 90)] {
        let mut chunk = string_chain(&engine.heap, input, &[b"alpha", b"beta"]);
        let changed = chunk
            .add_constant(Literal::String(engine.heap.intern(b"beta")))
            .unwrap();
        chunk.code[3] = Instruction::LoadConstant {
            destination: SUBJECT,
            constant: changed,
        };
        chunk.code[4] = Instruction::LoadNull {
            destination: RESULT,
        };
        assert_exit(&mut engine, chunk.clone(), expected);

        assert!(canonicalize_string_chains(&mut chunk).changed);
        prune_unreachable::optimize_chunk(&mut chunk);
        assert!(matches!(
            chunk.code[5],
            Instruction::LoadConstant {
                destination: TEMPORARY,
                ..
            }
        ));
        assert!(matches!(
            chunk.code[6],
            Instruction::StringJumpUnless { .. }
        ));
        assert_exit(&mut engine, chunk, expected);
    }
}

#[test]
fn string_chain_rebases_all_switch_targets_after_pruning() {
    let heap = Heap::new();
    let mut chunk = string_chain(&heap, b"alpha", &[b"alpha", b"beta"]);
    let first_load = chunk.code[1];
    assert!(canonicalize_string_chains(&mut chunk).changed);
    prune_unreachable::optimize_chunk(&mut chunk);

    assert_eq!(chunk.code[1], first_load);
    assert_eq!(chunk.code.len(), 9);
    let Instruction::SwitchString { table, .. } = chunk.code[2] else {
        panic!("the canonical switch remains after pruning");
    };
    let SwitchTable::String { arms, default, .. } =
        &chunk.switch_tables[usize::from(table.index())]
    else {
        panic!("the switch retains its string table");
    };
    assert_eq!(
        arms.iter().map(|(_, target)| *target).collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(*default, 5);
    for (target, code) in [(arms[0].1, 1), (arms[1].1, 2), (*default, 90)] {
        assert!(matches!(
            chunk.code[usize::try_from(2 + target).unwrap()],
            Instruction::LoadInt { immediate, .. } if immediate.value() == code
        ));
    }
    verify(&chunk).expect("all compacted switch targets verify");
}

#[test]
fn fresh_entry_string_chain_removes_first_load_and_rebases_return_targets() {
    let heap = Heap::new();
    let mut chunk = entry_string_chain(&heap);
    let changes = canonicalize_string_chains(&mut chunk);
    assert!(changes.changed);
    assert!(!changes.retained_initial_load);
    assert!(matches!(chunk.code[0], Instruction::SwitchString { .. }));
    prune_unreachable::optimize_chunk(&mut chunk);
    assert_eq!(chunk.code.len(), 4);
    let Instruction::SwitchString { table, .. } = chunk.code[0] else {
        panic!("the switch replaces the fresh temporary load");
    };
    let SwitchTable::String { arms, default, .. } =
        &chunk.switch_tables[usize::from(table.index())]
    else {
        panic!("the entry switch retains its string table");
    };
    assert_eq!(arms[0].1, 1);
    assert_eq!(arms[1].1, 2);
    assert_eq!(*default, 3);
    for (target, expected) in [(arms[0].1, 1), (arms[1].1, 2), (*default, 90)] {
        assert!(matches!(
            chunk.code[usize::try_from(target).unwrap()],
            Instruction::ReturnIntUnchecked { immediate } if immediate.value() == expected
        ));
    }
    verify(&chunk).expect("the compacted entry-switch targets verify");
}

#[test]
fn string_chain_with_back_edge_to_entry_retains_initial_drop() {
    let heap = Heap::new();
    let mut chunk = entry_string_chain(&heap);
    chunk.code[6] = Instruction::Jump {
        offset: JumpOffset::new(-6),
    };
    let first_load = chunk.code[0];
    let changes = canonicalize_string_chains(&mut chunk);
    assert!(changes.changed);
    assert!(changes.retained_initial_load);
    assert_eq!(chunk.code[0], first_load);
    assert!(matches!(chunk.code[1], Instruction::SwitchString { .. }));
    prune_unreachable::optimize_chunk(&mut chunk);
    verify(&chunk).expect("the loop retains its entry load and valid switch offsets");
}

#[test]
fn direct_return_shortcut_preserves_all_temporary_operand_reads() {
    let heap = Heap::new();
    for instruction in [
        Instruction::ReturnUnchecked { source: TEMPORARY },
        Instruction::ReturnReferenceUnchecked { source: TEMPORARY },
        Instruction::ReturnScalarUnchecked { source: TEMPORARY },
        Instruction::ReturnPairUnchecked {
            first: TEMPORARY,
            second: SUBJECT,
        },
        Instruction::ReturnPairUnchecked {
            first: SUBJECT,
            second: TEMPORARY,
        },
    ] {
        for body in [2, 5, 6] {
            let mut chunk = entry_string_chain(&heap);
            chunk.code[body] = instruction;
            let original = chunk.code.clone();
            assert!(!canonicalize_string_chains(&mut chunk).changed);
            assert_eq!(chunk.code, original);
        }
    }
}

#[test]
fn entry_chain_keeps_local_and_trace_snapshot_values() {
    let heap = Heap::new();
    let mut chunk = entry_string_chain(&heap);
    chunk.local_register_count = 2;
    chunk.trace_argument_registers.push(TEMPORARY);
    verify(&chunk).expect("a retained argument snapshot is a named local");
    let original = chunk.code.clone();
    assert!(!canonicalize_string_chains(&mut chunk).changed);
    assert_eq!(chunk.code, original);
}
