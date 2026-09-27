use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::rewrite::relative_target;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

const FIXTURE: &str = include_str!("../../../../tests/_fixtures/uint-numeric-loops.whim");

#[test]
fn unsigned_accumulators_keep_signed_numeric_loop_selection() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for function in &unit.functions {
        let code = &function.chunk.code;
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::IntNumericLoop { .. })),
            "{:?}: {code:?}",
            function.name
        );
        assert!(
            code.iter().any(|instruction| matches!(
                instruction,
                Instruction::IntegerAddAssign {
                    kind: IntegerKind::U64,
                    ..
                }
            )),
            "{:?}: {code:?}",
            function.name
        );
    }
    let code = &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"sum_lengths")
        .unwrap()
        .chunk
        .code;
    let length = code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::StringLength { .. }))
        .unwrap();
    let header = code
        .iter()
        .position(|instruction| matches!(instruction, Instruction::IntNumericLoop { .. }))
        .unwrap();
    let addition = code
        .iter()
        .position(|instruction| {
            matches!(
                instruction,
                Instruction::IntegerAddAssign {
                    kind: IntegerKind::U64,
                    ..
                }
            )
        })
        .unwrap();
    assert!(length < header && header < addition, "{code:?}");
    assert!(
        code.iter()
            .enumerate()
            .any(|(position, instruction)| matches!(instruction,
                Instruction::IntCounterLoop { offset, .. }
                    if relative_target(position, i32::from(offset.offset())) == addition
            )),
        "{code:?}"
    );
}

#[test]
fn unsupported_unsigned_operations_do_not_select_numeric_loops() {
    for operation in [
        "$sum -= $step",
        "$sum *= $step",
        "$sum %= $step",
        "$sum <<= 1",
    ] {
        let source = format!(
            "function update(uint $sum, uint $step, int $count): uint {{
            for ($index = 0; $index < $count; $index++) {{ {operation}; }}
            return $sum;
        }}"
        );
        let unit = compile(&source, OptimizationConfiguration::default());
        verify_unit(&unit).unwrap();
        let code = &unit.functions[0].chunk.code;
        assert!(
            !code.iter().any(|instruction| matches!(
                instruction,
                Instruction::NumericLoop { .. }
                    | Instruction::IntNumericLoop { .. }
                    | Instruction::PreparedIntNumericLoop { .. }
                    | Instruction::NumericRegionJump { .. }
            )),
            "{operation}: {code:?}"
        );
    }
}

#[test]
fn unsigned_constants_and_loads_stay_in_numeric_loops() {
    let unit = compile(
        r"
function replace_values(vec<uint> $values, int $count): vec<uint> {
    for ($index = 0; $index < $count; $index++) {
        if ($index == 0) { $values[$index] = 3u; }
        else { $values[$index] = 18446744073709551615u; }
    }
    return $values;
}
",
        OptimizationConfiguration {
            licm: false,
            ..OptimizationConfiguration::default()
        },
    );
    verify_unit(&unit).unwrap();
    let chunk = &unit.functions[0].chunk;
    assert!(
        chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::IntNumericLoop { .. })),
        "{:?}",
        chunk.code
    );
    assert!(
        chunk.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::LoadInteger {
                kind: IntegerKind::U64,
                ..
            }
        )),
        "{:?}",
        chunk.code
    );
    assert!(
        chunk.code.iter().any(|instruction| matches!(instruction,
            Instruction::LoadConstant { constant, .. }
                if matches!(chunk.constants[usize::from(constant.index())], Literal::Uint(u64::MAX))
        )),
        "{:?}",
        chunk.code
    );
}
