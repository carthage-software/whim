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
