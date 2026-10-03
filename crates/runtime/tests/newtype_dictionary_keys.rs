use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

fn run(source: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/newtype-dictionary-keys.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn tagged_keys_keep_identity_through_specialized_access_and_copies() {
    run(r"
use Whim\Marker\NeverInline;
newtype UserId = int;
newtype OrderId = int;
newtype UnsignedId = uint;
newtype Flag = bool;
newtype Name = string;
#[NeverInline]
function signed(dict $values, int $key): mixed { return $values[$key]; }
#[NeverInline]
function unsigned(dict $values, uint $key): mixed { return $values[$key]; }
#[NeverInline]
function text(dict $values, string $key): mixed { return $values[$key]; }
#[NeverInline]
function optional(dict $values, int|uint|string $key): mixed { return $values[$key] ?? 'missing'; }
#[NeverInline]
function optionalSigned(dict $values, int $key): mixed { return $values[$key] ?? 'missing'; }
#[NeverInline]
function optionalUnsigned(dict $values, uint $key): mixed { return $values[$key] ?? 'missing'; }
#[NeverInline]
function optionalText(dict $values, string $key): mixed { return $values[$key] ?? 'missing'; }
#[NeverInline]
function update(dict $values, int $key): dict { $values[$key] += 1; return $values; }
#[NeverInline]
function updateText(dict<string, int> $values, string $key): dict<string, int> { $values[$key] += 1; return $values; }
#[NeverInline]
function assignUnsigned(dict $values, uint $key): dict { $values[$key] = 5; return $values; }
#[NeverInline]
function assignText(dict $values, string $key): dict { $values[$key] = 6; return $values; }
$values = dict[0 => 10, 1 => 11];
$values[UserId(0)] = 20;
$values[OrderId(0)] = 30;
$values[UnsignedId(0u)] = 40;
$values[0u] = 50;
$values[Flag(true)] = 60;
$values[true] = 70;
$values[Name('short')] = 80;
$values['short'] = 90;
$values[Name('a longer string key')] = 100;
$values['a longer string key'] = 110;
assert!(length!($values) == 12u);
assert!(signed($values, UserId(0)) == 20 && signed($values, OrderId(0)) == 30);
assert!(signed($values, 0) == 10 && unsigned($values, 0u) == 50);
assert!(unsigned($values, UnsignedId(0u)) == 40 && $values[Flag(true)] == 60);
assert!(text($values, Name('short')) == 80 && text($values, 'short') == 90);
assert!(text($values, Name('a longer string key')) == 100);
assert!($values[UserId(0) as int] == 10);
assert!($values[UnsignedId(0u) as uint] == 50);
assert!($values[Name('short') as string] == 90);
assert!(optional($values, UserId(1)) == 'missing');
assert!(optional($values, UnsignedId(1u)) == 'missing');
assert!(optional($values, Name('absent')) == 'missing');
assert!(optional($values, UserId(0)) == 20 && optional($values, UnsignedId(0u)) == 40);
assert!(optional($values, Name('short')) == 80);
assert!(optionalSigned($values, UserId(0)) == 20 && optionalSigned($values, UserId(1)) == 'missing');
assert!(optionalUnsigned($values, UnsignedId(0u)) == 40 && optionalUnsigned($values, UnsignedId(1u)) == 'missing');
assert!(optionalText($values, Name('short')) == 80 && optionalText($values, Name('absent')) == 'missing');
$copy = update($values, UserId(0));
$copy = assignUnsigned($copy, UnsignedId(0u));
$copy = assignText($copy, Name('short'));
assert!($values[UserId(0)] == 20 && $copy[UserId(0)] == 21 && $copy[0] == 10);
assert!($values[UnsignedId(0u)] == 40 && $copy[UnsignedId(0u)] == 5 && $copy[0u] == 50);
assert!($values[Name('short')] == 80 && $copy[Name('short')] == 6 && $copy['short'] == 90);
$spread = dict[...$values];
assert!($spread == $values && $spread != $copy);
foreach ($spread as $key => $value) { assert!($values[$key] == $value); }
assert!(contains_key!($values, UserId(0)) && !contains_key!($values, UserId(1)));
remove!($spread, UserId(0));
assert!(!contains_key!($spread, UserId(0)) && contains_key!($spread, OrderId(0)));
assert!($spread[0] == 10 && contains_key!($values, UserId(0)));
assert!(dict[UserId(42) => 1] != dict[OrderId(42) => 1]);
assert!(dict[UserId(42) => 1] != dict[42 => 1]);
assert!(updateText(dict[Name('short') => 1, 'short' => 2], Name('short')) == dict[Name('short') => 2, 'short' => 2]);
assert!(updateText(dict[Name('a longer string key') => 1], Name('a longer string key')) == dict[Name('a longer string key') => 2]);
assert!(!(dict[Name('short') => 1] is dict['short' => int]));
assert!(Whim\_Private\dict_count_values(vec[UserId(0), OrderId(0), UserId(0), 0]) == dict[UserId(0) => 2u, OrderId(0) => 1u, 0 => 1u]);
assert!(Whim\_Private\vec_unique(vec[UserId(42), OrderId(42), 42]) == vec[UserId(42)]);
");
}

#[test]
fn tag_identity_uses_own_type_arguments_and_full_parent_chain() {
    run(r"
use Whim\Marker\NeverInline;
newtype Id = int;
newtype Outer = int;
newtype GenericId<T = string> = int;
#[NeverInline]
function make<T>(int $value): Id { return Id($value); }
#[NeverInline]
function cast<T>(int $value): Id { return $value as Id; }
#[NeverInline]
function generic<T>(int $value): GenericId<T> { return GenericId::<T>($value); }
$values = dict[Id(42) => 'id', GenericId(42) => 'string', GenericId::<int>(42) => 'int'];
assert!($values[make::<string>(42)] == 'id' && $values[make::<uint>(42)] == 'id');
assert!($values[cast::<bool>(42)] == 'id');
assert!($values[generic::<string>(42)] == 'string' && $values[generic::<int>(42)] == 'int');
$values[GenericId::<int|string>(42)] = 'union';
assert!($values[GenericId::<string|int>(42)] == 'union');
assert!($values[generic::<string|int>(42)] == 'union');
$values[Outer(42)] = 'outer';
$values[Outer(Id(42))] = 'nested';
assert!($values[Outer(42)] == 'outer' && $values[Outer(make::<bool>(42))] == 'nested');
assert!(length!($values) == 6u);
foreach (dict[Id(42) => 'id'] as $key => $_) { assert!($key is Id); }
assert!(Whim\Type\of($values) == Whim\Type\id::<dict<Id|GenericId<string>|GenericId<int>|GenericId<int|string>|Outer, string>>());
");
}
