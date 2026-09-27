mod argument_guards;
mod arithmetic;
mod callable_collections;
mod declaration_optimization;
mod final_class_checks;
mod matching;
mod nominal_refinement;
mod object_shape_checks;
mod returns;
mod rope_append;

use std::path::Path;

use whim_value::function::FuncId;

use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::symbols::ExactFunctionEntry;

fn run_both_modes(source: &str, path: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new(path));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn lengths_preserve_unsigned_types() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function dynamic_length(mixed $value): uint { return length!($value); }
#[NeverInline]
function string_length(string $value): uint { return length!($value); }
#[NeverInline]
function invalid_length(mixed $value): int { return length!($value); }
assert!(length!('abc') == 3u);
assert!(length!(vec[]) == 0u);
assert!(length!((1, 2)) is uint);
assert!(!(length!(dict['a' => 1]) is int));
assert!(dynamic_length('abc') == 3u);
assert!(dynamic_length('a string longer than the inline capacity') == 40u);
assert!(dynamic_length(vec[1, 2]) == 2u);
assert!(dynamic_length(dict['a' => 1]) == 1u);
assert!(dynamic_length((1, 2, 3)) == 3u);
assert!(string_length('abc') == 3u);
$caught = false;
try { invalid_length(vec[]); } catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
",
        "/unsigned-lengths.whim",
    );
}

#[test]
fn repeated_tuple_reads_preserve_reassignment_and_copy_on_write() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function repeated((int, int) $pair, int $count): int {
    $result = 0;
    for ($index = 0; $index < $count; $index++) {
        $result += $pair[0] + $pair[1] + $pair[0];
    }
    return $result;
}
#[NeverInline]
function reassigned((int, int) $pair, (int, int) $other, bool $replace): int {
    $first = $pair[0];
    if ($replace) { $pair = $other; }
    return $first + $pair[0];
}
#[NeverInline]
function copied((vec<int>, int) $pair, bool $change_first): int {
    $first = $pair[0];
    $second = $pair[0];
    if ($change_first) { $first[] = 3; } else { $second[] = 3; }
    return (length!($first) as int) * 100 + (length!($second) as int);
}
#[NeverInline]
function reused((vec<int>, int) $pair): int {
    $first = $pair[0];
    $size = length!($first) as int;
    $first = $pair[0];
    $first[] = 3;
    return $size * 100 + (length!($first) as int);
}
assert!(repeated((3, 4), 100) == 1000);
assert!(reassigned((3, 4), (5, 6), false) == 6);
assert!(reassigned((3, 4), (5, 6), true) == 8);
$pair = (vec[1, 2], 0);
assert!(copied($pair, false) == 203);
assert!(copied($pair, true) == 302);
assert!(reused($pair) == 203);
assert!($pair == (vec[1, 2], 0));
",
        "/tuple-reads.whim",
    );
}

#[test]
fn bounded_comparisons_preserve_integer_edges() {
    run_both_modes(
        include_str!("../../../../tests/_fixtures/comparison-ranges.whim"),
        "/comparison-ranges.whim",
    );
}

#[test]
fn direct_named_calls_preserve_borrowed_arguments() {
    run_both_modes(
        include_str!("../../../../tests/_fixtures/direct-named-calls.whim"),
        "/direct-named-calls.whim",
    );
}

#[test]
fn collection_unions_preserve_element_checks() {
    run_both_modes(
        include_str!("../../../../tests/_fixtures/collection-contracts.whim"),
        "/collection-tests.whim",
    );
}

#[test]
fn collection_elements_keep_class_and_return_contracts() {
    run_both_modes(
        include_str!("../../../../tests/_fixtures/element-contracts.whim"),
        "/element-contracts.whim",
    );
}

#[test]
fn byte_wrappers_keep_bounds_and_error_frames() {
    let source = r#"
#[Whim\Marker\NeverInline]
#[Whim\Marker\TrackCaller]
function byte_value(string $text, 0.. $offset): 0..=255 {
    return Whim\Str\byte_at($text, $offset);
}
#[Whim\Marker\NeverInline]
function reversed_byte(0.. $offset, string $text): 0..=255 {
    return Whim\Str\byte_at($text, $offset);
}
#[Whim\Marker\NeverInline]
function restricted_byte(string $text, 0.. $offset): 0..=127 {
    return Whim\Str\byte_at($text, $offset);
}
for ($round = 0; $round < 4; $round++) {
    foreach (vec['a', 'a long string', "\0\xff\x80\x7f"] as $text) {
        for ($offset = 0; $offset < length!($text); $offset++) {
            $expected = Whim\Str\byte_at($text, $offset);
            assert!(byte_value($text, $offset) == $expected);
            assert!(reversed_byte($offset, $text) == $expected);
        }
        $caught = false;
        try { byte_value($text, length!($text) as int); }
        catch (Whim\Unwind\OutOfBoundsError $error) {
            assert!($error->getTrace()[0]->function == 'Whim\Str\byte_at');
            assert!($error->getTrace()[1]->function == 'byte_value');
            $caught = true;
        }
        assert!($caught);
    }
    $caught = false;
    try { restricted_byte("\xff", 0); }
    catch (Whim\Unwind\TypeError $error) { $caught = true; }
    assert!($caught);
}
"#;
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..Default::default()
        });
        let result = engine.run_source(source, Path::new("/byte-wrappers.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        if optimize {
            let (position, function) = engine
                .tables
                .functions
                .iter()
                .enumerate()
                .find(|(_, function)| function.name.as_bytes() == b"byte_value")
                .unwrap();
            assert_eq!(
                ExactFunctionEntry::from_runtime(FuncId(position as u32), function, true)
                    .string_byte_at,
                Some((0, 1))
            );
        }
    }
}
