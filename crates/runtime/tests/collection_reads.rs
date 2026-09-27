use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn specialized_positional_indices_accept_both_integer_kinds() {
    let source = r"
use Whim\Marker\NeverInline;
use Whim\Unwind\OutOfBoundsError;
#[NeverInline]
function unsigned_read(vec<int> $values, uint $index): int { return $values[$index]; }
#[NeverInline]
function read(vec<int> $values, int|uint $index): int { return $values[$index]; }
#[NeverInline]
function write(vec<int> $values, int|uint $index, int $value): vec<int> {
    $values[$index] = $value;
    return $values;
}
#[NeverInline]
function optional(vec<int|null> $values, int|uint $index): int {
    return $values[$index] ?? 42;
}
#[NeverInline]
function byte(string $value, int|uint $index): string { return $value[$index]; }
#[NeverInline]
function optional_byte(string $value, int|uint $index): string { return $value[$index] ?? 'x'; }
#[NeverInline]
function compare_byte(string $value, int|uint $index): vec<bool> {
    return vec[$value[$index] == 'b', $value[$index] != 'b', $value[$index] < 'b',
        $value[$index] <= 'b', $value[$index] > 'b', $value[$index] >= 'b'];
}
#[NeverInline]
function branch_byte(string $value, int|uint $index): int {
    if ($value[$index] == 'b') { return 1; }
    if ($value[$index] != 'a') { return 3; }
    return 2;
}
#[NeverInline]
function numeric_read(vec<int> $values, int|uint $position): int {
    $sum = 0;
    for ($index = 0; $index < 4; $index++) { $sum += $values[$position]; }
    return $sum;
}
#[NeverInline]
function numeric_write(vec<int> $values, int|uint $position): vec<int> {
    for ($index = 0; $index < 4; $index++) { $values[$position] = $index; }
    return $values;
}
function fails(fn(): mixed $operation, int|uint $index): void {
    $caught = false;
    try { $operation(); } catch (OutOfBoundsError $error) {
        assert!($error->getMessage() == 'the index ' . $index . ' is outside the range 0 to 1');
        $caught = true;
    }
    assert!($caught);
}
assert!(unsigned_read(vec[10, 20], 1u) == 20);
foreach (vec[1, 1u] as $index) {
    assert!(read(vec[10, 20], $index) == 20);
    assert!(write(vec[10, 20], $index, 30) == vec[10, 30]);
    assert!(optional(vec[10, 20], $index) == 20);
    assert!(optional(vec[10, null], $index) == 42);
    assert!(byte('ab', $index) == 'b');
    assert!(optional_byte('ab', $index) == 'b');
    assert!(compare_byte('ab', $index) == vec[true, false, false, true, false, true]);
    assert!(branch_byte('ab', $index) == 1);
    assert!(branch_byte('aa', $index) == 2);
    assert!(branch_byte('ac', $index) == 3);
    assert!(numeric_read(vec[10, 20], $index) == 80);
    assert!(numeric_write(vec[10, 20], $index) == vec[10, 3]);
}
foreach (vec[-1, 2u, 9_223_372_036_854_775_808u, 18_446_744_073_709_551_615u] as $index) {
    fails(fn(): mixed => read(vec[10, 20], $index), $index);
    fails(fn(): mixed => write(vec[10, 20], $index, 30), $index);
    fails(fn(): mixed => byte('ab', $index), $index);
    fails(fn(): mixed => compare_byte('ab', $index), $index);
    fails(fn(): mixed => branch_byte('ab', $index), $index);
    assert!(optional(vec[10, 20], $index) == 42);
    assert!(optional_byte('ab', $index) == 'x');
}
assert!(numeric_read(vec[10, 20], 0u) == 40);
assert!(numeric_write(vec[10, 20], 0u) == vec[3, 20]);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/positional-integer-indices.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

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

#[test]
fn shape_indices_preserve_keys_mutation_newtypes_and_faults() {
    let source = r"
use Whim\Marker\NeverInline;
use Whim\Unwind\OutOfBoundsError;
use Whim\Unwind\TypeError;
type Line = dict['quantity' => int, 'price' => int];
type Order = dict['lines' => vec<Line>, 'shipping' => dict['price' => int]];
newtype ShapeAmount = int;
#[NeverInline]
function total(Order $order): int {
    $total = 0;
    foreach ($order['lines'] as $line) { $total += $line['quantity'] * $line['price']; }
    return $total + $order['shipping']['price'];
}
#[NeverInline]
function keys(dict[1 => int, 1u => string, true => bool, '1' => float] $row): (int, string, bool, float) {
    return ($row[1], $row[1u], $row[true], $row['1']);
}
#[NeverInline]
function positions(vec[int, string, ...bool] $row): (int, string, bool|null) {
    return ($row[0], $row[1u], $row[2u] ?? null);
}
#[NeverInline]
function rest(dict['n' => int, ...<string, bool>] $row): (int, bool|null) {
    return ($row['n'], $row['other'] ?? null);
}
#[NeverInline]
function optional(dict['n' => int|null] $row): (int|null, mixed) {
    return ($row['n'] ?? null, $row['absent'] ?? null);
}
#[NeverInline]
function changed(dict['n' => int, 'child' => dict['n' => int]] $row, mixed $value): (mixed, int, mixed, int) {
    $saved = $row;
    $row['n'] = $value;
    $row['child']['n'] = $value;
    return ($row['n'], $saved['n'], $row['child']['n'], $saved['child']['n']);
}
#[NeverInline]
function repeated(dict['n' => int] $row): vec<mixed> {
    $values = vec[];
    for ($index = 0; $index < 2; $index++) {
        $values[] = $row['n'];
        $row['n'] = 'changed';
    }
    return $values;
}
#[NeverInline]
function wrong_return(dict['n' => int] $row, mixed $value): int {
    $row['n'] = $value;
    return $row['n'];
}
#[NeverInline]
function absent(dict['n' => int] $row): mixed { return $row['absent']; }
#[NeverInline]
function removed(dict['n' => int] $row): mixed { remove!($row, 'n'); return $row['n']; }
#[NeverInline]
function beyond(vec[int, string] $row): mixed { return $row[2u]; }
#[NeverInline]
function scalar(dict['n' => int] $row): int { return $row['n']; }
#[NeverInline]
function nominal(dict['n' => ShapeAmount] $row): ShapeAmount { return $row['n']; }
#[NeverInline]
function selected(dict['n' => int]|dict['n' => string] $row): mixed { return $row['n']; }
class MutableField {
    public static int $accepted = 0;
    public function __construct(public mixed $value) {}
}
#[NeverInline]
function accept_shape(#{ value: int } $value): void { MutableField::$accepted++; }
#[NeverInline]
function changed_property(dict['child' => #{ value: int }] $row): void {
    $item = $row['child'];
    $item->value = 'wrong';
    $caught = false;
    try { accept_shape($row['child']); } catch (TypeError $_) { $caught = true; }
    assert!($caught);
    assert!(MutableField::$accepted == 0);
}
#[NeverInline]
function property(dict['child' => #{ value: int }] $row): mixed {
    $row['child']->value = 'changed';
    return $row['child']->value;
}
assert!(total(dict['lines' => vec[dict['quantity' => 3, 'price' => 7], dict['quantity' => 2, 'price' => 11]], 'shipping' => dict['price' => 5]]) == 48);
assert!(keys(dict[1 => 12, 1u => 'unsigned', true => false, '1' => 1.5]) == (12, 'unsigned', false, 1.5));
assert!(positions(vec[12, 'text', true]) == (12, 'text', true));
assert!(positions(vec[12, 'text']) == (12, 'text', null));
assert!(rest(dict['n' => 7, 'other' => false]) == (7, false));
assert!(rest(dict['n' => 7]) == (7, null));
assert!(optional(dict['n' => 7]) == (7, null));
assert!(optional(dict['n' => null]) == (null, null));
assert!(changed(dict['n' => 7, 'child' => dict['n' => 8]], 'changed') == ('changed', 7, 'changed', 8));
assert!(repeated(dict['n' => 7]) == vec[7, 'changed']);
$caught = false;
try { wrong_return(dict['n' => 7], 'changed'); } catch (TypeError $_) { $caught = true; }
assert!($caught);
foreach (vec[fn(): mixed => absent(dict['n' => 7]), fn(): mixed => removed(dict['n' => 7]), fn(): mixed => beyond(vec[7, 'text'])] as $operation) {
    $caught = false;
    try { $operation(); } catch (OutOfBoundsError $_) { $caught = true; }
    assert!($caught);
}
$amount = ShapeAmount(9);
assert!(scalar(dict['n' => $amount]) is ShapeAmount);
assert!(nominal(dict['n' => $amount]) is ShapeAmount);
assert!(selected(dict['n' => 7]) == 7);
assert!(selected(dict['n' => 'text']) == 'text');
assert!(property(dict['child' => new MutableField(7)]) == 'changed');
changed_property(dict['child' => new MutableField(7)]);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/shape-indices.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
