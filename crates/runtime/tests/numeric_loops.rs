use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn counted_loops_preserve_copies_types_and_faults() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function copy(dict<int, int> $source, dict<int, int> $target, int $count): dict<int, int> {
    for ($index = $count - 1; $index >= 0; $index--) {
        $target[$index] = $source[$index];
    }
    return $target;
}
#[NeverInline]
function increment(int $value): int {
    for ($index = 0; $index < 8; $index++) {
        $value++;
    }
    return $value;
}
#[NeverInline]
function unsigned_increment(uint $value): uint {
    for ($index = 0; $index < 8; $index++) {
        $value++;
    }
    return $value;
}
#[NeverInline]
function changing_limit(int $limit): int {
    $sum = 0;
    for ($index = 0; $index < $limit; $index = $index + 1) {
        $limit--;
        $sum += ($index * $index + $index) % 17;
    }
    return $sum;
}
#[NeverInline]
function nested_copy(dict<int, int> $source, int $count): int {
    $target = dict[0 => 0];
    $sum = 0;
    for ($round = 0; $round < 3; $round++) {
        $sum += $target[0];
        for ($index = $count - 1; $index >= 0; $index--) {
            $target[$index] = $source[$index];
        }
        $sum += $target[$count - 1];
    }
    return $sum;
}
#[NeverInline]
function accumulate(dict<int, int> $source, dict<int, int> $target, int $count): (dict<int, int>, int) {
    $index = $count - 1;
    try {
        for (; $index >= 0; $index--) {
            $target[$index] += $source[$index];
        }
    } catch (Whim\Unwind\ArithmeticError $_) {}
    return ($target, $index);
}
$original = dict[0 => 10, 1 => 20, 2 => 30, 3 => 40];
assert!(copy($original, dict[], 0) == dict[]);
assert!(copy($original, dict[], 1) == dict[0 => 10]);
$copied = copy($original, $original, 4);
$copied[0] = 99;
assert!($original[0] == 10);
assert!($copied == dict[0 => 99, 1 => 20, 2 => 30, 3 => 40]);
assert!(copy(dict[3 => 40, 2 => 30, 1 => 20, 0 => 10], dict[], 4)
    == dict[3 => 40, 2 => 30, 1 => 20, 0 => 10]);
$caught = false;
try {
    copy(dict[0 => 10, 2 => 30, 3 => 40], dict[], 4);
} catch (Whim\Unwind\OutOfBoundsError $_) {
    $caught = true;
}
assert!($caught);
assert!(increment(-12) == -4);
assert!(unsigned_increment(12u) == 20u);
assert!(changing_limit(10) == 23);
assert!(nested_copy($original, 4) == 140);
assert!(accumulate($original, $original, 4)
    == (dict[0 => 20, 1 => 40, 2 => 60, 3 => 80], -1));
assert!($original == dict[0 => 10, 1 => 20, 2 => 30, 3 => 40]);
assert!(accumulate($original, dict[0 => 10, 1 => 9223372036854775797, 2 => 30, 3 => 40], 4)
    == (dict[0 => 10, 1 => 9223372036854775797, 2 => 60, 3 => 80], 1));
$caught = false;
try {
    increment(9223372036854775804);
} catch (Whim\Unwind\OverflowError $_) {
    $caught = true;
}
assert!($caught);
";
    run_both_modes(source);
}

#[test]
fn integer_bursts_preserve_boundaries_and_partial_updates() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function less(int $start, int $limit, int $sum): (int, int, int, bool) {
    $index = $start;
    $temporary = 0;
    $caught = false;
    try {
        for (; $index < $limit; $index++) {
            $sum++;
            $temporary += 2;
        }
    } catch (Whim\Unwind\OverflowError $_) { $caught = true; }
    return ($index, $sum, $temporary, $caught);
}
#[NeverInline]
function less_equal(int $start, int $limit, int $sum): (int, int, int, bool) {
    $index = $start;
    $temporary = 0;
    $caught = false;
    try {
        for (; $index <= $limit; $index++) {
            $sum++;
            $temporary += 2;
        }
    } catch (Whim\Unwind\OverflowError $_) { $caught = true; }
    return ($index, $sum, $temporary, $caught);
}
assert!(less(0, 0, 0) == (0, 0, 0, false));
assert!(less(0, 1, 0) == (1, 1, 2, false));
assert!(less(-9223372036854775808, -9223372036854775806, 0)
    == (-9223372036854775806, 2, 4, false));
assert!(less(9223372036854775805, 9223372036854775807, 0)
    == (9223372036854775807, 2, 4, false));
assert!(less_equal(9223372036854775805, 9223372036854775807, 0)
    == (9223372036854775807, 3, 6, true));
assert!(less_equal(9223372036854775807, 9223372036854775807, 0)
    == (9223372036854775807, 1, 2, true));
