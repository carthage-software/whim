use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn inherited_static_returns_check_receiver_type_arguments() {
    let source = r"
class Base {
    #[Whim\Marker\NeverInline]
    public function checked(mixed $value): static { return $value; }
}
final class Child<T> extends Base {}
$integer = new Child::<int>();
$string = new Child::<string>();
for ($round = 0; $round < 4; $round++) {
    assert!($integer->checked($integer) == $integer);
    $caught = false;
    try { $string->checked($integer); }
    catch (Whim\Unwind\TypeError $_) { $caught = true; }
    assert!($caught);
    assert!($string->checked($string) == $string);
    $caught = false;
    try { $integer->checked($string); }
    catch (Whim\Unwind\TypeError $_) { $caught = true; }
    assert!($caught);
}
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/inherited-static-return.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn return_guards_preserve_specializations_and_mutable_shapes() {
    let source = r"
use Whim\Marker\NeverInline;
use Whim\Unwind\TypeError;
final class Box<T> {
    public function __construct(public T $value) {}
}

#[NeverInline]
function checked<T>(mixed $value): T { return $value; }
class Base {
    #[NeverInline]
    public static function checked(mixed $value): static { return $value; }
}
final class Child extends Base {}
final class Mutable {
    public function __construct(public mixed $value) {}
}
type Shape = #{ value: int };
#[NeverInline]
function shaped(mixed $value): Shape { return $value; }
$integer = new Box::<int>(42);
$string = new Box::<string>('kept');
$base = new Base();
$child = new Child();
$mutable = new Mutable(1);
for ($round = 0; $round < 8; $round++) {
    assert!(checked::<Box<int>>($integer) == $integer);
    assert!(checked::<Box<string>>($string) == $string);
    assert!(checked::<int>(42) == 42);
    assert!(checked::<string>('kept') == 'kept');
    assert!(Base::checked($base) == $base);
    assert!(Child::checked($child) == $child);
    $mutable->value = 1;
    assert!(shaped($mutable) == $mutable);
    $mutable->value = 'changed';
    $failures = 0;
    try { checked::<Box<int>>($string); } catch (TypeError $_) { $failures++; }
    try { checked::<Box<string>>($integer); } catch (TypeError $_) { $failures++; }
    try { checked::<int>('wrong'); } catch (TypeError $_) { $failures++; }
    try { checked::<string>(42); } catch (TypeError $_) { $failures++; }
    try { Child::checked($base); } catch (TypeError $_) { $failures++; }
    try { shaped($mutable); } catch (TypeError $_) { $failures++; }
    assert!($failures == 6);
}
final class Empty<out T> {}
final class Defaulted<out T, out U = Missing> {}
final class Bounded<out T: Missing|int> {}
class Loads { public static int $count = 0; }
#[NeverInline]
function unresolved(mixed $value): Empty<Missing> { return $value; }
#[NeverInline]
function defaulted(mixed $value): Defaulted<int> { return $value; }
#[NeverInline]
function bounded(mixed $value): Bounded<int> { return $value; }
Whim\_Private\register_symbol_autoloader(
    fn(int $kind, string $name): void {
        if ($name == 'Missing') { Loads::$count++; }
    },
);
$empty = new Empty::<never>();
unresolved($empty);
for ($round = 0; $round < 4; $round++) {
    $previous = Loads::$count;
    assert!(unresolved($empty) == $empty);
    assert!(Loads::$count == $previous + 1);
}
$defaulted = new Defaulted::<int, never>();
$bounded = new Bounded::<int>();
defaulted($defaulted);
bounded($bounded);
for ($round = 0; $round < 4; $round++) {
    $previous = Loads::$count;
    assert!(defaulted($defaulted) == $defaulted);
    assert!(Loads::$count == $previous + 1);
    $previous = Loads::$count;
    assert!(bounded($bounded) == $bounded);
    assert!(Loads::$count > $previous);
}
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/return-guards.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn return_guards_preserve_late_alias_autoloads() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let declarations = r"
final class Holder<out T> {}
#[Whim\Marker\NeverInline]
function late_alias(mixed $value): LateAlias { return $value; }
#[Whim\Marker\NeverInline]
function alias_argument(mixed $value): Holder<LateAlias<int>> { return $value; }
";
        let result = engine.run_source(declarations, Path::new("/late-alias-functions.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let source = r"
type LateAlias<T = Missing> = int;
class Loads { public static int $count = 0; }
Whim\_Private\register_symbol_autoloader(
    fn(int $kind, string $name): void {
        if ($name == 'Missing') { Loads::$count++; }
    },
);
$holder = new Holder::<int>();
late_alias(42);
alias_argument($holder);
for ($round = 0; $round < 4; $round++) {
    $previous = Loads::$count;
    assert!(late_alias(42) == 42);
    assert!(Loads::$count > $previous);
    $previous = Loads::$count;
    assert!(alias_argument($holder) == $holder);
    assert!(Loads::$count > $previous);
}
";
        let result = engine.run_source(source, Path::new("/late-alias-definition.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
