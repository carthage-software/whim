use super::run_both_modes;

#[test]
fn mixed_integer_counter_loops_preserve_bounds_and_overflow_state() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function visit(int $value): int { return $value; }
#[NeverInline]
function ascend(int $start, uint $limit): (int, int, bool) {
    $visits = 0;
    $index = $start;
    try {
        for (; $index < $limit; $index++) { $visits += visit(1); }
    } catch (Whim\Unwind\OverflowError $_) {
        return ($index, $visits, true);
    }
    return ($index, $visits, false);
}
#[NeverInline]
function unsigned_ascend(uint $start, int $limit): (uint, int) {
    $visits = 0;
    for ($index = $start; $index < $limit; $index++) { $visits += visit(1); }
    return ($index, $visits);
}
assert!(ascend(-2, 2u) == (2, 4, false));
assert!(ascend(0, 0u) == (0, 0, false));
assert!(ascend(9223372036854775805, 9223372036854775807u) == (9223372036854775807, 2, false));
assert!(ascend(9223372036854775806, 9223372036854775808u) == (9223372036854775807, 2, true));
assert!(ascend(9223372036854775807, 18446744073709551615u) == (9223372036854775807, 1, true));
assert!(unsigned_ascend(0u, -1) == (0u, 0));
assert!(unsigned_ascend(0u, 2) == (2u, 2));
assert!(unsigned_ascend(9223372036854775808u, 9223372036854775807) == (9223372036854775808u, 0));
newtype Signed = int;
newtype Unsigned = uint;
$start = Signed(-2);
$limit = Unsigned(2u);
assert!(ascend($start, $limit) == (2, 4, false));
assert!($start is Signed);
assert!($limit is Unsigned);
",
        "/mixed-integer-counter-loops.whim",
    );
}

#[test]
fn negated_comparisons_preserve_edges_nan_and_live_results() {
    for operator in ["<", "<=", ">", ">="] {
        let source = format!(
            r"
use Whim\Marker\NeverInline;
#[NeverInline]
function integers(int $left, int $right): bool {{
    if (!($left {operator} $right)) {{ return true; }}
    return false;
}}
#[NeverInline]
function literal(int $left): bool {{
    if (!($left {operator} 10)) {{ return true; }}
    return false;
}}
#[NeverInline]
function floats(float $left, float $right): bool {{
    if (!($left {operator} $right)) {{ return true; }}
    return false;
}}
#[NeverInline]
function mixed_values(mixed $left, mixed $right): bool {{
    if (!($left {operator} $right)) {{ return true; }}
    return false;
}}
#[NeverInline]
function live_result(int $left, int $right): (bool, int) {{
    $comparison = $left {operator} $right;
    if (!$comparison) {{ return ($comparison, 1); }}
    return ($comparison, 0);
}}
$values = vec[-9_223_372_036_854_775_808, -1, 0, 9, 10, 11, 9_223_372_036_854_775_807];
foreach ($values as $left) {{
    assert!(literal($left) == !($left {operator} 10));
    foreach ($values as $right) {{
        $expected = !($left {operator} $right);
        assert!(integers($left, $right) == $expected);
        assert!(mixed_values($left, $right) == $expected);
        assert!(live_result($left, $right) == (!$expected, match ($expected) {{ true => 1, false => 0 }}));
    }}
}}
$infinity = 1e308 * 10.0;
$nan = $infinity - $infinity;
foreach (vec[$nan, -$infinity, -1.0, -0.0, 0.0, 1.0, $infinity] as $left) {{
    foreach (vec[$nan, -$infinity, -0.0, 0.0, $infinity] as $right) {{
        assert!(floats($left, $right) == !($left {operator} $right));
        assert!(mixed_values($left, $right) == !($left {operator} $right));
    }}
}}
$caught = false;
try {{ mixed_values(vec[], 1); }}
catch (Whim\Unwind\IncompatibleOperandsError $error) {{ $caught = true; }}
assert!($caught);
#[NeverInline]
function backward(int $limit): int {{
    $value = 0;
    do {{ $value++; }} while (!($value >= $limit));
    return $value;
}}
assert!(backward(10) == 10);
"
        );
        run_both_modes(&source, "/negated-comparisons.whim");
    }
}

#[test]
fn negative_powers_preserve_result_types_and_consumers() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function power_kind(int $base, int $exponent): bool {
    $value = $base ** $exponent;
    return ($value is int) == ($exponent >= 0)
        && ($value is float) == ($exponent < 0);
}
#[NeverInline]
function power_value(int $base, int $exponent): int|float { return $base ** $exponent; }
#[NeverInline]
function negative_power(int $base, -10..=-1 $exponent): float { return $base ** $exponent; }
#[NeverInline]
function invalid_power(int $base, int $exponent): int { return $base ** $exponent; }
#[NeverInline]
function consume_power(int $base, int $exponent): float { return ($base ** $exponent) + 0.5; }
#[NeverInline]
function mixed_arithmetic(int $integer, float $float): bool {
    return ($integer + $float) is float && ($float - $integer) is float
        && ($integer * $float) is float && ($integer / $integer) is float
        && ($integer % $integer) is int && ($integer << 1) is int;
}
$literal = 2 ** -1;
assert!($literal == 0.5);
assert!($literal is float);
assert!(!($literal is int));
assert!((2 ** 3) is int);
assert!((2.0 ** -1) is float);
assert!((2 ** -1.0) is float);
assert!(power_kind(2, -1));
assert!(power_kind(2, 0));
assert!(power_kind(2, 3));
assert!(power_value(2, -1) == 0.5);
assert!(power_value(2, 3) == 8);
assert!(negative_power(2, -1) == 0.5);
assert!(consume_power(2, -1) == 1.0);
assert!(consume_power(2, 3) == 8.5);
assert!(mixed_arithmetic(2, 0.5));
$caught = false;
try { invalid_power(2, -1); } catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
";
    run_both_modes(source, "/power-proofs.whim");
}

