use whim_bytecode::chunk::descriptors::IcDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayValueMode;
use whim_bytecode::unit::CompiledFunction;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

const FIXTURE: &str = include_str!("../../../../tests/_fixtures/nested-record-proofs.whim");

fn function<'a>(unit: &'a CompiledUnit, name: &str) -> &'a CompiledFunction {
    unit.functions
        .iter()
        .find(|function| function.name.as_bytes() == name.as_bytes())
        .unwrap()
}

fn checked_call(function: &CompiledFunction, callee: &str) -> bool {
    function.chunk.code.iter().any(|instruction| {
        let Instruction::CallNamed { cache, .. } = instruction else {
            return false;
        };
        matches!(
            &function.chunk.ic_descriptors[usize::from(cache.index())],
            IcDescriptor::Member { name, .. } if name.as_bytes() == callee.as_bytes()
        )
    })
}

#[test]
fn nested_records_and_vector_shapes_skip_repeated_argument_checks() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for (name, callee) in [("totals", "total"), ("forward", "first")] {
        let function = function(&unit, name);
        assert!(
            !checked_call(function, callee),
            "{name}: {:?}",
            function.chunk.code
        );
        assert!(
            function.chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallNamedUnchecked { .. } | Instruction::CallNamedDirect { .. }
            )),
            "{name}: {:?}",
            function.chunk.code
        );
    }
    assert!(checked_call(
        function(&unit, "changed_cell"),
        "accepts_cell"
    ));
}

#[test]
fn nested_updates_keep_specialized_reads_iteration_and_arithmetic() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    let code = &function(&unit, "updated").chunk.code;
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::VecForeachNext { .. })),
        "{code:#?}"
    );

    assert!(
        !code.iter().any(|instruction| matches!(
            instruction,
            Instruction::IndexGet { .. }
                | Instruction::ForeachNext { .. }
                | Instruction::Multiply { kind: None, .. }
                | Instruction::Add { kind: None, .. }
                | Instruction::Return { .. }
        )),
        "{code:#?}"
    );
}

#[test]
fn numeric_index_modes_use_the_producer_even_before_unrelated_instructions() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for name in ["fields", "positions"] {
        let function = function(&unit, name);
        let modes: Vec<_> = function
            .chunk
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::DictIndexGetStringKey { value_mode, .. }
                | Instruction::VecIndexGet { value_mode, .. } => Some(*value_mode),
                _ => None,
            })
            .collect();
        assert_eq!(
            modes,
            [
                ArrayValueMode::Int,
                ArrayValueMode::Uint,
                ArrayValueMode::Float
            ],
            "{name}: {:?}",
            function.chunk.code
        );
    }
    let total = function(&unit, "total");
    assert_eq!(
        total
            .chunk
            .code
            .iter()
            .filter(|instruction| matches!(
                instruction,
                Instruction::DictIndexGetStringKey {
                    value_mode: ArrayValueMode::Int,
                    ..
                }
            ))
            .count(),
        3,
        "{:?}",
        total.chunk.code
    );
    assert!(
        total
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::IndexGetPath { .. }))
    );
    assert!(
        total
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ReturnScalarUnchecked { .. })),
        "{:?}",
        total.chunk.code
    );
    let changed = function(&unit, "changed");
    assert!(
        changed.chunk.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::DictIndexGetStringKey {
                value_mode: ArrayValueMode::Generic,
                ..
            }
        )),
        "{:?}",
        changed.chunk.code
    );
}

#[test]
fn numeric_index_modes_preserve_key_kinds_and_union_fields() {
    for (shape, key) in [
        ("dict[1 => int, 1u => string]", "1u"),
        ("dict['n' => int|string]", "'n'"),
        ("vec[int, string]", "1u"),
    ] {
        let source = format!(
            "function read({shape} $row): (mixed, uint) {{ $value = $row[{key}]; $other = 2u; return ($value, $other); }}"
        );
        let unit = compile(&source, OptimizationConfiguration::default());
        verify_unit(&unit).unwrap();
        let function = function(&unit, "read");
        assert!(
            function.chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::DictIndexGetUintKey {
                    value_mode: ArrayValueMode::Generic,
                    ..
                } | Instruction::DictIndexGetStringKey {
                    value_mode: ArrayValueMode::Generic,
                    ..
                } | Instruction::VecIndexGet {
                    value_mode: ArrayValueMode::Generic,
                    ..
                }
            )),
            "{source}: {:?}",
            function.chunk.code
        );
    }
}

#[test]
fn equal_shapes_keep_symbol_resolution_and_mutable_property_checks() {
    for shape in [
        "dict['item' => Holder<Missing>]",
        "dict['item' => Defaulted<int>]",
        "dict['item' => Bounded<int>]",
        "dict['item' => DefaultAlias]",
        "dict['item' => BoundAlias<int>]",
        "dict['item' => Hidden]",
        "dict['item' => Recursive]",
        "dict['item' => #{ value: int }]",
        "vec[Holder<Missing>, ...int]",
    ] {
        let source = format!(
            r"
final class Holder<out T> {{}}
final class Defaulted<out T, out U = Missing> {{}}
final class Bounded<out T: Missing|int> {{}}
type DefaultAlias<T = Missing> = int;
type BoundAlias<T: Missing|int> = T;
type Erase<T> = int;
type Hidden = Erase<Missing>;
type Recursive = int|vec<Recursive>;
#[Whim\Marker\NeverInline]
function accept({shape} $row): void {{}}
function forward({shape} $row): void {{ accept($row); }}
"
        );
        let unit = compile(&source, OptimizationConfiguration::default());
        verify_unit(&unit).unwrap();
        let function = function(&unit, "forward");
        assert!(
            checked_call(function, "accept"),
            "{shape}: {:?}",
            function.chunk.code
        );
    }
}

#[test]
fn shape_equality_keeps_key_kinds_rest_types_and_mutations_distinct() {
    for (actual, expected, body) in [
        ("dict[1 => int]", "dict[1u => int]", "accept($row);"),
        ("dict['1' => int]", "dict[1 => int]", "accept($row);"),
        ("dict[true => int]", "dict[1 => int]", "accept($row);"),
        (
            "dict['n' => int, ...]",
            "dict['n' => int, ...<string, int>]",
            "accept($row);",
        ),
        ("vec[int, ...string]", "vec[int, ...int]", "accept($row);"),
        (
            "dict['n' => int]",
            "dict['n' => int]",
            "$row['n'] = 'wrong'; accept($row);",
        ),
        (
            "dict['child' => dict['n' => int]]",
            "dict['child' => dict['n' => int]]",
            "$row['child']['n'] = 'wrong'; accept($row);",
        ),
    ] {
        let source = format!(
            r"
#[Whim\Marker\NeverInline]
function accept({expected} $row): void {{}}
function forward({actual} $row): void {{ {body} }}
"
        );
        let unit = compile(&source, OptimizationConfiguration::default());
        verify_unit(&unit).unwrap();
        let function = function(&unit, "forward");
        assert!(
            checked_call(function, "accept"),
            "{source}: {:?}",
            function.chunk.code
        );
    }
}
