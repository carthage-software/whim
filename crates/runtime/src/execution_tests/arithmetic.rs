use super::run_both_modes;

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
