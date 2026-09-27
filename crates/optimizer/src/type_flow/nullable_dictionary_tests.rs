use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayKind;
use whim_bytecode::instruction::operands::Count;
use whim_bytecode::instruction::operands::ImmediateInt;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_span::Span;
use whim_value::heap::Heap;

use super::ALL;
use super::DICTIONARY;
use super::FLOAT;
use super::Fact;
use super::INT;
use super::NULL;
use super::TypeFlow;
use super::transfer::transfer;

const DICT: Register = Register::new(0);
const KEY: Register = Register::new(1);
const READ: Register = Register::new(2);
const ONE: Register = Register::new(3);
const RESULT: Register = Register::new(4);
const CONDITION: Register = Register::new(5);

fn chunk(instructions: impl IntoIterator<Item = Instruction>) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.register_count = 6;
    for instruction in instructions {
        chunk.emit(instruction, Span::zero());
    }
    chunk
}

fn integer(destination: Register, value: i16) -> Instruction {
    Instruction::LoadInteger {
        kind: IntegerKind::I64,
        destination,
        immediate: ImmediateInteger::signed(value),
    }
}

fn empty() -> Instruction {
    Instruction::NewArray {
        kind: ArrayKind::Dict,
        destination: DICT,
        first_element: KEY,
        count: Count::new(0),
    }
}

fn lookup() -> Instruction {
    Instruction::IndexGetOrNull {
        destination: READ,
        container: DICT,
        index: KEY,
    }
}

fn feedback() -> Chunk {
    chunk([
        empty(),
        integer(KEY, 7),
        lookup(),
        Instruction::JumpIfNotNull {
            subject: READ,
            offset: JumpOffset::new(2),
        },
        integer(READ, 0),
        integer(ONE, 1),
        Instruction::Add {
            kind: None,
            destination: RESULT,
            left: READ,
            right: ONE,
        },
        Instruction::IndexSet {
            container: DICT,
            index: KEY,
            value: RESULT,
        },
        Instruction::JumpIfTrue {
            condition: CONDITION,
            offset: JumpOffset::new(-6),
        },
        Instruction::ReturnUnchecked { source: DICT },
    ])
}

#[test]
fn known_null_nonnull_edges_refine_but_initial_zero_keeps_its_state() {
    let heap = Heap::new();
    let chunk = chunk([
        Instruction::JumpIfNotNull {
            subject: READ,
            offset: JumpOffset::new(2),
        },
        Instruction::ReturnNull,
        Instruction::ReturnNull,
    ]);
    let flow = TypeFlow::analyze(&chunk, &[], false, None, &[], &heap);
    for (mask, expected) in [
        (NULL, Some(0)),
        (0, None),
        (NULL | INT, Some(INT)),
        (INT, Some(INT)),
    ] {
        let mut state = [Fact::UNKNOWN; 6];
        state[READ.index() as usize] = Fact::known(mask);
        assert_eq!(
            flow.is_true_edge(0, 2, &state).map(|(_, fact)| fact.mask),
            expected
        );
    }
    let mut state = [Fact::UNKNOWN; 6];
    state[READ.index() as usize] = Fact::known(0);
    assert!(flow.is_true_edge(0, 1, &state).is_none());
}

#[test]
fn null_copies_join_with_the_fallback_without_pruning_either_edge() {
    let heap = Heap::new();
    let chunk = chunk([
        Instruction::LoadNull { destination: READ },
        Instruction::Move {
            destination: RESULT,
            source: READ,
        },
        Instruction::JumpIfNotNull {
            subject: READ,
            offset: JumpOffset::new(2),
        },
        integer(RESULT, 9),
        Instruction::ReturnUnchecked { source: RESULT },
    ]);
    let flow = TypeFlow::analyze(&chunk, &[], false, None, &[], &heap);
    assert!(flow.reachable.iter().all(|reachable| *reachable));
    assert_eq!(flow.fact(4, RESULT).mask, INT);
}

#[test]
fn nullable_only_feedback_reaches_a_fixed_point() {
    let heap = Heap::new();
    for lookup in [
        lookup(),
        Instruction::DictIndexGetIntegerKeyOrNull {
            destination: READ,
            container: DICT,
            index: KEY,
            kind: IntegerKind::I64,
        },
        Instruction::DictIndexGetIntegerKeyOrNull {
            destination: READ,
            container: DICT,
            index: KEY,
            kind: IntegerKind::U64,
        },
        Instruction::DictIndexGetStringKeyOrNull {
            destination: READ,
            container: DICT,
            index: KEY,
        },
    ] {
        let mut chunk = feedback();
        chunk.code[2] = lookup;
        match lookup {
            Instruction::DictIndexGetIntegerKeyOrNull {
                kind: IntegerKind::U64,
                ..
            } => {
                chunk.code[1] = Instruction::LoadInteger {
                    destination: KEY,
                    kind: IntegerKind::U64,
                    immediate: ImmediateInteger::unsigned(7),
                };
            }
            Instruction::DictIndexGetStringKeyOrNull { .. } => {
                chunk.constants.push(Literal::String(heap.intern(b"key")));
                chunk.code[1] = Instruction::LoadConstant {
                    destination: KEY,
                    constant: whim_bytecode::instruction::operands::ConstantIndex::new(0),
                };
            }
            _ => {}
        }
        let flow = TypeFlow::analyze(&chunk, &[], false, None, &[], &heap);
        assert_eq!(flow.array_elements[1], INT);
        assert_eq!(flow.fact(3, READ).mask, INT | NULL);
        assert_eq!(flow.fact(6, READ).mask, INT);
        assert!(flow.reachable.iter().all(|reachable| *reachable));
    }
}

