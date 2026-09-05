use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn frame_growth_preserves_call_depth_and_caught_unwinding() {
    for optimize in [false, true] {
        for call_depth_limit in [2, 3, 5, 17, 65] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                call_depth_limit,
                ..EngineConfiguration::default()
            });
            let source = format!(
                r"
function descend(int $remaining): int {{
    if ($remaining == 0) {{
        return 0;
    }}
    return descend($remaining - 1) + 1;
}}
$caught = false;
assert!(descend({maximum}) == {maximum});
try {{
    descend({overflow});
}} catch (Whim\Unwind\StackOverflowError $error) {{
    $caught = true;
}}
assert!($caught);
assert!(descend({maximum}) == {maximum});
",
                maximum = call_depth_limit - 2,
                overflow = call_depth_limit * 4,
            );
            let result = engine.run_source(&source, Path::new("/frame-growth.whim"));
            assert_eq!(
                result.exit_code(),
                0,
                "depth {call_depth_limit}, optimization {optimize}",
            );
        }
    }
}

#[test]
fn tiny_depth_limits_are_not_relaxed_by_detached_frame_capacity() {
    for optimize in [false, true] {
        for call_depth_limit in [0, 1] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                call_depth_limit,
                ..EngineConfiguration::default()
            });
            let source = r"
function recurse(int $value): int { return recurse($value + 1); }
$caught = false;
try {
    recurse(0);
} catch (Whim\Unwind\StackOverflowError $error) {
    $caught = true;
}
assert!($caught);
";
            let result = engine.run_source(source, Path::new("/tiny-frame-depth.whim"));
            assert_eq!(
                result.exit_code(),
                0,
                "depth {call_depth_limit}, optimization {optimize}",
            );
        }
    }
}

#[test]
fn function_value_growth_preserves_arguments_and_recovers_from_overflow() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            call_depth_limit: 37,
            ..EngineConfiguration::default()
        });

        let source = r"
class FrameGrowthArgument {
    public static int $drops = 0;
    public function __construct(public int $value) {}
    public function __destruct(): void { self::$drops++; }
}

#[Whim\Marker\NeverInline]
function descend(
    int $remaining,
    fn(FrameGrowthArgument, vec<int>, int): vec<int> $callable,
    FrameGrowthArgument $argument,
    vec<int> $values,
): vec<int> {
    if ($remaining == 0) {
        return $callable($argument, $values, 3);
    }

    return descend($remaining - 1, $callable, $argument, $values);
}

#[Whim\Marker\NeverInline]
function attempt_overflow(
    fn(FrameGrowthArgument, vec<int>, int): vec<int> $callable,
    vec<int> $values,
): bool {
    try {
        descend(34, $callable, new FrameGrowthArgument(2), $values);
    } catch (Whim\Unwind\StackOverflowError $error) {
        return true;
    }

    return false;
}

$callable = function(FrameGrowthArgument $argument, vec<int> $values, int $last): vec<int> {
    $values[] = $argument->value;
    $values[] = $last;
    return $values;
};
$original = vec[1];

for ($depth = 0; $depth < 33; $depth++) {
    assert!(descend($depth, $callable, new FrameGrowthArgument(2), $original) == vec[1, 2, 3]);
    assert!($original == vec[1]);
    assert!(FrameGrowthArgument::$drops == $depth + 1);
}

assert!(attempt_overflow($callable, $original));
assert!(FrameGrowthArgument::$drops == 34);
assert!($original == vec[1]);
assert!(descend(0, $callable, new FrameGrowthArgument(2), $original) == vec[1, 2, 3]);
assert!(FrameGrowthArgument::$drops == 35);
assert!($original == vec[1]);
";
        let result = engine.run_source(source, Path::new("/function-value-frame-growth.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
