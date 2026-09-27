use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

fn run(source: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/string-length-proofs.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn exact_lengths_keep_copies_joins_tags_and_mutations() {
    run(r"
use Whim\Marker\NeverInline;
newtype Tagged = string[1];
newtype Outer = Tagged;
newtype Three = string[3];
class Cell { public mixed $value = 'a'; }
#[NeverInline]
function join(string[1] $value): string[3] { $copy = $value; return $copy . $value . $copy; }
#[NeverInline]
function width(string[1] $value): uint { return length!('ab' . $value); }
#[NeverInline]
function append(string[1] $value): string[3] { $value .= 'b'; $value .= 'c'; return $value; }
#[NeverInline]
function choose(string[1] $value, string $other, bool $condition): string[3] {
    if ($condition) { $value = $other; }
    return 'ab' . $value;
}
#[NeverInline]
function range(string[1..=3] $value): uint { return length!($value . $value); }
#[NeverInline]
function zero(string[0] $value): string[0] { return $value . $value; }
#[NeverInline]
function mutable(#{ value: string[1] } $object, fn(): void $change): string[3] {
    $change();
    return 'ab' . $object->value;
}
#[NeverInline]
function copied(#{ value: string[1] } $object, fn(): void $change): string[3] {
    $value = $object->value;
    $change();
    return 'ab' . $value;
}
#[NeverInline]
function tagged(string[1] $value): Three { return 'ab' . $value; }
function rejects(fn(): mixed $operation): void {
    $caught = false;
    try { $operation(); } catch (Whim\Unwind\TypeError $_error) { $caught = true; }
    assert!($caught);
}
foreach (vec['a', Tagged('a'), Outer(Tagged('a'))] as $value) {
    assert!(join($value) == 'aaa');
    assert!(width($value) == 3u);
    assert!(append($value) == 'abc');
    assert!(!(join($value) is Tagged|Outer));
}
assert!(choose('a', 'z', false) == 'aba');
assert!(choose('a', 'z', true) == 'abz');
rejects(fn(): mixed => choose('a', 'longer', true));
assert!(range('a') == 2u);
assert!(range('abc') == 6u);
assert!(zero('') == '');
$cell = new Cell();
rejects(fn(): mixed => mutable($cell, fn(): void { $cell->value = 'longer'; }));
$cell->value = 'a';
assert!(copied($cell, fn(): void { $cell->value = 'longer'; }) == 'aba');
rejects(fn(): mixed => tagged('a'));
assert!(length!(vec['a', 'bb', 'ccc']) == 3u);
assert!(length!(dict['a' => 1, 'b' => 2]) == 2u);
");
}

#[test]
fn folded_lengths_keep_calls_throws_and_must_use_rules() {
    run(r"
use Whim\Marker\MustUse;
use Whim\Marker\NeverInline;
use Whim\Unwind\DiscardedResultError;
use Whim\Unwind\Exception;
class Effects { public static int $calls = 0; }
#[MustUse]
#[NeverInline]
function byte(bool $fail): string[1] {
    Effects::$calls++;
    if ($fail) { throw new Exception('called'); }
    return 'a';
}
#[NeverInline]
function invalid(string $value): string[1] { Effects::$calls++; return $value; }
#[NeverInline]
function width(bool $fail): uint { return length!(byte($fail)); }
#[NeverInline]
function callback(fn(): string[1] $operation): uint { return length!('ab' . $operation()); }
#[NeverInline]
function invalid_width(string $value): uint { return length!(invalid($value)); }
#[NeverInline]
function discard(): void { byte(false); }
final class Reader {
    #[MustUse]
    #[NeverInline]
    public function byte(): string[1] { Effects::$calls++; return 'a'; }
}
#[NeverInline]
function method(Reader $reader): uint { return length!($reader->byte()); }
assert!(width(false) == 1u);
assert!(Effects::$calls == 1);
$caught = false;
try { width(true); } catch (Exception $error) { $caught = $error->getMessage() == 'called'; }
assert!($caught);
assert!(Effects::$calls == 2);
assert!(callback(fn(): string[1] => byte(false)) == 3u);
assert!(Effects::$calls == 3);
assert!(method(new Reader()) == 1u);
assert!(Effects::$calls == 4);
$caught = false;
try { invalid_width('longer'); } catch (Whim\Unwind\TypeError $_error) { $caught = true; }
assert!($caught);
assert!(Effects::$calls == 5);
$caught = false;
try { discard(); } catch (DiscardedResultError $_error) { $caught = true; }
assert!($caught);
assert!(Effects::$calls == 6);
assert!(length!(Whim\Str\chr(0)) == 1u);
");
}

#[test]
fn concatenated_returns_keep_named_alias_resolution() {
    let mut saw_autoload = false;
    for expected in [
        "Defaulted",
        "Bounded<string>",
        "Hidden",
        "Missing|Bounded<string>",
    ] {
        let source = format!(
            r"
type Defaulted<T = Missing> = string[3];
type Bounded<T: Missing|string> = string[3];
type Erase<T> = string[3];
type Hidden = Erase<Missing>;
class Loads {{ public static int $count = 0; }}
#[Whim\Marker\NeverInline]
function join(string[1] $value): {expected} {{ return 'ab' . $value; }}
#[Whim\Marker\NeverInline]
function accept({expected} $value): void {{}}
#[Whim\Marker\NeverInline]
function pass(string[1] $value): void {{ accept('ab' . $value); }}
Whim\_Private\register_symbol_autoloader(fn(int $kind, string $name): void {{
    if ($name == 'Missing') {{ Loads::$count++; }}
}});
join('a');
pass('a');
$before = Loads::$count;
assert!(join('b') == 'abb');
pass('b');
assert!(Loads::$count - $before < 255);
exit!(Loads::$count - $before);
"
        );
        let mut baseline = None;
        for optimize in [false, true] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                ..EngineConfiguration::default()
            });
            let result = engine.run_source(&source, Path::new("/string-length-alias-effects.whim"));
            assert_ne!(
                result.exit_code(),
                255,
                "{expected}, optimization {optimize}: {result:?}"
            );
            if let Some(baseline) = baseline {
                assert_eq!(result.exit_code(), baseline, "{expected}");
            } else {
                baseline = Some(result.exit_code());
                saw_autoload |= result.exit_code() > 0;
            }
        }
    }
    assert!(saw_autoload);
}
