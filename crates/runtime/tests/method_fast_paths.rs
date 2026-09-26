use std::env;
use std::fs;
use std::path::Path;
use std::process;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

const FAST_PATH_SOURCE: &str = r"
use Whim\Marker\NeverInline;
final class GetterTarget<T> {
    public function __construct(private T $value) {}
    #[NeverInline]
    public function get(): T { return $this->value; }
    #[NeverInline]
    public function same(): GetterTarget<T> { return $this; }
    #[NeverInline]
    public function identity<U>(U $value): U { return $value; }
}
#[NeverInline]
function direct(GetterTarget<int> $target): int { return $target->get(); }
#[NeverInline]
function generic_direct(GetterTarget<int> $target, string $value): string {
    return $target->identity::<string>($value);
}
final class GetterCaller {
    public function __construct(private GetterTarget<int> $target) {}
    #[NeverInline]
    public function indirect(): int { return $this->target->get(); }
    #[NeverInline]
    public function generic_indirect(string $value): string {
        return $this->target->identity::<string>($value);
    }
}
$target = new GetterTarget::<int>(42);
$caller = new GetterCaller($target);
for ($i = 0; $i < 4; $i++) {
    assert!(direct($target) == 42);
    assert!($caller->indirect() == 42);
    assert!(generic_direct($target, 'kept') == 'kept');
    assert!($caller->generic_indirect('kept') == 'kept');
}
";

