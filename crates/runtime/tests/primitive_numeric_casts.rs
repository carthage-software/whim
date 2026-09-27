use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

const CASTS: &str = r"
use Whim\Float;
use Whim\Math;
use Whim\Marker\NeverInline;
use Whim\Unwind\TypeError;
#[NeverInline]
function signed(mixed $value): int { return $value as int; }
#[NeverInline]
function unsigned(mixed $value): uint { return $value as uint; }
#[NeverInline]
function floating(mixed $value): float { return $value as float; }
#[NeverInline]
function maybe_signed(mixed $value): null|int { return $value ?as int; }
#[NeverInline]
function maybe_unsigned(mixed $value): null|uint { return $value ?as uint; }
#[NeverInline]
function maybe_floating(mixed $value): null|float { return $value ?as float; }
";

fn run(body: &str) {
    let source = format!("{CASTS}\n{body}");
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(&source, Path::new("/primitive-numeric-casts.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn numeric_casts_keep_integer_boundaries_and_nullable_failures() {
    run(r"
foreach (vec[42, 42u, 42.0] as $value) {
    assert!(signed($value) == 42);
    assert!(unsigned($value) == 42u);
    assert!(floating($value) == 42.0);
}
assert!(signed(9_223_372_036_854_775_807u) == Math\INT_MAX);
assert!(signed(-9_223_372_036_854_775_808.0) == Math\INT_MIN);
assert!(unsigned(Math\INT_MAX) == 9_223_372_036_854_775_807u);
assert!(unsigned(18_446_744_073_709_549_568.0) == 18_446_744_073_709_549_568u);
assert!(unsigned(-0.0) == 0u);
assert!(signed(-0.0) == 0);
assert!(floating(18_446_744_073_709_551_615u) == 18_446_744_073_709_551_616.0);
assert!(floating(Math\INT_MIN) == -9_223_372_036_854_775_808.0);
foreach (vec[9_223_372_036_854_775_808u, 18_446_744_073_709_551_615u,
    9_223_372_036_854_775_808.0, -9_223_372_036_854_777_856.0] as $value) {
    assert!(maybe_signed($value) == null);
}
foreach (vec[-1, Math\INT_MIN, -1.0, 18_446_744_073_709_551_616.0] as $value) {
    assert!(maybe_unsigned($value) == null);
}
foreach (vec[1.5, -1.5, Math\NAN, Math\INF, -Math\INF] as $value) {
    assert!(maybe_signed($value) == null);
    assert!(maybe_unsigned($value) == null);
}
class Input {}
foreach (vec['42', true, false, null, vec[42], dict['n' => 42], new Input()] as $value) {
    assert!(maybe_signed($value) == null);
    assert!(maybe_unsigned($value) == null);
    assert!(maybe_floating($value) == null);
}
");
}

#[test]
fn primitive_casts_remove_nested_tags_and_preserve_float_bits() {
    run(r"
newtype Signed = int;
newtype SignedOuter = Signed;
newtype Unsigned = uint;
newtype UnsignedOuter = Unsigned;
newtype Floating = float;
newtype FloatingOuter = Floating;
foreach (vec[Signed(42), SignedOuter(Signed(42)),
    Unsigned(42u), UnsignedOuter(Unsigned(42u)),
    Floating(42.0), FloatingOuter(Floating(42.0))] as $value) {
    $signed = signed($value);
    $unsigned = unsigned($value);
    $float = floating($value);
    assert!($signed == 42);
    assert!($unsigned == 42u);
    assert!($float == 42.0);
    foreach (vec[$signed, $unsigned, $float,
        maybe_signed($value), maybe_unsigned($value), maybe_floating($value)] as $plain) {
        assert!(!($plain is Signed|SignedOuter|Unsigned|UnsignedOuter|Floating|FloatingOuter));
    }
}
foreach (vec[0, Math\INT_MIN, 0x7ff8_0000_0000_1234,
    0x7ff0_0000_0000_1234, 0x7ff0_0000_0000_0000] as $bits) {
    $value = Float\from_bits($bits);
    assert!(Float\to_bits(floating($value)) == $bits);
    assert!(Float\to_bits(maybe_floating($value) as float) == $bits);
    $tagged = FloatingOuter(Floating($value));
    $plain = floating($tagged);
    assert!(Float\to_bits($plain) == $bits);
    assert!(!($plain is Floating|FloatingOuter));
}
");
}

#[test]
fn failed_numeric_casts_keep_error_messages_and_resume_catches() {
    run(r"
function rejects(fn(): mixed $operation, string $message): void {
    $caught = false;
    try { $operation(); } catch (TypeError $error) {
        assert!($error->getMessage() == $message);
        $caught = true;
    }
    assert!($caught);
}
#[NeverInline]
function recover(mixed $value): int {
    try { return $value as int; } catch (TypeError $_error) { return 73; }
}
rejects(fn(): mixed => signed(18_446_744_073_709_551_615u), 'expected int, uint given');
rejects(fn(): mixed => unsigned(-1), 'expected uint, int given');
rejects(fn(): mixed => signed(1.5), 'expected int, float given');
rejects(fn(): mixed => floating('42'), 'expected float, string given');
assert!(recover('bad') == 73);
assert!(recover(12u) == 12);
assert!(recover(Math\INF) == 73);
assert!(signed(13u) == 13);
");
}

#[test]
fn named_and_compound_numeric_casts_keep_their_checks() {
    run(r"
type SignedAlias = int;
type Bounded<T: uint> = T & 1u..=5u;
newtype Identifier = uint;
#[NeverInline]
function generic<T>(mixed $value): T { return $value as T; }
#[NeverInline]
function alias(mixed $value): int { return $value as SignedAlias; }
#[NeverInline]
function range(mixed $value): mixed { return $value ?as 1u..=5u; }
#[NeverInline]
function bounded(mixed $value): mixed { return $value ?as Bounded<uint>; }
#[NeverInline]
function ordered(mixed $value): mixed { return $value as (uint|int); }
#[NeverInline]
function identifier(mixed $value): Identifier { return $value as Identifier; }
assert!(generic::<uint>(3) == 3u);
assert!(generic::<int>(3u) == 3);
assert!(alias(3u) == 3);
assert!(range(3.0) == 3u);
assert!(range(6.0) == null);
assert!(bounded(3.0) == 3u);
assert!(bounded(6.0) == null);
assert!(ordered(3.0) == 3u);
assert!(ordered(3) == 3);
assert!(identifier(3u) is Identifier);
class Loads { public static int $count = 0; }
Whim\_Private\register_symbol_autoloader(fn(int $kind, string $name): void {
    if ($name == 'MissingCast') { Loads::$count++; }
});
assert!(generic::<MissingCast|int>(3.0) == 3);
assert!(Loads::$count > 0);
");
}
