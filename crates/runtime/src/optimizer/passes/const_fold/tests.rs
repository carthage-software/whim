use whim_span::Span;

use super::optimize_chunk;
use super::optimize_unit;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::CatchEntry;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Count;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::verify::verify;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::IndexedUnit;
use crate::optimizer::type_flow::World;
use crate::value::heap::Heap;

const INPUT: Register = Register::new(0);
const CONDITION: Register = Register::new(1);

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
