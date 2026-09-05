use std::rc::Rc;

use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::verify::verify_unit;
use crate::compiler::CompileConfiguration;
use crate::compiler::compile_with_configuration;
use crate::optimizer::OptimizationConfiguration;
use crate::value::heap::Heap;

struct Compiled {
    unit: CompiledUnit,
    _heap: Rc<Heap>,
}

impl Compiled {
    fn chunk(&self) -> &Chunk {
        &self.unit.functions[0].chunk
    }
}

fn compile(source: &str) -> Compiled {
    let arena = LocalArena::new();
    let program = parse(&arena, source).expect("the match fixture parses");
    let heap = Heap::new();
    let unit = compile_with_configuration(
        program,
        "/project/matching.whim",
        &heap,
        CompileConfiguration {
            optimization: OptimizationConfiguration {
                enabled: false,
                ..OptimizationConfiguration::default()
            },
            trusted_return_types: false,
        },
    )
    .expect("the match fixture compiles");
    verify_unit(&unit).expect("unoptimized match bytecode verifies");
    Compiled { unit, _heap: heap }
}

#[test]
fn small_string_prefixes_keep_typed_catchall_in_one_pattern_table() {
    let compiled = compile(
        r"function choose(string $subject): int {
            return match ($subject) { 'foo' => 1, 'bar' => 2, string => 3 };
        }",
    );

    let chunk = compiled.chunk();
    let (position, table) = chunk
        .code
        .iter()
        .enumerate()
        .find_map(|(position, instruction)| {
            if let Instruction::SwitchPattern { table, .. } = instruction {
                Some((position, table))
            } else {
                None
            }
        })
        .expect("the small literal prefix and typed catchall share one table");
    let SwitchTable::Pattern {
        descriptors,
        default,
        ..
    } = &chunk.switch_tables[usize::from(table.index())]
    else {
        panic!("the pattern table retains the typed catchall");
    };

    assert!(matches!(
        descriptors.as_slice(),
        [TypeDescriptor::StringLiteral(first), TypeDescriptor::StringLiteral(second), TypeDescriptor::String]
            if first.as_bytes() == b"foo" && second.as_bytes() == b"bar"
    ));

    let target = usize::try_from(i64::try_from(position).unwrap() + i64::from(*default)).unwrap();
    assert!(matches!(
        chunk.code[target],
        Instruction::ThrowUnhandledMatch { .. }
    ));
    assert!(
        !chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Is { .. }))
    );
}

#[test]
fn larger_string_prefixes_keep_specialized_dispatch_before_a_typed_catchall() {
    let compiled = compile(
        r"function choose(mixed $subject): int {
            return match ($subject) {
                'first' => 1, 'second' => 2, 'third' => 3,
                'fourth' => 4, 'fifth' => 5, string => 6,
            };
        }",
    );
    let chunk = compiled.chunk();
    let (position, table) = chunk
        .code
        .iter()
        .enumerate()
        .find_map(|(position, instruction)| {
            if let Instruction::SwitchString { table, .. } = instruction {
                Some((position, table))
            } else {
                None
            }
        })
        .expect("the literal prefix uses a string switch without optimization");
    let SwitchTable::String { arms, default, .. } =
        &chunk.switch_tables[usize::from(table.index())]
    else {
        panic!("multi-byte strings use the string lookup table");
    };

    assert_eq!(arms.len(), 5);
    let target = usize::try_from(i64::try_from(position).unwrap() + i64::from(*default)).unwrap();
    let Instruction::Is { descriptor, .. } = chunk.code[target] else {
        panic!("a failed literal lookup must still test the typed catchall");
    };
    assert!(matches!(
        chunk.type_descriptors[usize::from(descriptor.index())],
        TypeDescriptor::String
    ));
    assert!(
        chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ThrowUnhandledMatch { .. }))
    );
    assert!(
        !chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SwitchPattern { .. }))
    );
}

#[test]
fn literal_groups_do_not_cross_deferred_type_tests() {
    let compiled = compile(
        r"function choose(mixed $subject): int {
            return match ($subject) {
                'first' => 1, 'second' => 2,
                MissingMatchType => 3,
                'third' => 4, 'fourth' => 5,
                $_ => 6,
            };
        }",
    );

    let chunk = compiled.chunk();
    let switches: Vec<_> = chunk
        .code
        .iter()
        .enumerate()
        .filter_map(|(position, instruction)| {
            matches!(instruction, Instruction::SwitchString { .. }).then_some(position)
        })
        .collect();
    assert_eq!(switches.len(), 2);
    assert!(chunk.code[switches[0] + 1..switches[1]].iter().any(|instruction| {
        let Instruction::Is { descriptor, .. } = instruction else { return false; };
        matches!(&chunk.type_descriptors[usize::from(descriptor.index())], TypeDescriptor::Named { name, .. } if name.as_bytes() == b"MissingMatchType")
    }));
}

#[test]
fn mixed_literal_kinds_use_separate_existing_switches() {
    let compiled = compile(
        r"function choose(mixed $subject): int {
            return match ($subject) {
                -4 => 1, -3 => 2, -2 => 3, -1 => 4,
                0 => 5, 1 => 6, 2 => 7, 3 => 8,
                -0.0 => 9, 1.0 => 10,
                'a' => 11, 'b' => 12,
                $_ => 13,
            };
        }",
    );

    let chunk = compiled.chunk();
    assert_eq!(chunk.switch_tables.len(), 3);
    assert!(
        matches!(&chunk.switch_tables[0], SwitchTable::Int { base: -4, targets, .. } if targets.len() == 8)
    );
    assert!(
        matches!(&chunk.switch_tables[1], SwitchTable::Float { values, .. } if values.len() == 2)
    );
    assert!(
        matches!(&chunk.switch_tables[2], SwitchTable::StringByte { base: b'a', targets, .. } if targets.len() == 2)
    );
}

#[test]
fn sparse_integer_groups_keep_the_existing_table_size_limit() {
    let compiled = compile(
        r"function choose(mixed $subject): int {
            return match ($subject) {
                -9223372036854775808 => 1, 9223372036854775807 => 2,
                'first' => 3, 'second' => 4,
                $_ => 5,
            };
        }",
    );

    assert!(
        !compiled
            .chunk()
            .switch_tables
            .iter()
            .any(|table| matches!(table, SwitchTable::Int { .. }))
    );
    assert!(
        compiled
            .chunk()
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SwitchString { .. }))
    );
}
