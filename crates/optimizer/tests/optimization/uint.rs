use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::word::InstructionKind;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

#[test]
fn unsigned_constants_fold_without_narrowing_or_signed_return_proofs() {
    let unit = compile(
        r"
function folded(): uint { return (1u << 63) + 1u; }
function unsigned(uint $value): uint { return $value; }
function signed(uint $value): int { return $value; }
function ordered(): bool { return 9007199254740993u > 9007199254740992.0; }
function matched(): uint { return match (1u) { 1 => 10u, 1u => 20u }; }
function overflow(): uint { return 18446744073709551615u + 1u; }
function mixed(uint $value): uint { return $value + 1; }
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    let function = |name: &[u8]| {
        unit.functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .unwrap()
    };
    let folded = function(b"folded");
    assert!(
        folded
            .chunk
            .constants
            .iter()
            .any(|literal| matches!(literal, Literal::Uint(9_223_372_036_854_775_809)))
    );
    assert!(!folded.chunk.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::Add { .. } | Instruction::ShiftLeft { .. }
    )));
    assert!(
        !function(b"unsigned")
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Return { .. }))
    );
    assert!(
        function(b"signed")
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Return { .. }))
    );
    assert!(
        function(b"ordered")
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::LoadTrue { .. }))
    );
    assert!(
        function(b"matched")
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ReturnIntegerUnchecked { kind: IntegerKind::U64, immediate } | Instruction::LoadInteger { kind: IntegerKind::U64, immediate, .. } if immediate.as_uint() == 20))
    );
    assert!(
        function(b"overflow")
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(
                instruction,
                Instruction::IntegerAddImmediate {
                    kind: IntegerKind::U64,
                    ..
                } | Instruction::IntegerAdd {
                    kind: IntegerKind::U64,
                    ..
                }
            ))
    );
    assert!(
        function(b"mixed")
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(
                instruction,
                Instruction::AddImmediate { .. } | Instruction::Add { .. }
            ))
    );
}

#[test]
fn unsigned_operands_select_specialized_instructions() {
    let unit = compile(
        r"
function arithmetic(uint $a, uint $b): vec<uint> {
    return vec[$a + $b, $a - $b, $a * $b, $a % $b, $a & $b, $a | $b, $a ^ $b, ~$a, $a << $b, $a >> $b];
}
function shifts(uint $a, int $b): vec<uint> { return vec[$a << $b, $a >> $b]; }
function immediate(uint $a): vec<uint> { return vec[$a + 65535u, $a - 2u, $a * 3u, $a % 4u]; }
function steps(uint $a, uint $b): uint { $a += $b; $a++; $a--; return $a; }
function compare(uint $a, uint $b): uint { if ($a < $b) { return 1u; } return 65535u; }
function compare_literal(uint $a): bool { if ($a >= 32768u) { return true; } return false; }
function range(mixed $a): uint { if ($a is 1u..=18446744073709551615u) { return 1u; } return 0u; }
function counted(uint $a): uint { $sum = 0u; for ($i = 0u; $i < $a; $i++) { $sum += $i; } return $sum; }
function keyed(dict<uint, uint> $values, uint $key): uint { $values[$key] = 2u; return $values[$key]; }
function coalesced(dict<uint, uint> $values, uint $key): uint { return $values[$key] ?? 1u; }
function mixed(uint $a, int $b): uint { return $a + $b; }
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for (name, kinds) in [
        (
            "arithmetic",
            vec![
                InstructionKind::IntegerAdd,
                InstructionKind::IntegerSubtract,
                InstructionKind::IntegerMultiply,
                InstructionKind::IntegerModulo,
                InstructionKind::IntegerBitwiseAnd,
                InstructionKind::IntegerBitwiseOr,
                InstructionKind::IntegerBitwiseXor,
                InstructionKind::IntegerBitwiseNot,
                InstructionKind::IntegerShiftLeft,
                InstructionKind::IntegerShiftRight,
            ],
        ),
        (
            "shifts",
            vec![
                InstructionKind::IntegerShiftLeft,
                InstructionKind::IntegerShiftRight,
            ],
        ),
        (
            "immediate",
            vec![
                InstructionKind::IntegerAddImmediate,
                InstructionKind::IntegerSubtractImmediate,
                InstructionKind::IntegerMultiplyImmediate,
                InstructionKind::IntegerModuloImmediate,
            ],
        ),
        (
            "steps",
            vec![
                InstructionKind::IntegerAddAssign,
                InstructionKind::IntegerStep,
            ],
        ),
        (
            "compare",
            vec![
                InstructionKind::UintJumpUnless,
                InstructionKind::ReturnIntegerUnchecked,
            ],
        ),
        (
            "compare_literal",
            vec![InstructionKind::UintJumpUnlessImmediate],
        ),
        ("range", vec![InstructionKind::IntegerRangeJumpUnless]),
        ("counted", vec![InstructionKind::UintCounterLoop]),
        (
            "keyed",
            vec![
                InstructionKind::DictIndexSetIntegerKey,
                InstructionKind::DictIndexGetUintKey,
            ],
        ),
        ("coalesced", vec![InstructionKind::DictIndexCoalesceUintKey]),
        ("mixed", vec![InstructionKind::Add]),
    ] {
        let function = unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name.as_bytes())
            .unwrap();
        for kind in kinds {
            assert!(
                function
                    .chunk
                    .code
                    .iter()
                    .any(|instruction| instruction.kind() == kind),
                "{name} lacks {kind:?}: {:?}",
                function.chunk.code
            );
        }
    }
}

#[test]
fn inlined_unsigned_arithmetic_and_branches_still_fold() {
    let unit = compile(
        r"
#[Whim\Marker\AlwaysInline]
function calculate(uint $a): uint {
    if ($a > 10u) { return (($a + 3u) * 2u - 4u) % 7u; }
    return 65535u;
}
function result(): uint { return calculate(20u); }
function fallback(): uint { return calculate(0u); }
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for (name, expected) in [("result", 0), ("fallback", 65535)] {
        let function = unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name.as_bytes())
            .unwrap();
        assert!(function.chunk.code.iter().any(|instruction| matches!(instruction, Instruction::ReturnIntegerUnchecked { kind: IntegerKind::U64, immediate } if immediate.as_uint() == expected)), "{name}: {:?}", function.chunk.code);
        assert!(!function.chunk.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::IntegerAddImmediate {
                kind: IntegerKind::U64,
                ..
            } | Instruction::UintJumpUnlessImmediate { .. }
                | Instruction::CallNamed { .. }
                | Instruction::CallNamedUnchecked { .. }
        )));
    }
}
