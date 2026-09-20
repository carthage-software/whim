mod arithmetic;
mod matching;
mod returns;

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
        try { byte_value($text, length!($text)); }
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