assert!(less(4, 10, 9223372036854775806)
    == (5, 9223372036854775807, 2, true));
assert!(less(0, 65537, 0) == (65537, 65537, 131074, false));
assert!(less_equal(0, 65536, 0) == (65537, 65537, 131074, false));
";
    run_both_modes(source);
}

#[test]
fn numeric_regions_keep_destructor_boundaries() {
    let source = r"
use Whim\Marker\NeverInline;
final class Released {
    public static vec<int> $events = vec[];
    public function __construct(private int $index) {}
    public function __destruct(): void {
        self::$events[] = $this->index;
        if ($this->index == 1) { throw new Whim\Unwind\Exception('released'); }
    }
}
#[NeverInline]
function released_values(): dict<int, Released> {
    return dict[
        0 => new Released(0), 1 => new Released(1),
        2 => new Released(2), 3 => new Released(3),
    ];
}
#[NeverInline]
function release_by_copy(dict<int, int> $source, int $count, int $limit): (int, bool, vec<int>) {
    $target = released_values();
    $index = $count - 1;
    try {
        for (; $index >= $limit; $index--) {
            $target[$index] = $source[$index];
        }
    } catch (Whim\Unwind\Exception $_) {}
    return ($index, $target[0] is Released, Released::$events);
}
assert!(release_by_copy(dict[0 => 0, 1 => 10, 2 => 20, 3 => 30], 4, 0)
    == (1, true, vec[3, 2, 1]));
assert!(Released::$events == vec[3, 2, 1, 0]);
";
    run_both_modes(source);
}

#[test]
fn joined_string_lengths_preserve_live_strings_and_alternative_inputs() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function parity_length(int $value): int {
    return length!(match ($value & 1) { 0 => 'even', _ => 'odd' });
}
#[NeverInline]
function byte_length(bool $choose): int {
    return length!(match ($choose) { true => 'é', false => '' });
}
#[NeverInline]
function retained(int $value): (string, int) {
    $label = match ($value & 1) { 0 => 'even', _ => 'odd' };
    return ($label, length!($label));
}
#[NeverInline]
function fallback_length(string|null $fallback, bool $choose): int {
    return length!($fallback ?? match ($choose) { true => 'even', false => 'odd' });
}
for ($index = 0; $index < 20; $index++) {
    assert!(parity_length($index) == 4 - ($index & 1));
    assert!(retained($index) == match ($index & 1) { 0 => ('even', 4), _ => ('odd', 3) });
}
assert!(byte_length(true) == 2);
assert!(byte_length(false) == 0);
assert!(fallback_length('unchanged', true) == 9);
assert!(fallback_length('unchanged', false) == 9);
assert!(fallback_length(null, true) == 4);
assert!(fallback_length(null, false) == 3);
";
    run_both_modes(source);
}

fn run_both_modes(source: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/numeric-loops.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn concatenation_bursts_preserve_shared_sources_and_counters() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function concatenate(string $value, string $extra, int $count): (string, int, bool) {
    $failed = false;
    try {
        while ($count-- > 0) {
            $value .= $extra;
        }
    } catch (Whim\Unwind\UnderflowError $_) { $failed = true; }
    return ($value, $count, $failed);
}
#[NeverInline]
function join(string $left, string $right): string { return $left . $right; }
$shared = 'abcdefgh';
assert!(concatenate($shared, $shared, 3) == ($shared . $shared . $shared . $shared, -1, false));
assert!($shared == 'abcdefgh');
assert!(concatenate($shared, '', 3) == ($shared, -1, false));
assert!(concatenate($shared, 'x', 0) == ($shared, -1, false));
assert!(concatenate($shared, 'x', 1) == ('abcdefghx', -1, false));
assert!(concatenate($shared, 'x', -9223372036854775808) == ($shared, -9223372036854775808, true));
($result, $count, $failed) = concatenate($shared, 'x', 65537);
assert!(length!($result) == 65545 && $result[8] == 'x' && $result[65544] == 'x');
assert!($count == -1 && !$failed && $shared == 'abcdefgh');
$part = 'abcdefghijklmnopqrstuvwxyz';
$rope = join($part, $part);
($result, $count, $failed) = concatenate($shared, $rope, 65537);
assert!(length!($result) == 8 + 52 * 65537 && $result[8] == 'a' && $result[59] == 'z');
assert!($rope == $part . $part && $count == -1 && !$failed);
",
    );
}