#[test]
fn cached_direct_method_frames_preserve_inherited_types_and_defaults() {
    let source = r"
use Whim\Marker\NeverInline;
class GenericParent<T> {
    public function __construct(private T $value) {}
    #[NeverInline]
    public function read(int $extra = 1): (T, int) {
        assert!($extra >= 0);
        return ($this->value, $extra + 1);
    }
}
final class Child extends GenericParent<string> {}
#[NeverInline]
function read_default(Child $value): (string, int) { return $value->read(); }
#[NeverInline]
function read_extra(Child $value, int $extra): (string, int) {
    return $value->read($extra);
}
$value = new Child('kept');
for ($index = 0; $index < 4; $index++) {
    assert!(read_extra($value, $index) == ('kept', $index + 1));
    assert!(read_default($value) == ('kept', 2));
    $caught = false;
    try { read_extra($value, -1); }
    catch (Whim\Unwind\AssertionError $_) { $caught = true; }
    assert!($caught);
    assert!(read_default($value) == ('kept', 2));
}
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/direct-method-frames.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn specialized_method_fast_paths_do_not_push_frames() {
    let mut engine = Engine::new(EngineConfiguration {
        call_depth_limit: 2,
        ..EngineConfiguration::default()
    });
    let result = engine.run_source(FAST_PATH_SOURCE, Path::new("/method-fast-path-depth.whim"));
    assert_eq!(result.exit_code(), 0, "{result:?}");
}

#[test]
fn lazy_methods_are_finalized_before_caching_fast_paths() {
    let directory = env::temp_dir().join(format!("whim-method-fast-path-{}", process::id()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("methods.whim");
    fs::write(&path, FAST_PATH_SOURCE).unwrap();
    let mut engine = Engine::new(EngineConfiguration {
        call_depth_limit: 3,
        ..EngineConfiguration::default()
    });
    let result = engine.run_source("require!('methods.whim');", &directory.join("main.whim"));
    fs::remove_dir_all(directory).unwrap();
    assert_eq!(result.exit_code(), 0, "{result:?}");
}

#[test]
fn method_fast_paths_preserve_borrowed_and_consumed_values() {
    let source = r"
use Whim\Marker\NeverInline;
final class Token {
    public static int $drops = 0;
    public function __construct(public string $name) {}
    public function __destruct(): void { self::$drops++; }
}
final class Target<T> {
    public function __construct(private T $value) {}
    #[NeverInline]
    public function get(): T { return $this->value; }
    #[NeverInline]
    public function same(): Target<T> { return $this; }
    #[NeverInline]
    public function identity<U>(U $value): U { return $value; }
}
final class Caller<T> {
    public function __construct(private Target<T> $target) {}
    #[NeverInline]
    public function get(): T { return $this->target->get(); }
    #[NeverInline]
    public function same(): Target<T> { return $this->target->same(); }
    #[NeverInline]
    public function identity<U>(U $value): U {
        return $this->target->identity::<U>($value);
    }
}
#[NeverInline]
function exercise(): void {
    $token = new Token('retained token');
    $target = new Target::<Token>($token);
    $caller = new Caller::<Token>($target);
    $values = vec['retained collection'];
    for ($round = 0; $round < 4; $round++) {
        assert!($target->get() == $token);
        assert!($caller->get() == $token);
        assert!($target->same() == $target);
        assert!($caller->same() == $target);
        $copy = $target->identity::<vec<string>>($values);
        $copy[] = 'direct';
        assert!(length!($values) == 1);
        $copy = $caller->identity::<vec<string>>($values);
        $copy[] = 'indirect';
        assert!(length!($values) == 1);
        $token = $target->identity::<Token>($token);
        $token = $caller->identity::<Token>($token);
        assert!($token->name == 'retained token');
        assert!(Token::$drops == 0);
    }
}
exercise();
assert!(Token::$drops == 1);
";
    run_both_modes(source, "/method-fast-path-values.whim");
}

#[test]
fn cached_getters_fall_back_for_uninitialized_properties() {
    let source = r"
use Whim\Marker\NeverInline;
final class Target {
    public int $value;
    #[NeverInline]
    public function get(): int { return $this->value; }
}
#[NeverInline]
function direct(Target $target): int { return $target->get(); }
final class Caller {
    public function __construct(public Target $target) {}
    #[NeverInline]
    public function indirect(): int { return $this->target->get(); }
}
$initialized = new Target();
$initialized->value = 42;
$uninitialized = new Target();
$caller = new Caller($initialized);
for ($round = 0; $round < 4; $round++) {
    assert!(direct($initialized) == 42);
    $caller->target = $initialized;
    assert!($caller->indirect() == 42);
    $caught = 0;
    try { direct($uninitialized); }
    catch (Whim\Unwind\UninitializedPropertyError $error) {
        assert!($error->getTrace()[0]->function == 'Target::get');
        $caught++;
    }
    $caller->target = $uninitialized;
    try { $caller->indirect(); }
    catch (Whim\Unwind\UninitializedPropertyError $error) {
        assert!($error->getTrace()[0]->function == 'Target::get');
        $caught++;
    }
    assert!($caught == 2);
}
";
    run_both_modes(source, "/method-fast-path-uninitialized.whim");
}

#[test]
fn specialized_inherited_getters_preserve_class_environments() {
    let source = r"
use Whim\Marker\NeverInline;
class Base<T> {
    public T $value;
    #[NeverInline]
    public function get(): T { return $this->value; }
}
final class IntTarget extends Base<int> {}
final class StringTarget extends Base<string> {}
#[NeverInline]
function integer(IntTarget $target): int { return $target->get(); }
#[NeverInline]
function string_value(StringTarget $target): string { return $target->get(); }
$integer = new IntTarget();
$integer->value = 42;
$string = new StringTarget();
$string->value = 'retained string';
for ($round = 0; $round < 4; $round++) {
    assert!(integer($integer) == 42);
    assert!(string_value($string) == 'retained string');
}
";
    run_both_modes(source, "/method-fast-path-inheritance.whim");
}

#[test]
fn plain_callable_frames_preserve_checks_defaults_and_lexical_context() {
    let source = r"
use Whim\Marker\NeverInline;
use Whim\Unwind\{ArgumentCountError, TypeError};
newtype CaptureId = int;
#[NeverInline]
function invoke(fn $callback, mixed $value): mixed { return $callback($value); }
#[NeverInline]
function constrained<T>(): fn {
    return fn(int $value): int where T: int => $value;
}
class Scope {
    private static int $value = 40;
    #[NeverInline]
    public static function callback(): fn {
        return fn(int $extra): int => self::$value + $extra;
    }
}
$defaulted = fn(int $value, int $offset = 2): int => $value + $offset;
$captured = 40;
$capture = fn(int $value): int => $value + $captured;
$identifier = CaptureId(40);
$tagged = fn(int $_): CaptureId => $identifier;
$numbers = vec[1, 2];
$array = fn(int $value): vec<int> { $numbers[] = $value; return $numbers; };
for ($round = 0; $round < 4; $round++) {
    assert!(invoke($defaulted, 40) == 42);
    assert!(invoke($capture, 2) == 42);
    assert!(invoke($tagged, 0) is CaptureId);
    assert!(invoke($array, 3) == vec[1, 2, 3] && $numbers == vec[1, 2]);
    assert!(invoke(Scope::callback(), 2) == 42);
    assert!(invoke(constrained::<int>(), 42) == 42);
    $failures = 0;
    try { invoke($defaulted, 'wrong'); } catch (TypeError $_) { $failures++; }
    try { invoke(fn(): int => 42, 1); } catch (ArgumentCountError $_) { $failures++; }
    try { invoke(fn(int $a, int $b): int => $a + $b, 1); }
    catch (ArgumentCountError $_) { $failures++; }
    try { invoke(constrained::<string>(), 42); } catch (TypeError $_) { $failures++; }
    assert!($failures == 4);
}
";
    run_both_modes(source, "/plain-callable-fast-path.whim");
}

#[test]
fn polymorphic_methods_preserve_checks_after_cache_saturation() {
    let source = r"
use Whim\Marker\NeverInline;
class Target<T> {
    public function identity(T $value): T { return $value; }
}
#[NeverInline]
function invoke(object $target, mixed $value): mixed { return $target->identity($value); }
$targets = vec[
    (new Target::<int>(), 42), (new Target::<string>(), 'retained'),
    (new Target::<bool>(), true), (new Target::<float>(), 2.5),
    (new Target::<uint>(), 42u), (new Target::<vec<int>>(), vec[1, 2]),
];
for ($round = 0; $round < 3; $round++) {
    foreach ($targets as ($target, $value)) {
        assert!(invoke($target, $value) == $value);
        $failed = false;
        try { invoke($target, null); }
        catch (Whim\Unwind\TypeError $_) { $failed = true; }
        assert!($failed);
        assert!(invoke($target, $value) == $value);
    }
}
";
    run_both_modes(source, "/polymorphic-method-cache.whim");
}

#[test]
fn polymorphic_methods_bind_static_to_the_calling_class() {
    let source = r"
use Whim\Marker\NeverInline;
class Target {
    #[NeverInline]
    public function matches<T>(object $value): bool { return $value is T; }
}
class Caller {
    #[NeverInline]
    public static function matches(object $target, object $value): bool {
        return $target->matches::<static>($value);
    }
}
class First extends Caller {}
class Second extends Caller {}
$target = new Target();
for ($round = 0; $round < 3; $round++) {
    assert!(First::matches($target, new First()));
    assert!(Second::matches($target, new Second()));
    assert!(!Second::matches($target, new First()));
    assert!(!First::matches($target, new Second()));
}
";
    run_both_modes(source, "/polymorphic-method-static.whim");
}

#[test]
fn computed_captures_preserve_snapshots_newtypes_and_return_checks() {
    let source = r"
use Whim\Marker\NeverInline;
newtype CaptureToken = int;
#[NeverInline]
function computed(int $seed): fn(): (int, uint, float, string) {
    $integer = $seed % 31;
    $unsigned = ($seed as uint) % 31u;
    $floating = $seed * 0.5;
    $string = 'value=' . $seed;
    $callback = fn(): (int, uint, float, string) => ($integer, $unsigned, $floating, $string);
    $integer = 0;
    $unsigned = 0u;
    $floating = 0.0;
    $string = '';
    return $callback;
}
#[NeverInline]
function merged(bool $tagged): fn(): (bool, bool) {
    $value = match ($tagged) { true => CaptureToken(7), _ => 7 };
    return fn(): (bool, bool) => ($value is CaptureToken, $value is !CaptureToken);
}
#[NeverInline]
function mixed_capture(bool $valid): fn(): int {
    $value = match ($valid) { true => 7, _ => 'wrong' };
    return fn(): int => $value;
}
assert!(computed(42)() == (11, 11u, 21.0, 'value=42'));
assert!(merged(true)() == (true, false));
assert!(merged(false)() == (false, true));
assert!(mixed_capture(true)() == 7);
$caught = false;
try { mixed_capture(false)(); }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
";
    run_both_modes(source, "/computed-capture-facts.whim");
}

#[test]
fn borrowed_captures_preserve_owners_defaults_and_unwinding() {
    let source = r"
use Whim\Marker\NeverInline;
newtype CaptureId = int;
class Capture {
    public static int $drops = 0;
    public function __construct(public int $value) {}
    public function __destruct(): void { self::$drops++; }
}
#[NeverInline]
function make(int $seed): fn(int): int {
    $object = new Capture($seed);
    $tagged = CaptureId($seed);
    $values = vec[$seed];
    return fn(int $value, int $offset = 1): int {
        assert!($value >= 0 && $tagged is CaptureId);
        $values[] = $value;
        return $object->value + $tagged + $value + $offset + length!($values);
    };
}
#[NeverInline]
function invoke(fn(int): int $callback, int $value): int { return $callback($value); }
#[NeverInline]
function descend(fn(int): int $callback, int $depth): int {
    if ($depth == 0) { return $callback(2); }
    return descend($callback, $depth - 1);
}
#[NeverInline]
function exercise(): void {
    $callback = make(20);
    for ($round = 0; $round < 3; $round++) {
        assert!(invoke($callback, 2) == 45);
        $failed = false;
        try { invoke($callback, -1); }
        catch (Whim\Unwind\AssertionError $_) { $failed = true; }
        assert!($failed && Capture::$drops == 0);
    }
    $callback = $callback(2);
    assert!($callback == 45);
}
exercise();
assert!(Capture::$drops == 1);
#[NeverInline]
function check_limit(): void {
    $callback = make(20);
    $overflowed = false;
    try { descend($callback, 1); }
    catch (Whim\Unwind\StackOverflowError $_) { $overflowed = true; }
    assert!($overflowed && Capture::$drops == 1);
    assert!($callback(2) == 45);
}
check_limit();
assert!(Capture::$drops == 2);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            call_depth_limit: 4,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/borrowed-captures.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
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
