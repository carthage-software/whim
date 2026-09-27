use super::compile;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

#[test]
fn joined_literal_lengths_fold_without_losing_other_inputs_or_uses() {
    let unit = compile(
        r"
        function label(int $value): string {
            return match (($value & 1) == 0) { true => 'even', _ => 'odd' };
        }
        function inlined(int $value): uint { return length!(label($value)); }
        function direct(bool $condition): uint {
            return length!(match ($condition) { true => 'even', _ => 'odd' });
        }
        function bytes(bool $condition): uint {
            return length!(match ($condition) { true => 'é', _ => '' });
        }
        function reused(bool $condition): (string, uint) {
            $value = match ($condition) { true => 'even', _ => 'odd' };
            return ($value, length!($value));
        }
        function coalesced(string|null $value, bool $condition): uint {
            return length!($value ?? match ($condition) { true => 'even', _ => 'odd' });
        }
        function dynamic(bool $condition, string $value): uint {
            return length!(match ($condition) { true => 'even', _ => $value });
        }
        ",
        OptimizationConfiguration::default(),
    );
    for (name, expected) in [
        (b"inlined".as_slice(), 0),
        (b"direct".as_slice(), 0),
        (b"bytes".as_slice(), 0),
        (b"reused".as_slice(), 1),
        (b"coalesced".as_slice(), 1),
        (b"dynamic".as_slice(), 1),
    ] {
        let chunk = &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .unwrap()
            .chunk;
        let count = chunk
            .code
            .iter()
            .filter(|instruction| {
                matches!(
                    instruction,
                    Instruction::Length { .. } | Instruction::StringLength { .. }
                )
            })
            .count();
        assert_eq!(count, expected, "{name:?}: {:?}", chunk.code);
    }
    verify_unit(&unit).unwrap();
}
