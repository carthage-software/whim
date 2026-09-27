use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

#[test]
fn inlined_integer_results_fuse_into_compound_immediates() {
    let unit = compile(
        r"
function signed_value(): int { return 5; }
function unsigned_value(string $value): uint { return length!($value); }
function signed_total(int $count): int {
    $sum = 0;
    for ($index = 0; $index < $count; $index++) { $sum += signed_value(); }
    return $sum;
}
function unsigned_total(int $count): uint {
    $sum = 0u;
    for ($index = 0; $index < $count; $index++) { $sum += unsigned_value('hallo'); }
    return $sum;
}
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for (name, kind) in [
        ("signed_total", IntegerKind::I64),
        ("unsigned_total", IntegerKind::U64),
    ] {
        let chunk = &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name.as_bytes())
            .unwrap()
            .chunk;
        assert!(
            chunk.code.iter().any(|instruction| matches!(instruction,
                Instruction::AddImmediate { destination, source, immediate, kind: Some(actual) }
                    if destination == source && *actual == kind && immediate.as_uint() == 5
            )),
            "{name}: {:?}",
            chunk.code
        );
        if kind == IntegerKind::U64 {
            assert!(
                chunk
                    .code
                    .iter()
                    .any(|instruction| matches!(instruction, Instruction::IntNumericLoop { .. })),
                "{name}: {:?}",
                chunk.code
            );
        }
        assert!(!chunk.code.windows(2).any(|pair| matches!(pair,
            [Instruction::LoadInteger { destination, .. }, Instruction::IntegerAddAssign { source, .. }]
                if destination == source
        )), "{name}: {:?}", chunk.code);
    }
}

#[test]
fn unsigned_immediates_keep_numeric_loop_selection_but_other_steps_do_not() {
    for statement in ["$sum += 65535u", "$sum = $value + 65535u"] {
        let source = format!(
            "function calculate(uint $value, int $count): uint {{
            $sum = 0u;
            for ($index = 0; $index < $count; $index++) {{ {statement}; }}
            return $sum;
        }}"
        );
        let unit = compile(
            &source,
            OptimizationConfiguration {
                licm: false,
                ..OptimizationConfiguration::default()
            },
        );
        verify_unit(&unit).unwrap();
        let code = &unit.functions[0].chunk.code;
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::IntNumericLoop { .. })),
            "{statement}: {code:?}"
        );
        assert!(
            code.iter().any(|instruction| matches!(instruction,
                Instruction::AddImmediate { kind: Some(IntegerKind::U64), immediate, .. }
                    if immediate.as_uint() == 65535
            )),
            "{statement}: {code:?}"
        );
    }
    for statement in ["$sum -= 3u", "$sum++"] {
        let source = format!(
            "function calculate(uint $sum, int $count): uint {{
            for ($index = 0; $index < $count; $index++) {{ {statement}; }}
            return $sum;
        }}"
        );
        let unit = compile(&source, OptimizationConfiguration::default());
        verify_unit(&unit).unwrap();
        let code = &unit.functions[0].chunk.code;
        assert!(
            !code.iter().any(|instruction| matches!(
                instruction,
                Instruction::IntNumericLoop { .. }
                    | Instruction::NumericLoop { .. }
                    | Instruction::PreparedIntNumericLoop { .. }
            )),
            "{statement}: {code:?}"
        );
    }
}
