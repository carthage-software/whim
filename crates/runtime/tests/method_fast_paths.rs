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
for ($round = 0; $round < 4; $round++) {
    assert!(invoke($defaulted, 40) == 42);
    assert!(invoke($capture, 2) == 42);
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
