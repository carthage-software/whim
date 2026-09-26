use super::compile;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

#[test]
fn computed_scalar_captures_keep_types_without_erasing_named_types() {
    let unit = compile(
        r"
        newtype Token = int;
        function integer(int $value): fn(int): int {
            $bias = $value % 31;
            return fn(int $input): int => $input + $bias;
        }
        function unsigned(uint $value): fn(uint): uint {
            $bias = $value % 31u;
            return fn(uint $input): uint => $input + $bias;
        }
        function floating(float $value): fn(float): float {
            $scale = $value * 0.5;
            return fn(float $input): float => $input * $scale;
        }
        function label(int $value): fn(): int {
            $text = 'value=' . $value;
            return fn(): int => length!($text);
        }
        function named(Token $value): fn(): Token { return fn(): Token => $value; }
        function uncertain(bool $condition): fn(): int {
            $value = match ($condition) { true => 1, _ => 'wrong' };
            return fn(): int => $value;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let closures = unit
        .functions
        .iter()
        .filter(|function| !function.capture_types.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(closures.len(), 6);
    assert!(matches!(
        closures[0].capture_types.as_slice(),
        [Some(TypeDescriptor::Int)]
    ));
    assert!(matches!(
        closures[1].capture_types.as_slice(),
        [Some(TypeDescriptor::Uint)]
    ));
    assert!(matches!(
        closures[2].capture_types.as_slice(),
        [Some(TypeDescriptor::Float)]
    ));
    assert!(matches!(
        closures[3].capture_types.as_slice(),
        [Some(TypeDescriptor::String)]
    ));
    assert!(
        matches!(&closures[4].capture_types[0], Some(TypeDescriptor::Named { name, .. }) if name.as_bytes() == b"Token")
    );
    assert!(matches!(closures[5].capture_types.as_slice(), [None]));
    for (function, kind) in [
        (closures[0], IntegerKind::I64),
        (closures[1], IntegerKind::U64),
    ] {
        assert!(function.chunk.code.iter().any(|instruction| matches!(instruction, Instruction::Add { kind: Some(actual), .. } if *actual == kind)));
    }
    assert!(
        closures[2]
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::FloatMultiply { .. }))
    );
    assert!(
        closures[3]
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::StringLength { .. }))
    );
    assert!(
        closures[5]
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Return { .. }))
    );
    verify_unit(&unit).unwrap();
}
