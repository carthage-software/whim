use super::run_both_modes;

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
