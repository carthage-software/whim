use super::compile;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

#[test]
fn known_integer_bits_remove_only_redundant_masks() {
    let unit = compile(
        r"
        function masked(int $value): int {
            $value &= 2147483647;
            return ($value >> 16) & 32767;
        }
        function unsigned(uint $value): uint {
            $value &= 4294967295u;
            return ($value >> 16) & 65535u;
        }
        function signed(int $value): int {
            return ($value >> 16) & 32767;
        }
        function branched(int $value, bool $negative): int {
            $value &= 2147483647;
            if ($negative) { $value = -1; }
            return ($value >> 16) & 32767;
        }
        function overwritten(int $value): int {
            $value &= 2147483647;
            $value = ~$value;
            return ($value >> 16) & 32767;
        }
        function invalid_shift(int $value): int {
            $value &= 2147483647;
            return ($value >> 64) & 32767;
        }
        ",
        OptimizationConfiguration::default(),
    );
    for (name, expected) in [
        (b"masked".as_slice(), 1),
        (b"unsigned".as_slice(), 1),
        (b"signed".as_slice(), 1),
        (b"branched".as_slice(), 2),
        (b"overwritten".as_slice(), 2),
        (b"invalid_shift".as_slice(), 2),
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
            .filter(|instruction| matches!(instruction, Instruction::BitwiseAnd { .. }))
            .count();
        assert_eq!(count, expected, "{name:?}: {:?}", chunk.code);
    }
    verify_unit(&unit).unwrap();
}

#[test]
fn nested_masks_only_discard_bits_with_no_other_use() {
    let unit = compile(
        r"
        function folded(int $value, int $other): int {
            $masked = ($value | 1) & 65535;
            return ($masked ^ $other) & 255;
        }
        function unsigned(uint $value, uint $other): uint {
            $masked = ($value | 1u) & 65535u;
            return ($masked ^ $other) & 255u;
        }
        function shared(int $value, int $other): (int, int) {
            $masked = ($value | 1) & 65535;
            return ($masked, ($masked ^ $other) & 255);
        }
        function shifted(int $value, int $other): int {
            $masked = ($value | 1) & 65535;
            return (($masked >> 8) ^ $other) & 255;
        }
        function narrower(int $value, int $other): int {
            $masked = ($value | 1) & 15;
            return ($masked ^ $other) & 255;
        }
        function tagged(int $value, int $other): int {
            $masked = $value & 65535;
            return ($masked ^ $other) & 255;
        }
        function joined(int $value, int $other, bool $branch): int {
            $masked = ($value | 1) & 65535;
            if ($branch) { $other = 1; }
            return ($masked ^ $other) & 255;
        }
        function overwritten(int $value, int $other): int {
            $masked = ($value | 1) & 65535;
            $masked ^= $other;
            return $masked & 255;
        }
        function repeated(int $value): int {
            $masked = ($value | 1) & 65535;
            return $masked & 65535;
        }
        function parameter(int $value, int $other): int {
            $value = ($value | 1) & 65535;
            return ($value ^ $other) & 255;
        }
        ",
        OptimizationConfiguration::default(),
    );
    for (name, expected) in [
        (b"folded".as_slice(), 1),
        (b"unsigned".as_slice(), 1),
        (b"shared".as_slice(), 2),
        (b"shifted".as_slice(), 2),
        (b"narrower".as_slice(), 2),
        (b"tagged".as_slice(), 2),
        (b"joined".as_slice(), 2),
        (b"overwritten".as_slice(), 1),
        (b"repeated".as_slice(), 1),
        (b"parameter".as_slice(), 2),
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
            .filter(|instruction| matches!(instruction, Instruction::BitwiseAnd { .. }))
            .count();
        assert_eq!(count, expected, "{name:?}: {:?}", chunk.code);
    }
    verify_unit(&unit).unwrap();
}
