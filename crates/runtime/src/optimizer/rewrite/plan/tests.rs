use whim_span::Span;

use super::ChunkRewrite;
use super::RewritePlan;
use super::compact_switch_tables;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::chunk::descriptors::string_switch_buckets;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Count;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::SwitchTableIndex;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::verify::verify;
use crate::optimizer::passes::FunctionLocation;
use crate::value::heap::Heap;

const SUBJECT: Register = Register::new(0);

fn pattern_table() -> SwitchTable {
    SwitchTable::Pattern {
        descriptors: vec![TypeDescriptor::String],
        targets: vec![1],
        default: 1,
    }
}

fn string_table(heap: &Heap, target: i32, default: i32) -> SwitchTable {
    let arms = vec![(heap.intern(b"foo"), target)];
    let buckets = string_switch_buckets(&arms);
    SwitchTable::String {
        arms,
        buckets,
        default,
    }
}

fn unit(heap: &Heap, main: Chunk) -> CompiledUnit {
    CompiledUnit {
        path: heap.intern(b"/switch-table-rewrite.whim"),
        main,
        functions: Vec::new(),
        classes: Vec::new(),
        constants: Vec::new(),
        type_aliases: Vec::new(),
        newtypes: Vec::new(),
    }
}

#[test]
fn added_switch_tables_discard_replaced_table_after_code_compaction() {
    let heap = Heap::new();
    let mut chunk = Chunk::new();
    chunk.register_count = 1;
    let old = chunk.add_switch_table(pattern_table()).unwrap();
    chunk.emit(
        Instruction::SwitchPattern {
            subject: SUBJECT,
            table: old,
        },
        Span::zero(),
    );
    chunk.emit(
        Instruction::LoadTrue {
            destination: SUBJECT,
        },
        Span::zero(),
    );
    chunk.emit(Instruction::ReturnNull, Span::zero());
    chunk.emit(Instruction::ReturnNull, Span::zero());
    let mut rewrite = ChunkRewrite::new(FunctionLocation::Main);
    rewrite.prepare(chunk.code.len());
    rewrite.switch_tables.push(string_table(&heap, 3, 2));
    rewrite.replacements[0] = Some(Instruction::SwitchString {
        subject: SUBJECT,
        table: SwitchTableIndex::new(1),
    });
    rewrite.removals[1] = true;
    let mut unit = unit(&heap, chunk);

    RewritePlan {
        chunks: vec![rewrite],
    }
    .apply(&mut unit);

    assert_eq!(unit.main.switch_tables.len(), 1);
    assert!(
        matches!(unit.main.code[0], Instruction::SwitchString { table, .. } if table.index() == 0)
    );
    assert!(
        matches!(&unit.main.switch_tables[0], SwitchTable::String { arms, default: 1, .. } if arms[0].1 == 2)
    );
    verify(&unit.main).expect("the retained table uses rebased instruction offsets");
}

#[test]
fn shared_switch_table_survives_one_consumers_conversion_in_original_order() {
    let heap = Heap::new();
    let mut chunk = Chunk::new();
    chunk.register_count = 1;
    chunk.add_switch_table(pattern_table()).unwrap();
    let shared = chunk.add_switch_table(pattern_table()).unwrap();
    for _ in 0..2 {
        chunk.emit(
            Instruction::SwitchPattern {
                subject: SUBJECT,
                table: shared,
            },
            Span::zero(),
        );
    }
    chunk.emit(Instruction::ReturnNull, Span::zero());
    let mut rewrite = ChunkRewrite::new(FunctionLocation::Main);
    rewrite.prepare(chunk.code.len());
    rewrite.switch_tables.push(string_table(&heap, 1, 1));
    rewrite.replacements[0] = Some(Instruction::SwitchString {
        subject: SUBJECT,
        table: SwitchTableIndex::new(2),
    });
    let mut unit = unit(&heap, chunk);

    RewritePlan {
        chunks: vec![rewrite],
    }
    .apply(&mut unit);

    assert_eq!(unit.main.switch_tables.len(), 2);
    assert!(matches!(
        unit.main.switch_tables[0],
        SwitchTable::Pattern { .. }
    ));
    assert!(matches!(
        unit.main.switch_tables[1],
        SwitchTable::String { .. }
    ));
    assert!(
        matches!(unit.main.code[0], Instruction::SwitchString { table, .. } if table.index() == 1)
    );
    assert!(
        matches!(unit.main.code[1], Instruction::SwitchPattern { table, .. } if table.index() == 0)
    );
    verify(&unit.main).expect("both live consumers retain their own table kind");
}

fn switch_cases(heap: &Heap) -> [(Instruction, SwitchTable); 6] {
    let table = SwitchTableIndex::new(1);
    [
        (
            Instruction::SwitchInt {
                subject: SUBJECT,
                table,
            },
            SwitchTable::Int {
                base: 0,
                targets: vec![1],
                default: 1,
            },
        ),
        (
            Instruction::SwitchString {
                subject: SUBJECT,
                table,
            },
            string_table(heap, 1, 1),
        ),
        (
            Instruction::SwitchBool {
                subject: SUBJECT,
                table,
            },
            SwitchTable::Bool {
                targets: vec![1, 1],
                default: 1,
            },
        ),
        (
            Instruction::SwitchFloat {
                subject: SUBJECT,
                table,
            },
            SwitchTable::Float {
                values: vec![0.0],
                targets: vec![1],
                default: 1,
            },
        ),
        (
            Instruction::SwitchPattern {
                subject: SUBJECT,
                table,
            },
            pattern_table(),
        ),
        (
            Instruction::SwitchTuplePattern {
                first_element: SUBJECT,
                element_count: Count::new(1),
                table,
            },
            SwitchTable::Pattern {
                descriptors: vec![TypeDescriptor::Tuple(vec![TypeDescriptor::String])],
                targets: vec![1],
                default: 1,
            },
        ),
    ]
}

#[test]
fn switch_table_compaction_remaps_every_switch_opcode() {
    let heap = Heap::new();
    for (instruction, table) in switch_cases(&heap) {
        let mut chunk = Chunk::new();
        chunk.register_count = 1;
        chunk.add_switch_table(pattern_table()).unwrap();
        chunk.add_switch_table(table).unwrap();
        chunk.emit(instruction, Span::zero());
        chunk.emit(Instruction::ReturnNull, Span::zero());
        compact_switch_tables(&mut chunk);
        assert_eq!(chunk.switch_tables.len(), 1);
        assert_eq!(chunk.code[0].kind(), instruction.kind());
        verify(&chunk).expect("the remapped switch preserves its operands and table kind");
    }
}
