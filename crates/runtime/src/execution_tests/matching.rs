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