#[test]
fn integer_masks_preserve_sign_branches_and_shift_faults() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
newtype SignedMaskInput = int;
newtype UnsignedMaskInput = uint;
#[NeverInline]
function signed_identity(int $value): int { return $value & -1; }
#[NeverInline]
function unsigned_identity(uint $value): uint { return $value & 18446744073709551615u; }
#[NeverInline]
function masked(int $value): int {
    $value &= 2147483647;
    return ($value >> 16) & 32767;
}
#[NeverInline]
function signed(int $value): int { return ($value >> 16) & 32767; }
#[NeverInline]
function branched(int $value, bool $negative): int {
    $value &= 2147483647;
    if ($negative) { $value = -1; }
    return ($value >> 16) & 32767;
}
#[NeverInline]
function unsigned(uint $value): uint {
    $value &= 4294967295u;
    return ($value >> 16) & 65535u;
}
#[NeverInline]
function invalid_shift(int $value): int {
    $value &= 2147483647;
    return ($value >> 64) & 32767;
}
assert!(masked(-1) == 32767);
assert!(masked(-2147483648) == 0);
assert!(masked(65536) == 1);
assert!(signed(-1) == 32767);
assert!(signed(-2147483648) == 0);
assert!(branched(0, false) == 0);
assert!(branched(0, true) == 32767);
assert!(unsigned(18446744073709551615u) == 65535u);
assert!(unsigned(65536u) == 1u);
assert!(signed_identity(SignedMaskInput(1)) == 1);
assert!(!(signed_identity(SignedMaskInput(1)) is SignedMaskInput));
assert!(unsigned_identity(UnsignedMaskInput(1u)) == 1u);
assert!(!(unsigned_identity(UnsignedMaskInput(1u)) is UnsignedMaskInput));
$caught = false;
try { invalid_shift(-1); } catch (Whim\Unwind\ArithmeticError $_) { $caught = true; }
assert!($caught);
",
    );
}

#[test]
fn numeric_regions_preserve_input_newtype_tags() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
newtype ShiftCount = int;
newtype FloatInput = float;
#[NeverInline]
function shifted(int $shift): (int, bool) {
    $value = 0;
    for ($index = 0; $index < 3; $index++) {
        $value = ($index << $shift) + ($index >> $shift);
    }
    return ($value, $shift is ShiftCount);
}
#[NeverInline]
function summed(float $input): (float, bool) {
    $sum = 0.0;
    for ($index = 0; $index < 3; $index++) {
        $sum += $input;
    }
    return ($sum, $input is FloatInput);
}
assert!(shifted(ShiftCount(1)) == (5, true));
assert!(summed(FloatInput(3.5)) == (10.5, true));
",
    );
}

#[test]
fn reused_string_lengths_preserve_empty_loops_and_changed_inputs() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function stable(string $value, int $count): int {
    $sum = 0;
    for ($index = 0; $index < $count; $index++) { $sum += length!($value); }
    return $sum;
}
#[NeverInline]
function changed(string $value, int $count): int {
    $sum = 0;
    for ($index = 0; $index < $count; $index++) {
        $sum += length!($value);
        $value .= 'x';
    }
    return $sum;
}
foreach (vec[(0, 0), (1, 2), (10, 65)] as ($count, $expected)) {
    assert!(stable('', $count) == 0);
    assert!(stable('é', $count) == $count * 2);
    assert!(changed('é', $count) == $expected);
}
",
    );
}

#[test]
fn nested_integer_masks_keep_values_and_faults() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
newtype MaskInput = int;
#[NeverInline]
function folded(int $value, int $other): int {
    $masked = ($value | 1) & 65535;
    return ($masked ^ $other) & 255;
}
#[NeverInline]
function unsigned(uint $value, uint $other): uint {
    $masked = ($value | 1u) & 65535u;
    return ($masked ^ $other) & 255u;
}
#[NeverInline]
function shared(int $value, int $other): (int, int) {
    $masked = ($value | 1) & 65535;
    return ($masked, ($masked ^ $other) & 255);
}
#[NeverInline]
function caught(int $value, int $shift): int {
    $masked = ($value | 1) & 65535;
    try {
        $other = 1 << $shift;
        return ($masked ^ $other) & 255;
    } catch (Whim\Unwind\ArithmeticError $_) { return $masked; }
}
#[NeverInline]
function repeated(int $value): int {
    $masked = ($value | 1) & 65535;
    return $masked & 65535;
}
#[NeverInline]
function shift_fault(int $value, int $shift): int {
    $masked = ($value | 1) & 65535;
    return ($masked ^ (1 << $shift)) & 255;
}
assert!(folded(-256, 2) == 3);
assert!(folded(MaskInput(-1), 2) == 253);
assert!(!(folded(MaskInput(-1), 2) is MaskInput));
assert!(unsigned(18446744073709551615u, 2u) == 253u);
assert!(shared(-1, 2) == (65535, 253));
assert!(caught(-1, 64) == 65535);
assert!(caught(-1, 1) == 253);
assert!(repeated(-1) == 65535);
$failed = false;
try { shift_fault(-1, 64); } catch (Whim\Unwind\ArithmeticError $_) { $failed = true; }
assert!($failed);
",
    );
}
