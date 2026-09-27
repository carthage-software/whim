use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn optional_parameter_checks_preserve_defaults_and_supplied_values() {
    let source = include_str!("../../../tests/_fixtures/default-parameter-checks.whim");
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/default-parameter-checks.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn supplied_defaults_keep_generic_argument_autoloads() {
    let source = r"
final class Holder<out T> {}
final class Defaulted<out T, out U = Missing> {}
final class Bounded<out T: Missing|int> {}
type DefaultAlias<T = Missing> = int;
type BoundAlias<T: Missing|int> = T;
class Loads { public static int $count = 0; }
#[Whim\Marker\NeverInline]
function supplied(Holder<Missing> $value = null): void {}
#[Whim\Marker\NeverInline]
function defaulted(Defaulted<int> $value = null): void {}
#[Whim\Marker\NeverInline]
function bounded(Bounded<int> $value = null): void {}
#[Whim\Marker\NeverInline]
function default_alias(DefaultAlias $value = 1): void {}
#[Whim\Marker\NeverInline]
function bound_alias(BoundAlias<int> $value = 1): void {}
Whim\_Private\register_symbol_autoloader(
    fn(int $kind, string $name): void {
        if ($name == 'Missing') { Loads::$count++; }
    },
);
$holder = new Holder::<never>();
$before = Loads::$count;
supplied($holder);
assert!(Loads::$count == $before + 9);
$before = Loads::$count;
supplied($holder);
assert!(Loads::$count == $before + 2);
$defaulted = new Defaulted::<int, never>();
$bounded = new Bounded::<int>();
for ($round = 0; $round < 2; $round++) {
    $expected = 1;
    if ($round == 0) { $expected = 9; }
    $before = Loads::$count;
    defaulted($defaulted);
    assert!(Loads::$count == $before + $expected);
    $before = Loads::$count;
    bounded($bounded);
    if ($round > 0) { assert!(Loads::$count == $before + 7); }
    $before = Loads::$count;
    default_alias(1);
    assert!(Loads::$count == $before + 1);
    $before = Loads::$count;
    bound_alias(1);
    assert!(Loads::$count == $before + 14);
}
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/default-argument-autoloads.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn scalar_alias_defaults_keep_bounds_and_invalid_values_checked() {
    let source = r"
type NonZero<T: uint> = T & !0u;
#[Whim\Marker\NeverInline]
function limit(null|NonZero<uint> $value = null): mixed { return $value; }
#[Whim\Marker\NeverInline]
function invalid(NonZero<uint> $value = 0u): mixed { return $value; }
#[Whim\Marker\NeverInline]
function wrong_bound<T>(NonZero<T> $value = 1): mixed { return $value; }
function rejects(fn(): mixed $operation): void {
    $caught = false;
    try { $operation(); } catch (Whim\Unwind\TypeError $_error) { $caught = true; }
    assert!($caught);
}
assert!(limit() == null);
assert!(limit(null) == null);
assert!(limit(2u) == 2u);
assert!(invalid(3u) == 3u);
rejects(fn(): mixed => limit(0u));
rejects(fn(): mixed => limit(2));
rejects(fn(): mixed => invalid());
rejects(fn(): mixed => wrong_bound::<int>());
rejects(fn(): mixed => wrong_bound::<int>(1));
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/scalar-default-checks.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
