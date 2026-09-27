use std::path::Path;

use whim_bytecode::instruction::Instruction;

use crate::engine::Engine;
use crate::engine::EngineConfiguration;

use super::run_both_modes;

#[path = "../../../../tests/_fixtures/constant_branches.rs"]
mod constant_branches;

#[test]
fn constant_conditions_and_matches_select_the_expected_arm() {
    for body in constant_branches::bodies() {
        let source =
            format!("function folded(): string {{ {body} }} assert!(folded() == 'chosen');");
        run_both_modes(&source, "/constant-branches.whim");
    }
}

#[test]
fn branches_preserve_effects_dynamic_choices_and_errors() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function choose(bool $flag): int {
    if (operating_system!() == operating_system!() && $flag) { return 1; }
    return 2;
}
#[Whim\Marker\AlwaysInline]
function incomplete(string $value): int {
    return match ($value) { 'linux' => 1, 'macos' => 2 };
}
assert!(choose(true) == 1);
assert!(choose(false) == 2);
$seen = vec[];
if (($seen[] = 'linux') == 'linux') { $seen[] = 'chosen'; }
else { $seen[] = 'discarded'; }
assert!($seen == vec['linux', 'chosen']);
if (false && sequence!($seen[] = 'discarded', true)) { $seen[] = 'discarded'; }
if (true || sequence!($seen[] = 'discarded', false)) { $seen[] = 'short-circuit'; }
assert!($seen == vec['linux', 'chosen', 'short-circuit']);
foreach (vec[1, 2, 3] as $number) {
    if (false) { $seen[] = 'discarded'; }
}
assert!($seen == vec['linux', 'chosen', 'short-circuit']);
try {
    if ('linux' == 'linux') { $seen[] = 'try'; }
    else { panic!('discarded'); }
} finally { $seen[] = 'finally'; }
assert!($seen == vec['linux', 'chosen', 'short-circuit', 'try', 'finally']);
$caught = false;
try { if ('linux') { panic!('unreachable'); } }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { if ('linux' < 1) { panic!('unreachable'); } }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { $_ = match ('linux') { 'macos' => 1 }; }
catch (Whim\Unwind\UnhandledMatchError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { $_ = incomplete('other'); }
catch (Whim\Unwind\UnhandledMatchError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { $_ = incomplete(null); }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
";
    run_both_modes(source, "/constant-branch-effects.whim");
}

#[test]
fn match_order_and_type_errors_are_preserved() {
    let simple = include_str!("../../../../tests/_fixtures/matching.whim");
    let source = format!(
        r"{simple}
        function duplicate(string $value): int {{
            if ($value == 'foo') {{ return 1; }}
            if ($value == 'foo') {{ return 99; }}
            if ($value == 'bar') {{ return 2; }}
            return 3;
        }}
        function changed(string $value): int {{
            if ($value == 'foo') {{ return 1; }}
            $value = 'bar';
            if ($value == 'bar') {{ return 2; }}
            return 3;
        }}
        #[Whim\Marker\NeverInline]
        function changed_in_body(string $value): int {{
            if ($value == 'foo') {{ $value = 'bar'; }}
            if ($value == 'bar') {{ return 2; }}
            return 90;
        }}
        function typed_binding(string $value): string {{
            return match ($value) {{ 'foo' => 'one', $text @ string => $text }};
        }}
        foreach (vec['foo', 'bar', 'other', '', 'é'] as $value) {{
            assert!(literal_match($value) == typed_match($value));
            assert!(literal_match($value) == conditional($value));
            assert!(literal_match($value) == duplicate($value));
        }}
        assert!(changed('other') == 2);
        assert!(changed_in_body('foo') == 2);
        assert!(changed_in_body('bar') == 2);
        assert!(changed_in_body('other') == 90);
        assert!(typed_binding('other') == 'other');
        $caught = false;
        try {{ invalid_return('other'); }} catch (Whim\Unwind\TypeError $_) {{ $caught = true; }}
        assert!($caught);
        $caught = false;
        try {{ incomplete(1); }} catch (Whim\Unwind\UnhandledMatchError $_) {{ $caught = true; }}
        assert!($caught);
    "
    );
    run_both_modes(&source, "/project/matching.whim");
}

#[test]
fn overlapping_patterns_preserve_order_and_unhandled_errors() {
    run_both_modes(
        include_str!("../../../../tests/_fixtures/pattern-order.whim"),
        "/matching/impossible-types.whim",
    );
}

#[test]
fn fast_is_branches_preserve_live_results_shapes_and_backedges() {
    let source = r"
use Whim\Marker\NeverInline;
final class Leaf { public mixed $value; }
final class Other {}
newtype Wrapped = Leaf;
#[NeverInline]
function choose(mixed $value): (bool, int) {
    $matches = $value is Leaf;
    if ($matches) { return ($matches, 1); }
    return ($matches, 2);
}
#[NeverInline]
function pattern(mixed $value): mixed {
    return match ($value) { Leaf #{ value: $item } => $item, _ => 'missing' };
}
#[NeverInline]
function repeat(mixed $value): int {
    $count = 0;
    do {
        $count++;
        if ($count == 3) { $value = null; }
    } while ($value is Leaf);
    return $count;
}
#[NeverInline]
function different(mixed $value, mixed $condition): bool {
    $matches = $value is Leaf;
    if ($condition) { return $matches; }
    return !$matches;
}
$leaf = new Leaf();
for ($round = 0; $round < 3; $round++) {
    assert!(choose($leaf) == (true, 1));
    assert!(choose(Wrapped($leaf)) == (true, 1));
    assert!(choose(new Other()) == (false, 2));
    assert!(choose(null) == (false, 2));
    assert!(repeat($leaf) == 3 && repeat(null) == 1);
    assert!(different($leaf, true) && !different($leaf, false));
}
assert!(pattern($leaf) == 'missing');
$leaf->value = 9;
assert!(pattern($leaf) == 9);
$leaf->value = null;
assert!(pattern(Wrapped($leaf)) == null);
$caught = false;
try { discard!(different($leaf, 7)); }
catch (Whim\Unwind\TypeError $error) {
    assert!($error->getMessage() == 'a condition must be bool, int given');
    $caught = true;
}
assert!($caught);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/fast-is-branch-values.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        if !optimize {
            continue;
        }
        let code = |name: &[u8]| {
            let function = engine
                .tables
                .functions
                .iter()
                .find(|function| function.name.as_bytes() == name)
                .unwrap();
            // SAFETY: the idle engine owns this function chunk for the read.
            &unsafe { function.chunk.as_ref() }.code
        };
        for (name, minimum) in [(b"choose".as_slice(), 1), (b"pattern".as_slice(), 2)] {
            let code = code(name);
            let count = code.windows(2).filter(|pair| matches!(pair,
                [Instruction::Is { destination, .. }, Instruction::JumpIfFalse { condition, offset }]
                    if destination == condition && offset.offset() > 0
            )).count();
            assert!(count >= minimum, "{name:?}: {code:?}");
        }
        let repeat = code(b"repeat");
        assert!(
            repeat.windows(2).any(|pair| matches!(pair,
                [Instruction::Is { destination, .. }, Instruction::JumpIfTrue { condition, offset }]
                    if destination == condition && offset.offset() < 0
            )),
            "repeat: {repeat:?}"
        );
        let different = code(b"different");
        assert!(
            different.windows(2).any(|pair| matches!(pair,
                [Instruction::Is { destination, .. }, Instruction::JumpIfFalse { condition, .. }]
                    if destination != condition
            )),
            "different: {different:?}"
        );
    }
}

#[test]
fn fast_is_branches_preserve_fallback_effects_and_fault_positions() {
    let source = r"
use Whim\Marker\NeverInline;
final class Leaf {}
final class Loads { public static int $count = 0; }
type Bounded<T: uint> = T;
Whim\_Private\register_symbol_autoloader(fn(int $kind, string $name): void {
    if ($name == 'MissingBranchType') { Loads::$count++; }
});
#[NeverInline]
function unresolved(mixed $value): bool {
    if ($value is MissingBranchType) { return true; }
    return false;
}
#[NeverInline]
function bounded<T>(mixed $value): bool {
    if ($value is Bounded<T>) { return true; }
    return false;
}
#[NeverInline]
function fail_after_branch(mixed $value): void {
    if ($value is Leaf) {
        throw new Whim\Unwind\Exception('chosen branch');
    }
    throw new Whim\Unwind\Exception('other branch');
}
$leaf = new Leaf();
for ($round = 0; $round < 3; $round++) {
    $before = Loads::$count;
    assert!(!unresolved($leaf));
    assert!(Loads::$count > $before);
    assert!(bounded::<uint>(1u));
    $caught = false;
    try { discard!(bounded::<string>('value')); }
    catch (Whim\Unwind\TypeError $_) { $caught = true; }
    assert!($caught);
}
";
    let chosen_line = source
        .lines()
        .position(|line| line.contains("Exception('chosen branch')"))
        .unwrap()
        + 1;
    let other_line = source
        .lines()
        .position(|line| line.contains("Exception('other branch')"))
        .unwrap()
        + 1;
    let source = format!(
        r"{source}
foreach (vec[($leaf, 'chosen branch', {chosen_line}), (null, 'other branch', {other_line})] as $case) {{
    $caught = false;
    try {{ fail_after_branch($case[0]); }}
    catch (Whim\Unwind\Exception $error) {{
        assert!($error->getMessage() == $case[1]);
        assert!($error->getFile() == '/fast-is-branch-effects.whim');
        assert!($error->getLine() == $case[2]);
        assert!($error->getTrace()[0]->function == 'fail_after_branch');
        $caught = true;
    }}
    assert!($caught);
}}
"
    );
    run_both_modes(&source, "/fast-is-branch-effects.whim");
}

#[test]
fn fast_is_branches_keep_finalizer_order_and_throwing_finalizers() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
final class Leaf {}
final class Events { public static vec<string> $items = vec[]; }
final class Released {
    public function __destruct(): void { Events::$items[] = 'drop'; }
}
final class Throwing {
    public function __destruct(): void { throw new Whim\Unwind\Exception('drop failed'); }
}
#[NeverInline]
function choose(mixed $value): bool {
    $result = new Released();
    $result = $value is Leaf;
    if ($result) { Events::$items[] = 'true'; }
    else { Events::$items[] = 'false'; }
    return $result;
}
#[NeverInline]
function failed(mixed $value): void {
    $result = new Throwing();
    $result = $value is Leaf;
    if ($result) { Events::$items[] = 'unreachable true'; }
    else { Events::$items[] = 'unreachable false'; }
}
assert!(choose(new Leaf()));
assert!(!choose(null));
assert!(Events::$items == vec['drop', 'true', 'drop', 'false']);
foreach (vec[new Leaf(), null] as $value) {
    $caught = false;
    try { failed($value); }
    catch (Whim\Unwind\Exception $error) {
        assert!($error->getMessage() == 'drop failed');
        $caught = true;
    }
    assert!($caught);
}
assert!(Events::$items == vec['drop', 'true', 'drop', 'false']);
",
        "/fast-is-branch-finalizers.whim",
    );
}