#[test]
fn stub_constants_use_native_values() {
    let source = r"
namespace Whim\_Private;
use Whim\Marker\Stub;
#[Stub]
const OS = OS;
assert!(OS is string);
assert!(!(OS is null));
assert!(OS != '');
";
    run_both_modes(source, "/stub-constant-proofs.whim");
}

#[test]
fn inlined_compound_constants_keep_signed_unsigned_bits_and_tags() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
newtype Signed = int;
newtype Unsigned = uint;
function signed_value(): int { return -32768; }
function unsigned_value(): uint { return 65535u; }
#[NeverInline]
function signed(int $value): int { $value += signed_value(); return $value; }
#[NeverInline]
function unsigned(uint $value): uint { $value += unsigned_value(); return $value; }
assert!(signed(32770) == 2);
assert!(signed(-9223372036854743040) == -9223372036854775808);
assert!(unsigned(0u) == 65535u);
assert!(unsigned(18446744073709486080u) == 18446744073709551615u);
$signed = Signed(32770);
$unsigned = Unsigned(0u);
$first = signed($signed);
$second = unsigned($unsigned);
assert!($first == 2 && !($first is Signed));
assert!($second == 65535u && !($second is Unsigned));
assert!($signed is Signed && $unsigned is Unsigned);
function length_value(string $value): uint { return length!($value); }
#[NeverInline]
function total(int $count): uint {
    $sum = 0u;
    for ($index = 0; $index < $count; $index++) { $sum += length_value('hallo'); }
    return $sum;
}
assert!(total(0) == 0u && total(13) == 65u);
",
        "/inlined-integer-add-assign-constants.whim",
    );
}

#[test]
fn compound_overflow_keeps_values_tags_catches_and_error_messages() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
use Whim\Unwind\OverflowError;
use Whim\Unwind\UnderflowError;
newtype Signed = int;
newtype Unsigned = uint;
function positive(): int { return 5; }
function negative(): int { return -5; }
function unsigned_value(): uint { return 5u; }
#[NeverInline]
function signed(int $value): int { $value += positive(); return $value; }
#[NeverInline]
function lower(int $value): int { $value += negative(); return $value; }
#[NeverInline]
function unsigned(uint $value): uint { $value += unsigned_value(); return $value; }
$signed = Signed(9223372036854775807);
$unsigned = Unsigned(18446744073709551615u);
$caught = 0;
try { signed($signed); } catch (OverflowError $error) {
    assert!($error->getMessage() == 'the integer result overflows the 64-bit range');
    $caught++;
}
try { unsigned($unsigned); } catch (OverflowError $error) {
    assert!($error->getMessage() == 'the integer result overflows the 64-bit range');
    $caught++;
}
try { lower(-9223372036854775808); } catch (UnderflowError $error) {
    assert!($error->getMessage() == 'the integer result underflows the 64-bit range');
    $caught++;
}
assert!($caught == 3 && $signed is Signed && $unsigned is Unsigned);
#[NeverInline]
function caught_signed(int $value): (int, int, bool) {
    $increment = 77;
    try { $increment = 5; $value += $increment; }
    catch (OverflowError $_) { return ($value, $increment, $value is Signed); }
    return ($value, $increment, false);
}
#[NeverInline]
function caught_unsigned(uint $value): (uint, uint, bool) {
    $increment = 77u;
    try { $increment = 5u; $value += $increment; }
    catch (OverflowError $_) { return ($value, $increment, $value is Unsigned); }
    return ($value, $increment, false);
}
assert!(caught_signed($signed) == ($signed, 5, true));
assert!(caught_unsigned($unsigned) == ($unsigned, 5u, true));
assert!(caught_signed(3) == (8, 5, false));
assert!(caught_unsigned(3u) == (8u, 5u, false));
",
        "/integer-add-assign-overflow.whim",
    );
}

#[test]
fn unsigned_immediate_loops_keep_partial_overflow_and_reference_destinations() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
use Whim\Unwind\OverflowError;
#[NeverInline]
function accumulate(uint $sum, int $count): (uint, int, bool) {
    $index = 0;
    try {
        for (; $index < $count; $index++) { $sum += 65535u; }
    } catch (OverflowError $_) { return ($sum, $index, true); }
    return ($sum, $index, false);
}
#[NeverInline]
function replace(uint $source, vec<string> $values, int $count): (mixed, vec<string>) {
    $result = $values;
    for ($index = 0; $index < $count; $index++) { $result = $source + 65535u; }
    return ($result, $values);
}
assert!(accumulate(0u, 3) == (196605u, 3, false));
assert!(accumulate(18446744073709420545u, 3) == (18446744073709551615u, 2, true));
assert!(accumulate(18446744073709551615u, 1) == (18446744073709551615u, 0, true));
$values = vec['a retained string longer than an inline value'];
assert!(replace(2u, $values, 0) == ($values, $values));
assert!(replace(2u, $values, 3) == (65537u, $values));
assert!(replace(18446744073709486080u, $values, 1) == (18446744073709551615u, $values));
",
        "/unsigned-add-immediate-loops.whim",
    );
}

#[test]
fn fused_literal_consumers_keep_values_live_on_branch_and_catch_edges() {
    run_both_modes(
        include_str!("../../../../tests/_fixtures/literal-consumer-liveness.whim"),
        "/literal-consumer-liveness.whim",
    );
}
