use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::CatchEntry;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::Count;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::verify::verify;
use whim_span::Span;
use whim_value::heap::Heap;

use super::optimize_chunk;
use super::optimize_unit;
use super::prepare_chunk;
use crate::OptimizationConfiguration;
use crate::OptimizationStatistics;
use crate::analysis::Analysis;
use crate::rewrite::plan::RewritePlan;
use crate::type_flow::IndexedUnit;
use crate::type_flow::World;

const INPUT: Register = Register::new(0);
const CONDITION: Register = Register::new(1);

#[test]
fn repeated_path_keys_survive_mutations_but_not_register_writes_or_joins() {
    let heap = Heap::new();
    let mut chunk = Chunk::new();
    chunk.register_count = 6;
    chunk.local_register_count = 2;
    let field = chunk
        .add_constant(Literal::String(heap.intern(b"field")))
        .unwrap();
    let key = Register::new(2);
    let copy = Register::new(3);
    let other = Register::new(4);
    let output = Register::new(5);
    let load = Instruction::LoadConstant {
        destination: key,
        constant: field,
    };

    let repeated = Instruction::Move {
        destination: copy,
        source: key,
    };

    let path = Instruction::IndexSetPath {
        index_count: Count::new(2),
        container: INPUT,
        first_index: key,
        value: CONDITION,
    };

    for instruction in [
        load,
        repeated,
        path,
        load,
        repeated,
        path,
        Instruction::MoveOwned {
            destination: other,
            source: key,
        },
        load,
        Instruction::JumpIfFalse {
            condition: CONDITION,
            offset: JumpOffset::new(2),
        },
        load,
        load,
        Instruction::IndexGetPath {
            index_count: Count::new(2),
            destination: output,
            container: INPUT,
            first_index: key,
        },
        Instruction::Return { source: output },
    ] {
        chunk.emit(instruction, Span::zero());
    }

    let mut statistics = OptimizationStatistics::default();
    prepare_chunk(
        &mut chunk,
        OptimizationConfiguration::default(),
        &mut statistics,
    );

    verify(&chunk).unwrap();
    assert_eq!(statistics.instructions_removed, 2);
    assert_eq!(
        chunk
            .code
            .iter()
            .filter(|instruction| **instruction == load)
            .count(),
        4
    );

    assert_eq!(
        chunk
            .code
            .iter()
            .filter(|instruction| **instruction == repeated)
            .count(),
        1
    );
}

#[test]
fn constant_keys_load_into_their_consuming_window() {
    let heap = Heap::new();
    let mut chunk = Chunk::new();
    chunk.register_count = 6;
    chunk.local_register_count = 1;
    let row = chunk
        .add_constant(Literal::String(heap.intern(b"row")))
        .unwrap();
    let value = chunk
        .add_constant(Literal::String(heap.intern(b"value")))
        .unwrap();
    for instruction in [
        Instruction::LoadConstant {
            destination: Register::new(1),
            constant: row,
        },
        Instruction::LoadConstant {
            destination: Register::new(2),
            constant: value,
        },
        Instruction::Move {
            destination: Register::new(3),
            source: Register::new(1),
        },
        Instruction::MoveOwned {
            destination: Register::new(4),
            source: Register::new(2),
        },
        Instruction::IndexGetPath {
            index_count: Count::new(2),
            destination: Register::new(5),
            container: Register::new(0),
            first_index: Register::new(3),
        },
        Instruction::Return {
            source: Register::new(5),
        },
    ] {
        chunk.emit(instruction, Span::zero());
    }
    optimize_chunk(
        &mut chunk,
        &heap,
        OptimizationConfiguration::default(),
        &mut OptimizationStatistics::default(),
    );
    verify(&chunk).unwrap();
    assert_eq!(chunk.code.len(), 4, "{:?}", chunk.code);
    assert!(
        matches!(chunk.code[0], Instruction::LoadConstant { destination, constant } if destination == Register::new(3) && constant == row)
    );
    assert!(
        matches!(chunk.code[1], Instruction::LoadConstant { destination, constant } if destination == Register::new(4) && constant == value)
    );
}

fn assertion_chunk(heap: &Heap, success: bool) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.register_count = 2;
    chunk.local_register_count = 1;
    let text = chunk
        .add_constant(Literal::String(heap.intern(b"!input")))
        .unwrap();
    let input = if success {
        Instruction::LoadFalse { destination: INPUT }
    } else {
        Instruction::LoadTrue { destination: INPUT }
    };
    for instruction in [
        input,
        Instruction::Not {
            destination: CONDITION,
            source: INPUT,
        },
        Instruction::Assert {
            operand_count: Count::new(0),
            first_value: CONDITION,
            message: Register::NONE,
            text,
        },
        Instruction::Exit {
            code: Register::NONE,
        },
    ] {
        chunk.emit(instruction, Span::zero());
    }

    chunk
}

