use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn specialized_collection_reads_preserve_scalar_newtypes() {
    let source = r"
use Whim\Marker\NeverInline;
newtype SignedElement = int;
newtype UnsignedElement = uint;
newtype FloatElement = float;
#[NeverInline]
function direct(vec<int> $vector, dict<int, int> $integers, dict<uint, uint> $unsigned,
    dict<string, int> $strings, dict<string, float> $floats, int $index): (int, int, uint, int, float) {
    return ($vector[$index], $integers[$index], $unsigned[0u], $strings['x'], $floats['x']);
}
#[NeverInline]
function iterated(vec<int> $vi, dict<string, int> $di, vec<uint> $vu, dict<uint, uint> $du,
    vec<float> $vf, dict<string, float> $df): (int, int, uint, uint, float, float) {
    $a = 0; foreach ($vi as $value) { $a = $value; }
    $b = 0; foreach ($di as $value) { $b = $value; }
    $c = 0u; foreach ($vu as $value) { $c = $value; }
    $d = 0u; foreach ($du as $value) { $d = $value; }
    $e = 0.0; foreach ($vf as $value) { $e = $value; }
    $f = 0.0; foreach ($df as $value) { $f = $value; }
    return ($a, $b, $c, $d, $e, $f);
}
#[NeverInline]
function numeric_vector(vec<int> $values): (int, int) {
    $last = 0;
    for ($index = 0; $index < 3; $index++) { $last = $values[$index]; }
    return ($last, $index);
}
#[NeverInline]
function numeric_float_vector(vec<float> $values): (float, int) {
    $last = 0.0;
    for ($index = 0; $index < 3; $index++) { $last = $values[$index]; }
    return ($last, $index);
}
#[NeverInline]
function numeric_dict(dict<int, int> $values): (int, int) {
    $last = 0;
    for ($index = 0; $index < 3; $index++) { $last = $values[$index]; }
    return ($last, $index);
}
#[NeverInline]
function copied(dict<int, int> $values): (dict<int, int>, int, int) {
    $copy = dict[];
    $last = 0;
    $index = 2;
    while ($index >= 0) { $last = $values[$index]; $copy[$index] = $last; $index--; }
    return ($copy, $last, $index);
}
#[NeverInline]
function accumulated(dict<int, int> $values): (dict<int, int>, int, int) {
    $target = dict[0 => 10, 1 => 20, 2 => 30];
    $last = 0;
    $index = 2;
    while ($index >= 0) { $last = $values[$index]; $target[$index] += $last; $index--; }
    return ($target, $last, $index);
}
#[NeverInline]
function replaced(vec<int> $values): vec<int> {
    for ($index = 0; $index < 3; $index++) { $values[$index] = $index; }
    return $values;
}
#[NeverInline]
function exactly_three(dict<string, 3> $_values): void {}
#[NeverInline]
function increment(dict<string, int> $values, string $key): dict<string, int> {
    exactly_three($values);
    $values[$key] += 1;
    $caught = false;
    try { exactly_three($values); }
    catch (Whim\Unwind\TypeError $_) { $caught = true; }
    assert!($caught);
    return $values;
}
$i = SignedElement(3);
$u = UnsignedElement(3u);
$f = FloatElement(3.0);
($a, $b, $c, $d, $e) = direct(vec[$i], dict[0 => $i], dict[0u => $u], dict['x' => $i], dict['x' => $f], 0);
assert!($a is SignedElement && $b is SignedElement && $c is UnsignedElement);
assert!($d is SignedElement && $e is FloatElement);
($a, $b, $c, $d, $e, $f) = iterated(vec[$i], dict['x' => $i], vec[$u], dict[0u => $u], vec[$f], dict['x' => $f]);
assert!($a is SignedElement && $b is SignedElement && $c is UnsignedElement);
assert!($d is UnsignedElement && $e is FloatElement && $f is FloatElement);
($last, $index) = numeric_vector(vec[1, 2, $i]);
assert!($last is SignedElement && $last == 3 && $index == 3);
($last, $index) = numeric_float_vector(vec[1.0, 2.0, $f]);
assert!($last is FloatElement && $last == 3.0 && $index == 3);
($last, $index) = numeric_dict(dict[0 => 1, 1 => 2, 2 => $i]);
assert!($last is SignedElement && $last == 3 && $index == 3);
($copy, $last, $index) = copied(dict[0 => $i, 1 => 2, 2 => 1]);
assert!($copy[0] is SignedElement && $last is SignedElement && $index == -1);
assert!($copy[1] == 2 && $copy[2] == 1);
($sum, $last, $index) = accumulated(dict[0 => $i, 1 => 2, 2 => 1]);
assert!($sum == dict[0 => 13, 1 => 22, 2 => 31] && !($sum[0] is SignedElement));
assert!($last is SignedElement && $index == -1);
$replaced = replaced(vec[1, $i, 3]);
assert!($replaced == vec[0, 1, 2] && !($replaced[1] is SignedElement));
foreach (vec['x', 'longer-than-a-short-string'] as $key) {
    $incremented = increment(dict[$key => $i], $key);
    assert!($incremented[$key] == 4 && !($incremented[$key] is SignedElement));
}
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/collection-newtypes.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn cached_collection_arguments_preserve_mutation_and_type_checks() {
    let source = r"
use Whim\Marker\NeverInline;
use Whim\Unwind\TypeError;
#[NeverInline]
function accept<T>(T $value): bool { return true; }
#[NeverInline]
function check<T>(mixed $value): bool { return accept::<T>($value); }
class Item { public function __construct(public mixed $value) {} }
$vector = vec[1];
$dictionary = dict['a' => 1];
$tuple = (1, 'kept');
$item = new Item(1);
$objects = vec[$item];
for ($round = 0; $round < 4; $round++) {
    assert!(check::<vec<int>>($vector));
    assert!(check::<dict<string, int>>($dictionary));
    assert!(check::<(int, string)>($tuple));
    assert!(check::<vec<string>>(vec['kept']));
    assert!(check::<vec<#{ value: int }>>($objects));
    $saved = $vector;
    $vector[0] = 'wrong';
    $dictionary['a'] = 'wrong';
    $item->value = 'wrong';
    $failures = 0;
    try { check::<vec<int>>($vector); } catch (TypeError $_) { $failures++; }
    try { check::<dict<string, int>>($dictionary); } catch (TypeError $_) { $failures++; }
    try { check::<(string, int)>($tuple); } catch (TypeError $_) { $failures++; }
    try { check::<vec<#{ value: int }>>($objects); } catch (TypeError $_) { $failures++; }
    assert!($failures == 4);
    assert!(check::<vec<int>>($saved));
    $vector[0] = 1;
    $dictionary['a'] = 1;
    $item->value = 1;
}
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/cached-collection-arguments.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn collection_getters_preserve_lifetimes_keys_and_faults() {
    let source = r"
use Whim\Marker\NeverInline;
use Whim\Unwind\OutOfBoundsError;
final class Token {
    public static int $drops = 0;
    public function __construct(public string $name) {}
    public function __destruct(): void { self::$drops++; }
}
function vector_item(vec<Token> $values, int $index): Token { return $values[$index]; }
function dictionary_item(dict<string, Token> $values, string $key): Token { return $values[$key]; }
function tuple_item((Token, int) $values): Token { return $values[0]; }
#[NeverInline]
function exercise_lifetimes(): void {
    $first = vector_item(vec[new Token('vector')], 0);
    $second = dictionary_item(dict['key' => new Token('dictionary')], 'key');
    $third = tuple_item((new Token('tuple'), 42));
    assert!(Token::$drops == 0);
    assert!($first->name == 'vector');
    assert!($second->name == 'dictionary');
    assert!($third->name == 'tuple');
}
function integer_key(dict<int|uint|string, int> $values, int $key): int {
    return $values[$key];
}
function unsigned_key(dict<int|uint|string, int> $values, uint $key): int {
    return $values[$key];
}
function string_key(dict<int|uint|string, int> $values, string $key): int {
    return $values[$key];
}
function optional_item(vec<int> $values, int $index): int|null { return $values[$index] ?? null; }
function optional_key(dict<string, int> $values, string $key): int|null { return $values[$key] ?? null; }
function integer_item(vec<int> $values, int $index): int { return $values[$index]; }
#[NeverInline]
function exercise_keys(dict<int|uint|string, int> $values): void {
    assert!(integer_key($values, 1) == 10);
    assert!(unsigned_key($values, 1u) == 20);
    assert!(string_key($values, '1') == 30);
    $caught = false;
    try { integer_key($values, 2); } catch (OutOfBoundsError $_) { $caught = true; }
    assert!($caught);
    $caught = false;
    try { integer_item(vec[10], 1); } catch (OutOfBoundsError $_) { $caught = true; }
    assert!($caught);
    assert!(optional_item(vec[10], 0) == 10);
    assert!(optional_item(vec[10], -1) == null);
    assert!(optional_item(vec[10], 1) == null);
    assert!(optional_key(dict['a' => 10], 'a') == 10);
    assert!(optional_key(dict['a' => 10], 'b') == null);
}
exercise_lifetimes();
assert!(Token::$drops == 3);
exercise_keys(dict[1 => 10, 1u => 20, '1' => 30]);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/collection-reads.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
