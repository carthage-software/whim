use std::path::Path;

use whim_value::function::FuncId;
use whim_value::object::ClassId;
use whim_value::object::TypeEnvironmentId;

use super::run_both_modes;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::symbols::CachedParameterGuard;

#[test]
fn nominal_argument_guards_accept_subtypes_and_reject_wrong_values_after_warmup() {
    let source = r"
use Whim\Marker\NeverInline;
interface Contract {}
interface ChildContract extends Contract {}
abstract class Base implements ChildContract {
    public function __construct(public int $id) {}
}
final class First extends Base {}
final class Second extends Base {}
final class Generic<T> extends Base {}
final class Other {}
final class Calls { public static int $count = 0; }
type Alias = Base;
#[NeverInline]
function accept(Alias $value): int { Calls::$count++; return $value->id; }
final class Receiver {
    #[NeverInline]
    public function accept(Contract $value): int {
        Calls::$count++;
        return ($value as Base)->id;
    }
}
#[NeverInline]
function accept_slow(vec<int> $prefix, Base $value): int {
    Calls::$count++;
    return $prefix[0] + $value->id;
}
#[NeverInline]
function named(mixed $value): int { return accept($value); }
#[NeverInline]
function method(Receiver $receiver, mixed $value): int { return $receiver->accept($value); }
#[NeverInline]
function slow(mixed $prefix, mixed $value): int { return accept_slow($prefix, $value); }
$receiver = new Receiver();
$values = vec[new First(1), new Second(2), new Generic::<int>(3), new Generic::<string>(4)];
for ($round = 0; $round < 3; $round++) {
    foreach ($values as $value) {
        assert!(named($value) == $value->id);
        assert!(method($receiver, $value) == $value->id);
        assert!(slow(vec[$round], $value) == $round + $value->id);
    }
}
assert!(Calls::$count == 36);
foreach (vec[new Other(), 42, null] as $wrong) {
    $caught = 0;
    try { discard!(named($wrong)); } catch (Whim\Unwind\TypeError $_) { $caught++; }
    try { discard!(method($receiver, $wrong)); } catch (Whim\Unwind\TypeError $_) { $caught++; }
    try { discard!(slow(vec[1], $wrong)); } catch (Whim\Unwind\TypeError $_) { $caught++; }
    assert!($caught == 3);
    assert!(Calls::$count == 36);
}
assert!(named($values[0]) == 1);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/nominal-argument-guards.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");

        let caller = engine
            .tables
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == b"named")
            .unwrap();
        let target = FuncId(
            u32::try_from(
                engine
                    .tables
                    .functions
                    .iter()
                    .position(|function| function.name.as_bytes() == b"accept")
                    .unwrap(),
            )
            .unwrap(),
        );
        let base = ClassId(
            u32::try_from(
                engine
                    .tables
                    .classes
                    .iter()
                    .position(|class| class.name.as_bytes() == b"Base")
                    .unwrap(),
            )
            .unwrap(),
        );
        // SAFETY: this test owns the idle engine and does not mutate its cache.
        let guards = unsafe { &*caller.cache.argument_guards() };
        let entry = guards
            .iter()
            .find_map(|ways| ways.get(target, TypeEnvironmentId::default()))
            .unwrap();
        assert!(
            matches!(entry.guards.as_ref(), [CachedParameterGuard::NominalClass(class)] if *class == base)
        );
    }
}

#[test]
fn nominal_argument_guards_preserve_newtypes_and_generic_checks() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
class Base {}
final class First extends Base {}
final class Second extends Base {}
final class Holder<T> { public function __construct(public T $value) {} }
final class Calls { public static int $count = 0; }
newtype Wrapped = Base;
type Bounded<T: Base> = T;
#[NeverInline]
function nominal(Base $value): void { Calls::$count++; }
#[NeverInline]
function tagged(Wrapped $value): void { Calls::$count++; }
#[NeverInline]
function specialized(Holder<First> $value): void { Calls::$count++; }
#[NeverInline]
function bounded(Bounded<First> $value): void { Calls::$count++; }
#[NeverInline]
function generic<T: Base>(T $value): void { Calls::$count++; }
#[NeverInline]
function dispatch(int $kind, mixed $value): void {
    if ($kind == 0) { nominal($value); }
    else if ($kind == 1) { tagged($value); }
    else if ($kind == 2) { specialized($value); }
    else if ($kind == 3) { bounded($value); }
    else { generic::<First>($value); }
}
$first = new First();
$second = new Second();
$wrapped = Wrapped($first);
$integer = new Holder::<First>($first);
$other = new Holder::<Second>($second);
for ($round = 0; $round < 3; $round++) {
    dispatch(0, $first);
    dispatch(0, new Base());
    dispatch(0, $wrapped);
    dispatch(0, $second);
    dispatch(1, $wrapped);
    dispatch(2, $integer);
    dispatch(3, $first);
    dispatch(4, $first);
    $before = Calls::$count;
    $caught = 0;
    try { dispatch(1, $first); } catch (Whim\Unwind\TypeError $_) { $caught++; }
    try { dispatch(2, $other); } catch (Whim\Unwind\TypeError $_) { $caught++; }
    try { dispatch(3, $second); } catch (Whim\Unwind\TypeError $_) { $caught++; }
    try { dispatch(4, $second); } catch (Whim\Unwind\TypeError $_) { $caught++; }
    assert!($caught == 4);
    assert!(Calls::$count == $before);
}
",
        "/nominal-argument-types.whim",
    );
}

#[test]
fn nominal_argument_guards_preserve_unresolved_names_and_late_declarations() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let source = r"
use Whim\Marker\NeverInline;
final class Calls { public static int $count = 0; public static int $loads = 0; }
#[NeverInline]
function accept(Late $value): void { Calls::$count++; }
#[NeverInline]
function relay(mixed $value): void { accept($value); }
Whim\_Private\register_symbol_autoloader(fn(int $kind, string $name): void {
    if ($name == 'Late') { Calls::$loads++; }
});
for ($round = 0; $round < 3; $round++) {
    $before = Calls::$loads;
    $caught = false;
    try { relay(42); } catch (Whim\Unwind\TypeError $_) { $caught = true; }
    assert!($caught);
    assert!(Calls::$loads > $before);
    assert!(Calls::$count == 0);
}
";
        let result = engine.run_source(source, Path::new("/unresolved-argument-guard.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let source = r"
interface Late {}
final class First implements Late {}
final class Second implements Late {}
$before = Calls::$loads;
foreach (vec[new First(), new Second(), new First()] as $value) { relay($value); }
assert!(Calls::$count == 3);
assert!(Calls::$loads == $before);
$caught = false;
try { relay(42); } catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
assert!(Calls::$count == 3);
assert!(Calls::$loads == $before);
";
        let result = engine.run_source(source, Path::new("/resolved-argument-guard.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
