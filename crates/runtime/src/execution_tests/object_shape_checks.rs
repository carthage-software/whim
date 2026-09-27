use super::run_both_modes;
use std::path::Path;

use crate::engine::Engine;
use crate::engine::EngineConfiguration;

#[test]
fn object_shape_checks_recheck_initialization_and_mutable_values() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
final class Box { public mixed $value; }
type IntShape = #{ value: int };
#[NeverInline]
function initialized(mixed $value): bool { return $value is #{ value: _ }; }
#[NeverInline]
function integer(mixed $value): bool { return $value is #{ value: int }; }
#[NeverInline]
function union_shape(mixed $value): bool { return $value is null|#{ value: int }; }
#[NeverInline]
function intersection_shape(mixed $value): bool { return $value is Box & #{ value: int }; }
#[NeverInline]
function negated_shape(mixed $value): bool { return $value is !#{ value: int }; }
#[NeverInline]
function alias_shape(mixed $value): bool { return $value is IntShape; }
#[NeverInline]
function generic_shape<T>(mixed $value): bool { return $value is T; }
#[NeverInline]
function pattern(mixed $value): mixed {
    return match ($value) { #{ value: $property } => $property, _ => 'missing' };
}
#[NeverInline]
function nominal_pattern(mixed $value): mixed {
    return match ($value) { Box #{ value: $property } => $property, _ => 'missing' };
}
$ready = new Box();
$ready->value = 1;
$pending = new Box();
for ($round = 0; $round < 3; $round++) {
    assert!(initialized($ready));
    assert!(!initialized($pending));
    assert!(pattern($ready) == $ready->value);
    assert!(pattern($pending) == 'missing');
    assert!(nominal_pattern($ready) == $ready->value);
    assert!(nominal_pattern($pending) == 'missing');
    $ready->value = 1;
    assert!(integer($ready));
    assert!(union_shape($ready));
    assert!(intersection_shape($ready));
    assert!(alias_shape($ready));
    assert!(generic_shape::<IntShape>($ready));
    $ready->value = 'changed';
    assert!(!integer($ready));
    assert!(!union_shape($ready));
    assert!(!intersection_shape($ready));
    assert!(!alias_shape($ready));
    assert!(!generic_shape::<IntShape>($ready));
    assert!(negated_shape($ready));
    $ready->value = 2;
    assert!(!negated_shape($ready));
}
$pending->value = null;
assert!(initialized($pending));
assert!(pattern($pending) == null);
assert!(!integer($pending));
assert!(union_shape(null));
",
        "/object-shape-current-values.whim",
    );
}

#[test]
fn wildcard_object_shapes_preserve_public_layout_and_closed_counts() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
class Parent { public mixed $value; }
final class Child extends Parent {
    private mixed $hidden;
    protected mixed $protected;
    public static mixed $shared = null;
}

final class Extra extends Parent { public mixed $extra; }
final class Reordered { public mixed $extra; public mixed $value = 9; }
final class Hidden {
    private mixed $value = 1;
    #[NeverInline]
    public function check(): bool { return $this is #{ value: _, ... }; }
}
final class StaticOnly { public static mixed $value = 1; }
final readonly class Immutable {
    public mixed $value;
    public function __construct(bool $initialize) { if ($initialize) { $this->value = 7; } }
}
newtype Tagged = Parent;
#[NeverInline]
function closed(mixed $value): bool { return $value is #{ value: _ }; }
#[NeverInline]
function open(mixed $value): bool { return $value is #{ value: _, ... }; }
#[NeverInline]
function empty_shape(mixed $value): bool { return $value is #{}; }
#[NeverInline]
function any_object(mixed $value): bool { return $value is #{ ... }; }
#[NeverInline]
function pair(mixed $value): bool { return $value is #{ value: _, extra: _ }; }
$ready = new Child();
$ready->value = null;
$pending = new Child();
$extra = new Extra();
$extra->value = 1;
$reordered = new Reordered();
$hidden = new Hidden();
$static = new StaticOnly();
$immutable = new Immutable(true);
$immutable_pending = new Immutable(false);
for ($round = 0; $round < 3; $round++) {
    assert!(closed($ready));
    assert!(open($ready));
    assert!(closed(Tagged($ready)));
    assert!(!closed($pending));
    assert!(!open($pending));
    assert!(!closed($extra));
    assert!(open($extra));
    assert!(!pair($extra));
    assert!(!closed($reordered));
    assert!(open($reordered));
    assert!(!pair($reordered));
    assert!(!closed($hidden));
    assert!(!open($hidden));
    assert!(!$hidden->check());
    assert!(!closed($static));
    assert!(!open($static));
    assert!(empty_shape($hidden));
    assert!(empty_shape($static));
    assert!(!empty_shape($pending));
    assert!(any_object($pending));
    assert!(any_object($hidden));
    assert!(any_object($static));
    assert!(any_object(Tagged($ready)));
    assert!(closed($immutable));
    assert!(!closed($immutable_pending));
}
$extra->extra = false;
$reordered->extra = null;
assert!(pair($extra));
assert!(pair($reordered));
assert!(!closed($extra));
foreach (vec[null, false, 1, '', vec[], dict[], (1, 2)] as $value) {
    assert!(!closed($value));
    assert!(!open($value));
    assert!(!empty_shape($value));
    assert!(!any_object($value));
}
",
        "/wildcard-object-shape-layouts.whim",
    );
}

#[test]
fn object_shape_and_nominal_argument_caches_ignore_failed_declarations() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let source = r"
use Whim\Marker\NeverInline;
final class Calls { public static int $count = 0; }
#[NeverInline]
function accept(Future $value): void { Calls::$count++; }
#[NeverInline]
function relay(mixed $value): void { accept($value); }
#[NeverInline]
function shape(mixed $value): bool { return $value is #{ value: _ }; }
#[NeverInline]
function stage(mixed $value): string {
    assert!(shape($value));
    relay($value);
    return 'bad';
}

";
        let result = engine.run_source(source, Path::new("/cache-rollback-functions.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let class_count = engine.tables.classes.len();
        let source = r"
final class Future { public int $value = 1; }
final class Broken { public static int $value = stage(new Future()); }
";
        let result = engine.run_source(source, Path::new("/cache-rollback-failure.whim"));
        assert_ne!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        assert_eq!(engine.tables.classes.len(), class_count);
        let source = r"
final class Replacement { public int $other = 1; }
final class Future { public int $value = 2; }
assert!(Calls::$count == 1);
$replacement = new Replacement();
assert!(!shape($replacement));
$caught = false;
try { relay($replacement); } catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
assert!(Calls::$count == 1);
$valid = new Future();
assert!(shape($valid));
relay($valid);
assert!(Calls::$count == 2);
assert!(!shape($replacement));
";
        let result = engine.run_source(source, Path::new("/cache-rollback-reused-classes.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        assert_eq!(
            engine.tables.classes[class_count].name.as_bytes(),
            b"Replacement"
        );
    }
}

#[test]
fn nominal_is_caches_ignore_failed_declarations_and_changed_symbol_kinds() {
    for optimize in [false, true] {
        for replacement in [
            r"
final class Replacement {}
final class Future {}
assert!(!nominal(new Replacement()));
assert!(nominal(new Future()));
",
            r"
final class Filler {}
final class Cell { public mixed $value = 1; }
type Future = #{ value: int };
$cell = new Cell();
assert!(nominal($cell));
$cell->value = 'changed';
assert!(!nominal($cell));
",
        ] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                ..EngineConfiguration::default()
            });
            let result = engine.run_source(
                r"
use Whim\Marker\NeverInline;
#[NeverInline]
function nominal(mixed $value): bool { return $value is Future; }
#[NeverInline]
function stage(mixed $value): string { assert!(nominal($value)); return 'bad'; }
",
                Path::new("/nominal-is-before-rollback.whim"),
            );
            assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
            let class_count = engine.tables.classes.len();
            let result = engine.run_source(
                r"
final class Future {}
final class Broken { public static int $value = stage(new Future()); }
",
                Path::new("/nominal-is-failed-declaration.whim"),
            );
            assert_ne!(result.exit_code(), 0);
            assert_eq!(engine.tables.classes.len(), class_count);
            let result =
                engine.run_source(replacement, Path::new("/nominal-is-after-rollback.whim"));
            assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        }
    }
}
