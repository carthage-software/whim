use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn counted_loops_preserve_copies_types_and_faults() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function copy(dict<int, int> $source, dict<int, int> $target, int $count): dict<int, int> {
    for ($index = $count - 1; $index >= 0; $index--) {
        $target[$index] = $source[$index];
    }
    return $target;
}
#[NeverInline]
function increment(int $value): int {
    for ($index = 0; $index < 8; $index++) {
        $value++;
    }
    return $value;
}
#[NeverInline]
function unsigned_increment(uint $value): uint {
    for ($index = 0; $index < 8; $index++) {
        $value++;
    }
    return $value;
}
#[NeverInline]
function changing_limit(int $limit): int {
    $sum = 0;
    for ($index = 0; $index < $limit; $index = $index + 1) {
        $limit--;
        $sum += ($index * $index + $index) % 17;
    }
    return $sum;
}
#[NeverInline]
function nested_copy(dict<int, int> $source, int $count): int {
    $target = dict[0 => 0];
    $sum = 0;
    for ($round = 0; $round < 3; $round++) {
        $sum += $target[0];
        for ($index = $count - 1; $index >= 0; $index--) {
            $target[$index] = $source[$index];
        }
        $sum += $target[$count - 1];
    }
    return $sum;
}
#[NeverInline]
function accumulate(dict<int, int> $source, dict<int, int> $target, int $count): (dict<int, int>, int) {
    $index = $count - 1;
    try {
        for (; $index >= 0; $index--) {
            $target[$index] += $source[$index];
        }
    } catch (Whim\Unwind\ArithmeticError $_) {}
    return ($target, $index);
}
$original = dict[0 => 10, 1 => 20, 2 => 30, 3 => 40];
assert!(copy($original, dict[], 0) == dict[]);
assert!(copy($original, dict[], 1) == dict[0 => 10]);
$copied = copy($original, $original, 4);
$copied[0] = 99;
assert!($original[0] == 10);
assert!($copied == dict[0 => 99, 1 => 20, 2 => 30, 3 => 40]);
assert!(copy(dict[3 => 40, 2 => 30, 1 => 20, 0 => 10], dict[], 4)
    == dict[3 => 40, 2 => 30, 1 => 20, 0 => 10]);
$caught = false;
try {
    copy(dict[0 => 10, 2 => 30, 3 => 40], dict[], 4);
} catch (Whim\Unwind\OutOfBoundsError $_) {
    $caught = true;
}
assert!($caught);
assert!(increment(-12) == -4);
assert!(unsigned_increment(12u) == 20u);
assert!(changing_limit(10) == 23);
assert!(nested_copy($original, 4) == 140);
assert!(accumulate($original, $original, 4)
    == (dict[0 => 20, 1 => 40, 2 => 60, 3 => 80], -1));
assert!($original == dict[0 => 10, 1 => 20, 2 => 30, 3 => 40]);
assert!(accumulate($original, dict[0 => 10, 1 => 9223372036854775797, 2 => 30, 3 => 40], 4)
    == (dict[0 => 10, 1 => 9223372036854775797, 2 => 60, 3 => 80], 1));
$caught = false;
try {
    increment(9223372036854775804);
} catch (Whim\Unwind\OverflowError $_) {
    $caught = true;
}
assert!($caught);
";
    run_both_modes(source);
}

#[test]
fn integer_bursts_preserve_boundaries_and_partial_updates() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function less(int $start, int $limit, int $sum): (int, int, int, bool) {
    $index = $start;
    $temporary = 0;
    $caught = false;
    try {
        for (; $index < $limit; $index++) {
            $sum++;
            $temporary += 2;
        }
    } catch (Whim\Unwind\OverflowError $_) { $caught = true; }
    return ($index, $sum, $temporary, $caught);
}
#[NeverInline]
function less_equal(int $start, int $limit, int $sum): (int, int, int, bool) {
    $index = $start;
    $temporary = 0;
    $caught = false;
    try {
        for (; $index <= $limit; $index++) {
            $sum++;
            $temporary += 2;
        }
    } catch (Whim\Unwind\OverflowError $_) { $caught = true; }
    return ($index, $sum, $temporary, $caught);
}
assert!(less(0, 0, 0) == (0, 0, 0, false));
assert!(less(0, 1, 0) == (1, 1, 2, false));
assert!(less(-9223372036854775808, -9223372036854775806, 0)
    == (-9223372036854775806, 2, 4, false));
assert!(less(9223372036854775805, 9223372036854775807, 0)
    == (9223372036854775807, 2, 4, false));
assert!(less_equal(9223372036854775805, 9223372036854775807, 0)
    == (9223372036854775807, 3, 6, true));
assert!(less_equal(9223372036854775807, 9223372036854775807, 0)
    == (9223372036854775807, 1, 2, true));
assert!(less(4, 10, 9223372036854775806)
    == (5, 9223372036854775807, 2, true));
assert!(less(0, 65537, 0) == (65537, 65537, 131074, false));
assert!(less_equal(0, 65536, 0) == (65537, 65537, 131074, false));
";
    run_both_modes(source);
}

#[test]
fn numeric_regions_keep_destructor_boundaries() {
    let source = r"
use Whim\Marker\NeverInline;
final class Released {
    public static vec<int> $events = vec[];
    public function __construct(private int $index) {}
    public function __destruct(): void {
        self::$events[] = $this->index;
        if ($this->index == 1) { throw new Whim\Unwind\Exception('released'); }
    }
}
#[NeverInline]
function released_values(): dict<int, Released> {
    return dict[
        0 => new Released(0), 1 => new Released(1),
        2 => new Released(2), 3 => new Released(3),
    ];
}
#[NeverInline]
function release_by_copy(dict<int, int> $source, int $count, int $limit): (int, bool, vec<int>) {
    $target = released_values();
    $index = $count - 1;
    try {
        for (; $index >= $limit; $index--) {
            $target[$index] = $source[$index];
        }
    } catch (Whim\Unwind\Exception $_) {}
    return ($index, $target[0] is Released, Released::$events);
}
assert!(release_by_copy(dict[0 => 0, 1 => 10, 2 => 20, 3 => 30], 4, 0)
    == (1, true, vec[3, 2, 1]));
assert!(Released::$events == vec[3, 2, 1, 0]);
";
    run_both_modes(source);
}

fn run_both_modes(source: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/numeric-loops.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
