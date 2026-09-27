use whim_bytecode::chunk::descriptors::IcDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

#[test]
fn nested_shape_fields_keep_their_element_types() {
    let unit = compile(
        r"
        type Line = dict['quantity' => int, 'price' => int];
        type Order = dict['lines' => vec<Line>, 'shipping' => dict['price' => int]];
        function total(Order $order): int {
            $total = 0;
            foreach ($order['lines'] as $line) {
                $total += $line['quantity'] * $line['price'];
            }
            return $total + $order['shipping']['price'];
        }
        ",
        OptimizationConfiguration::default(),
    );
    let code = &unit.functions[0].chunk.code;
    assert!(
        code.iter().any(|instruction| matches!(
            instruction,
            Instruction::Multiply {
                kind: Some(IntegerKind::I64),
                ..
            }
        )),
        "{code:#?}",
    );
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::VecForeachNext { .. })),
        "{code:#?}",
    );
    assert!(
        !code.iter().any(|instruction| matches!(
            instruction,
            Instruction::IndexGet { .. }
                | Instruction::ForeachNext { .. }
                | Instruction::Multiply { kind: None, .. }
                | Instruction::Add { kind: None, .. }
        )),
        "{code:#?}",
    );
    verify_unit(&unit).unwrap();
}

#[test]
fn shape_key_types_and_vector_positions_prove_only_matching_returns() {
    for (shape, key, result, proven) in [
        (
            "dict[1 => int, 1u => string, true => bool, '1' => float]",
            "1",
            "int",
            true,
        ),
        (
            "dict[1 => int, 1u => string, true => bool, '1' => float]",
            "1u",
            "string",
            true,
        ),
        (
            "dict[1 => int, 1u => string, true => bool, '1' => float]",
            "true",
            "bool",
            true,
        ),
        (
            "dict[1 => int, 1u => string, true => bool, '1' => float]",
            "'1'",
            "float",
            true,
        ),
        (
            "dict[1 => int, 1u => string, true => bool, '1' => float]",
            "1u",
            "int",
            false,
        ),
        (
            "dict[1 => int, 1u => string, true => bool, '1' => float]",
            "'1'",
            "int",
            false,
        ),
        ("dict['n' => int, ...<string, bool>]", "'n'", "int", true),
        (
            "dict['n' => int, ...<string, bool>]",
            "'other'",
            "bool",
            true,
        ),
        ("dict['n' => int, ...]", "'other'", "int", false),
        ("dict['n' => FutureValue]", "'n'", "int", false),
        ("dict['n' => int]|dict['n' => string]", "'n'", "int", false),
        ("vec[int, string, ...bool]", "0", "int", true),
        ("vec[int, string, ...bool]", "1u", "string", true),
        ("vec[int, string, ...bool]", "2u", "bool", true),
        ("vec[int, string]", "1", "int", false),
        ("vec[int, string]", "'0'", "int", false),
        ("vec[int, string]", "true", "int", false),
    ] {
        let source = format!("function read({shape} $value): {result} {{ return $value[{key}]; }}");
        let unit = compile(&source, OptimizationConfiguration::default());
        let code = &unit.functions[0].chunk.code;
        assert_eq!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            !proven,
            "{source}: {code:#?}",
        );
        verify_unit(&unit).unwrap();
    }
}

#[test]
fn writes_discard_shape_field_proofs() {
    for body in [
        "$row['n'] = $value; return $row['n'];",
        "$copy = $row; $copy['n'] = $value; return $copy['n'];",
        "if ($flag) { $row['n'] = $value; } return $row['n'];",
        "$row['child']['n'] = $value; return $row['child']['n'];",
    ] {
        let source = format!(
            "function read(dict['n' => int, 'child' => dict['n' => int]] $row, mixed $value, bool $flag): int {{ {body} }}"
        );
        let unit = compile(&source, OptimizationConfiguration::default());
        let code = &unit.functions[0].chunk.code;
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            "{source}: {code:#?}",
        );
        verify_unit(&unit).unwrap();
    }
}

#[test]
fn mutable_properties_inside_shapes_keep_argument_checks() {
    let unit = compile(
        r"
        #[Whim\Marker\NeverInline]
        function accept(#{ value: int } $item): void {}
        function run(dict['item' => #{ value: int }] $row): void {
            $item = $row['item'];
            $item->value = 'wrong';
            accept($row['item']);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let function = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"run")
        .unwrap();
    assert!(
        function.chunk.code.iter().any(|instruction| {
            let Instruction::CallNamed { cache, .. } = instruction else {
                return false;
            };
            matches!(
                &function.chunk.ic_descriptors[usize::from(cache.index())],
                IcDescriptor::Member { name, .. } if name.as_bytes() == b"accept"
            )
        }),
        "{:#?}",
        function.chunk.code,
    );
    verify_unit(&unit).unwrap();
}
