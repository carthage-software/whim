use std::path::Path;

use crate::engine::Engine;
use crate::engine::EngineConfiguration;

#[test]
fn power_facts_preserve_negative_exponent_result_types_and_consumers() {
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
fn stub_constants_do_not_prove_their_placeholder_type() {
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

fn run_both_modes(source: &str, path: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new(path));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