fn fold(heap: &Heap, chunk: Chunk) -> (Chunk, OptimizationStatistics) {
    verify(&chunk).expect("the assertion fixture verifies");
    let mut unit = CompiledUnit {
        path: heap.intern(b"/optimizer/assertion-progress.whim"),
        files: Vec::new(),
        main: chunk,
        functions: Vec::new(),
        classes: Vec::new(),
        constants: Vec::new(),
        type_aliases: Vec::new(),
        newtypes: Vec::new(),
    };

    let configuration = OptimizationConfiguration::default();
    let world = World::new(&[], &[]);
    let mut statistics = OptimizationStatistics::default();
    let plan = {
        let indexed = IndexedUnit::with_world(&unit, &world);
        let analysis = Analysis::of(&indexed, configuration, heap);
        let mut plan = RewritePlan::for_analysis(&analysis);
        optimize_unit(&analysis, &mut plan, configuration, &mut statistics);
        plan
    };

    plan.apply(&mut unit);
    verify(&unit.main).expect("the folded assertion verifies");
    (unit.main, statistics)
}

#[test]
fn terminal_assertion_keeps_fold_statistics_without_requesting_analysis() {
    let heap = Heap::new();
    let (chunk, statistics) = fold(&heap, assertion_chunk(&heap, true));
    assert!(matches!(
        chunk.code[1],
        Instruction::LoadTrue {
            destination: CONDITION
        }
    ));
    assert_eq!(statistics.constants_folded, 1);
    assert_eq!(statistics.terminal_assertion_constants, 1);
    assert_eq!(statistics.specialized_total(), 0);
}

#[test]
fn returned_reused_and_protected_assertion_values_still_request_analysis() {
    let heap = Heap::new();
    let initial = assertion_chunk(&heap, true);
    let mut returned = initial.clone();
    returned.code[3] = Instruction::ReturnUnchecked { source: CONDITION };
    let mut reused = initial.clone();
    reused.code[3] = initial.code[2];
    reused.emit(
        Instruction::Exit {
            code: Register::NONE,
        },
        Span::zero(),
    );
    let mut local = initial.clone();
    local.local_register_count = 2;
    let mut snapshot = initial.clone();
    snapshot.local_register_count = 2;
    snapshot.parameter_register_count = 1;
    snapshot.trace_argument_registers.push(CONDITION);
    let mut not_immediate = initial;
    not_immediate
        .code
        .insert(2, Instruction::Clear { target: INPUT });
    not_immediate.spans.insert(2, Span::zero());
    for chunk in [returned, reused, local, snapshot, not_immediate] {
        let (_, statistics) = fold(&heap, chunk);
        assert_eq!(statistics.constants_folded, 1);
        assert_eq!(statistics.terminal_assertion_constants, 0);
        assert_eq!(statistics.specialized_total(), 1);
    }
}

#[test]
fn assertion_progress_accounts_for_catch_handler_reads() {
    let heap = Heap::new();
    let mut chunk = assertion_chunk(&heap, true);
    let Instruction::Assert { text, .. } = chunk.code[2] else {
        unreachable!();
    };
    chunk.code[3] = Instruction::Assert {
        operand_count: Count::new(0),
        first_value: INPUT,
        message: Register::NONE,
        text,
    };
    chunk.emit(
        Instruction::Exit {
            code: Register::NONE,
        },
        Span::zero(),
    );
    chunk.emit(
        Instruction::ReturnUnchecked { source: CONDITION },
        Span::zero(),
    );
    let type_descriptor = chunk.add_type_descriptor(TypeDescriptor::Mixed).unwrap();
    chunk.catch_table.push(CatchEntry {
        start: 3,
        end: 4,
        handler: 5,
        type_descriptor,
        temporary_floor: 2,
        binding: None,
    });
    let (_, statistics) = fold(&heap, chunk);
    assert_eq!(statistics.constants_folded, 1);
    assert_eq!(statistics.terminal_assertion_constants, 0);
    assert_eq!(statistics.specialized_total(), 1);
}

#[test]
fn failing_assertions_and_local_folding_keep_existing_progress_accounting() {
    let heap = Heap::new();
    let (chunk, statistics) = fold(&heap, assertion_chunk(&heap, false));
    assert!(matches!(
        chunk.code[1],
        Instruction::LoadFalse {
            destination: CONDITION
        }
    ));
    assert_eq!(statistics.constants_folded, 1);
    assert_eq!(statistics.terminal_assertion_constants, 0);
    assert_eq!(statistics.specialized_total(), 1);

    let mut chunk = assertion_chunk(&heap, true);
    let mut statistics = OptimizationStatistics::default();
    optimize_chunk(
        &mut chunk,
        &heap,
        OptimizationConfiguration::default(),
        &mut statistics,
    );
    assert_eq!(statistics.constants_folded, 1);
    assert_eq!(statistics.terminal_assertion_constants, 0);
    assert_eq!(statistics.specialized_total(), 1);
}
