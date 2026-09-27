use std::path::Path;

use whim_value::object::ClassId;

use crate::engine::Engine;
use crate::engine::EngineConfiguration;

fn cached_class(engine: &Engine, name: &[u8]) -> Option<ClassId> {
    let function = engine
        .tables
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == name)
        .unwrap();
    // SAFETY: this test owns the idle engine and only reads its cache.
    let checks = unsafe { &*function.cache.is_checks() };
    checks.iter().find_map(|check| check.final_class)
}

#[test]
fn final_class_checks_cache_the_target_after_a_miss() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(
            r"
use Whim\Marker\NeverInline;
final class Target { public mixed $value; }
final class First {}
final class Second {}
final class Third {}
final class Fourth {}
final class Fifth {}
newtype Wrapped = Target;
#[NeverInline]
function check(mixed $value): bool { return $value is Target; }
assert!(!check(new First()));
",
            Path::new("/final-class-first-miss.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let target = cached_class(&engine, b"check").unwrap();
        assert_eq!(
            engine.tables.classes[target.0 as usize].name.as_bytes(),
            b"Target"
        );
        let result = engine.run_source(
            r"
$target = new Target();
for ($round = 0; $round < 3; $round++) {
    assert!(check($target));
    assert!(check(Wrapped($target)));
    foreach (vec[new First(), new Second(), new Third(), new Fourth(), new Fifth(),
        null, false, 1, 1u, 1.5, 'target', vec[], dict[], (1, 2)] as $value) {
        assert!(!check($value));
    }
    $target->value = $round;
    assert!(check($target));
}
",
            Path::new("/final-class-hits-and-misses.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        assert_eq!(cached_class(&engine, b"check"), Some(target));
    }
}

#[test]
fn final_class_checks_keep_inheritance_generics_aliases_and_tags() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(
            r"
use Whim\Marker\NeverInline;
interface Contract {}
class Base {}
final class Leaf extends Base implements Contract {}
final class Other {}
final class Box<T> {}
type Alias = Leaf;
type Bounded<T: Leaf> = T;
newtype Wrapped = Leaf;
#[NeverInline]
function base(mixed $value): bool { return $value is Base; }
#[NeverInline]
function contract(mixed $value): bool { return $value is Contract; }
#[NeverInline]
function raw(mixed $value): bool { return $value is Box; }
#[NeverInline]
function specialized(mixed $value): bool { return $value is Box<int>; }
#[NeverInline]
function alias(mixed $value): bool { return $value is Alias; }
#[NeverInline]
function bounded<T>(mixed $value): bool { return $value is Bounded<T>; }
#[NeverInline]
function tagged(mixed $value): bool { return $value is Wrapped; }
#[NeverInline]
function parameter<T>(mixed $value): bool { return $value is T; }
#[NeverInline]
function union_class(mixed $value): bool { return $value is Leaf|Other; }
#[NeverInline]
function negated(mixed $value): bool { return $value is !Leaf; }
$leaf = new Leaf();
$other = new Other();
$integer = new Box::<int>();
$string = new Box::<string>();
for ($round = 0; $round < 3; $round++) {
    assert!(base($leaf));
    assert!(!base($other));
    assert!(contract($leaf));
    assert!(!contract($other));
    assert!(raw($integer));
    assert!(raw($string));
    assert!(specialized($integer));
    assert!(!specialized($string));
    assert!(alias($leaf));
    assert!(!alias($other));
    assert!(bounded::<Leaf>($leaf));
    assert!(!bounded::<Leaf>($other));
    $caught = false;
    try { discard!(bounded::<string>($leaf)); }
    catch (Whim\Unwind\TypeError $_error) { $caught = true; }
    assert!($caught);
    assert!(tagged(Wrapped($leaf)));
    assert!(!tagged($leaf));
    assert!(parameter::<Leaf>($leaf));
    assert!(!parameter::<Other>($leaf));
    assert!(union_class($leaf));
    assert!(union_class($other));
    assert!(!union_class($integer));
    assert!(!negated($leaf));
    assert!(negated($other));
}
",
            Path::new("/final-class-fallbacks.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        for name in [
            b"base".as_slice(),
            b"contract",
            b"raw",
            b"specialized",
            b"alias",
            b"bounded",
            b"tagged",
            b"parameter",
            b"union_class",
            b"negated",
        ] {
            assert_eq!(cached_class(&engine, name), None, "{name:?}");
        }
    }
}

#[test]
fn final_class_checks_preserve_unresolved_names_and_late_declarations() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(
            r"
use Whim\Marker\NeverInline;
final class Loads { public static int $count = 0; }
final class Existing {}
#[NeverInline]
function late(mixed $value): bool { return $value is Late; }
Whim\_Private\register_symbol_autoloader(fn(int $kind, string $name): void {
    if ($name == 'Late') { Loads::$count++; }
});
for ($round = 0; $round < 3; $round++) {
    $before = Loads::$count;
    assert!(!late(new Existing()));
    assert!(Loads::$count > $before);
}
",
            Path::new("/final-class-unresolved.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        assert_eq!(cached_class(&engine, b"late"), None);
        let result = engine.run_source(
            r"
final class Late {}
$before = Loads::$count;
assert!(!late(new Existing()));
assert!(late(new Late()));
assert!(!late(null));
assert!(Loads::$count == $before);
",
            Path::new("/final-class-late-declaration.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let target = cached_class(&engine, b"late").unwrap();
        assert_eq!(
            engine.tables.classes[target.0 as usize].name.as_bytes(),
            b"Late"
        );
    }
}

#[test]
fn final_class_checks_ignore_classes_from_failed_declarations() {
    for optimize in [false, true] {
        for replacement in [
            r"
final class Replacement {}
final class Future {}
assert!(!future(new Replacement()));
assert!(future(new Future()));
assert!(!future(new Existing()));
",
            r"
final class Replacement {}
final class Cell { public mixed $value = 1; }
type Future = #{ value: int };
assert!(!future(new Replacement()));
$cell = new Cell();
assert!(future($cell));
$cell->value = 'changed';
assert!(!future($cell));
",
        ] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                ..EngineConfiguration::default()
            });
            let result = engine.run_source(
                r"
use Whim\Marker\NeverInline;
final class Existing {}
#[NeverInline]
function future(mixed $value): bool { return $value is Future; }
#[NeverInline]
function stage(mixed $valid, mixed $other): string {
    assert!(future($valid));
    assert!(!future($other));
    return 'bad';
}
",
                Path::new("/final-class-before-rollback.whim"),
            );
            assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
            let class_count = engine.tables.classes.len();
            let result = engine.run_source(
                r"
final class Future {}
final class Broken {
    public static int $value = stage(new Future(), new Existing());
}
",
                Path::new("/final-class-failed-declaration.whim"),
            );
            assert_ne!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
            assert_eq!(engine.tables.classes.len(), class_count);
            assert_eq!(cached_class(&engine, b"future"), None);
            let result =
                engine.run_source(replacement, Path::new("/final-class-after-rollback.whim"));
            assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
            assert_eq!(
                engine.tables.classes[class_count].name.as_bytes(),
                b"Replacement"
            );
        }
    }
}
