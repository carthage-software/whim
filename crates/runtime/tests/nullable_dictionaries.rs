use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

fn run(source: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/nullable-dictionaries.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn dictionary_feedback_keeps_keys_loops_and_snapshots() {
    run(r"
use Whim\Marker\NeverInline;
#[NeverInline]
function counts(vec<int|uint|string> $keys): dict {
    $counts = dict[];
    foreach ($keys as $key) { $counts[$key] = ($counts[$key] ?? 0) + 1; }
    return $counts;
}
#[NeverInline]
function nested(int $rounds): dict {
    $counts = dict[];
    $round = 0;
    while ($round < $rounds) {
        $inner = 0;
        while ($inner < 3) {
            $counts[$inner] = ($counts[$inner] ?? 0) + 1;
            $inner++;
        }
        $round++;
    }
    return $counts;
}
#[NeverInline]
function snapshots(vec<string> $keys): (dict, dict, dict) {
    $counts = dict[];
    $snapshot = $counts;
    $copy = $counts;
    foreach ($keys as $key) {
        $copy = $counts;
        $copy[$key] = 0.5;
        $counts[$key] = ($counts[$key] ?? 0) + 1;
    }
    return ($counts, $snapshot, $copy);
}
assert!(counts(vec[]) == dict[]);
assert!(counts(vec[1, 1u, '1', 1, '1']) == dict[1 => 2, 1u => 1, '1' => 2]);
assert!(nested(0) == dict[]);
assert!(nested(4) == dict[0 => 4, 1 => 4, 2 => 4]);
assert!(snapshots(vec['a', 'a']) == (dict['a' => 2], dict[], dict['a' => 0.5]));
$rows = vec[];
for ($round = 0; $round < 4; $round++) {
    $counts = dict[];
    $counts['a'] = ($counts['a'] ?? 0) + 1;
    $rows[] = $counts;
}
assert!($rows == vec[dict['a' => 1], dict['a' => 1], dict['a' => 1], dict['a' => 1]]);
");
}

#[test]
fn nullable_reads_widen_after_writes_calls_and_escapes() {
    run(r"
use Whim\Marker\NeverInline;
final class Holder { public dict $values = dict[]; }
#[NeverInline]
function mutate(dict $values): dict { $values['a'] = 0.5; return $values; }
#[NeverInline]
function values(vec<mixed> $values): vec<mixed> {
    $counts = dict[];
    $results = vec[];
    foreach ($values as $value) {
        $counts['a'] = $value;
        $results[] = ($counts['a'] ?? 0) + 1;
    }
    return $results;
}
#[NeverInline]
function parameter(dict $values): mixed { return ($values['a'] ?? 0) + 1; }
#[NeverInline]
function generic<T>(dict<string, T> $values): mixed { return ($values['a'] ?? 0) + 1; }
#[NeverInline]
function escaped(): (dict, dict, dict) {
    $values = dict[];
    $holder = new Holder();
    $holder->values = $values;
    $changed = mutate($values);
    $holder->values['a'] = 3.5;
    $values['a'] = ($values['a'] ?? 0) + 1;
    return ($values, $changed, $holder->values);
}
assert!(values(vec[null, 2, 0.5]) == vec[1, 3, 1.5]);
assert!(parameter(dict[]) == 1);
assert!(parameter(dict['a' => 0.5]) == 1.5);
assert!(generic::<float>(dict['a' => 0.5]) == 1.5);
assert!(escaped() == (dict['a' => 1], dict['a' => 0.5], dict['a' => 3.5]));
$counts = dict[];
$operation = fn(): dict => mutate($counts);
$changed = $operation();
$counts['a'] = ($counts['a'] ?? 0) + 1;
assert!($changed == dict['a' => 0.5] && $counts == dict['a' => 1]);
$copy = dict[...$counts, ...dict['a' => 2.5]];
assert!(parameter($copy) == 3.5);
");
}

#[test]
fn reads_keep_errors_tags_and_observable_producers() {
    run(r"
use Whim\Marker\NeverInline;
use Whim\Unwind\Throwable;
newtype Tagged = int;
final class Token { public static int $calls = 0; }
#[NeverInline]
function value(mixed $value): mixed { Token::$calls++; return $value; }
#[NeverInline]
function compute(mixed $key, mixed $value): mixed {
    $counts = dict[];
    $counts[$key] = value($value);
    return ($counts[$key] ?? 0) + 1;
}
#[NeverInline]
function empty_read(mixed $key): mixed {
    $counts = dict[];
    return $counts[$key] ?? 0;
}
assert!(compute('a', null) == 1);
assert!(compute('a', Tagged(4)) == 5);
assert!(Token::$calls == 2);
$caught = 0;
foreach (vec['wrong', 9223372036854775807] as $bad) {
    $saved = 'unchanged';
    try { $saved = compute('a', $bad); }
    catch (Throwable $_) { $caught++; assert!($saved == 'unchanged'); }
}
assert!($caught == 2 && Token::$calls == 4);
$caught = false;
try { compute(vec[], 3); } catch (Throwable $_) { $caught = true; }
assert!($caught);
$caught = false;
try { empty_read(vec[]); } catch (Throwable $_) { $caught = true; }
assert!($caught && empty_read('missing') == 0);
");
}
