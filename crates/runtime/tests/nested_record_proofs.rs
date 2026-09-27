use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn nested_record_proofs_preserve_values_tags_and_mutation_checks() {
    let source = include_str!("../../../tests/_fixtures/nested-record-proofs.whim");
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/nested-record-proofs.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn repeated_shape_checks_keep_generic_alias_and_bound_autoloads() {
    for (shape, value) in [
        ("dict['item' => Holder<Missing>]", "new Holder::<never>()"),
        (
            "dict['item' => Defaulted<int>]",
            "new Defaulted::<int, never>()",
        ),
        ("dict['item' => Bounded<int>]", "new Bounded::<int>()"),
        ("dict['item' => BoundAlias<int>]", "1"),
    ] {
        let source = format!(
            r"
final class Holder<out T> {{}}
final class Defaulted<out T, out U = Missing> {{}}
final class Bounded<out T: Missing|int> {{}}
type DefaultAlias<T = Missing> = int;
type BoundAlias<T: Missing|int> = T;
type Erase<T> = int;
type Hidden = Erase<Missing>;
class Loads {{ public static int $count = 0; }}
#[Whim\Marker\NeverInline]
function accept({shape} $row): void {{}}
#[Whim\Marker\NeverInline]
function forward({shape} $row): void {{
    $before = Loads::$count;
    accept($row);
    assert!(Loads::$count > $before);
}}
Whim\_Private\register_symbol_autoloader(fn(int $kind, string $name): void {{
    if ($name == 'Missing') {{ Loads::$count++; }}
}});
$row = dict['item' => {value}];
forward($row);
forward($row);
"
        );
        for optimize in [false, true] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                ..EngineConfiguration::default()
            });
            let result = engine.run_source(&source, Path::new("/shape-autoloads.whim"));
            assert_eq!(
                result.exit_code(),
                0,
                "{shape}, optimization {optimize}: {result:?}"
            );
        }
    }
}