#[test]
fn provisional_null_does_not_hide_a_later_float_store() {
    let heap = Heap::new();
    let mut chunk = feedback();
    chunk.code[8] = Instruction::LoadConstant {
        destination: RESULT,
        constant: whim_bytecode::instruction::operands::ConstantIndex::new(0),
    };
    chunk.constants.push(Literal::Float(0.5));
    chunk.code[9] = Instruction::IndexSet {
        container: DICT,
        index: KEY,
        value: RESULT,
    };
    chunk.emit(
        Instruction::JumpIfTrue {
            condition: CONDITION,
            offset: JumpOffset::new(-8),
        },
        Span::zero(),
    );
    chunk.emit(Instruction::ReturnUnchecked { source: DICT }, Span::zero());
    let flow = TypeFlow::analyze(&chunk, &[], false, None, &[], &heap);
    assert_ne!(flow.array_elements[1] & FLOAT, 0);
    assert_ne!(flow.fact(6, READ).mask & FLOAT, 0);
    assert_ne!(flow.fact(6, READ).mask, INT);
}

#[test]
fn an_initially_impossible_nonnull_edge_still_contributes_writes() {
    let heap = Heap::new();
    let mut chunk = chunk([
        empty(),
        integer(KEY, 7),
        lookup(),
        Instruction::JumpIfNull {
            subject: READ,
            offset: JumpOffset::new(3),
        },
        Instruction::LoadConstant {
            destination: RESULT,
            constant: whim_bytecode::instruction::operands::ConstantIndex::new(0),
        },
        Instruction::IndexSet {
            container: DICT,
            index: KEY,
            value: RESULT,
        },
        Instruction::JumpIfTrue {
            condition: CONDITION,
            offset: JumpOffset::new(-4),
        },
        Instruction::ReturnUnchecked { source: DICT },
    ]);
    chunk.constants.push(Literal::Float(0.5));
    let flow = TypeFlow::analyze(&chunk, &[], false, None, &[], &heap);
    assert_eq!(flow.array_elements[1], FLOAT);
    assert_eq!(flow.fact(3, READ).mask, FLOAT | NULL);
    assert!(flow.reachable.iter().all(|reachable| *reachable));
}

#[test]
fn state_budget_declines_without_publishing_an_empty_summary_proof() {
    let heap = Heap::new();
    let mut chunk = feedback();
    chunk.register_count = u16::MAX;
    for _ in 0..40 {
        chunk.emit(
            Instruction::JumpIfTrue {
                condition: CONDITION,
                offset: JumpOffset::new(1),
            },
            Span::zero(),
        );
    }
    let flow = TypeFlow::analyze(&chunk, &[], false, None, &[], &heap);
    assert!(flow.declined);
    assert_eq!(flow.fact(3, READ).mask, ALL);
}

#[test]
fn only_fresh_empty_dictionary_families_use_the_new_nullable_rule() {
    let mut nonempty = empty();
    let Instruction::NewArray { count, .. } = &mut nonempty else {
        unreachable!()
    };
    *count = Count::new(1);
    for (producer, identity, mask, tracking, expected) in [
        (empty(), 1, DICTIONARY, true, INT | NULL),
        (empty(), 1, DICTIONARY, false, ALL),
        (empty(), 1, ALL, true, ALL),
        (empty(), 0, DICTIONARY, true, ALL),
        (empty(), 3, DICTIONARY, true, ALL),
        (nonempty, 1, DICTIONARY, true, ALL),
        (
            Instruction::Rest {
                destination: DICT,
                subject: KEY,
                from: ImmediateInt::new(0),
            },
            1,
            DICTIONARY,
            true,
            ALL,
        ),
    ] {
        let chunk = chunk([producer, lookup()]);
        let mut state = [Fact::UNKNOWN; 6];
        state[DICT.index() as usize] = Fact::array(mask, identity, false);
        let elements = [0, INT, 0, INT];
        transfer(&chunk, 1, &mut state, tracking.then_some(&elements), None);
        assert_eq!(state[READ.index() as usize].mask, expected);
    }
}
