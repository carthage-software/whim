use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn loaded_files_retain_source_metadata_across_optimization() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        for (path, source) in [
            ("/reflection/empty.whim", ""),
            ("/reflection/namespace.whim", "namespace Empty {}"),
            (
                "/reflection/code.whim",
                "namespace Executable; if (false) { write_line!('unreachable'); }",
            ),
            (
                "/reflection/declarations.whim",
                r"
namespace First {
    class Box { public function value(): int { return 1; } }
    interface Marker {}
    enum State { case Ready; }
    type Alias = string;
    newtype Id = int;
    const VALUE = 1;
    function make(): fn(): int { return fn(): int => 1; }
}
namespace Second { function helper(): int { return 2; } }
",
            ),
        ] {
            let result = engine.run_source(source, Path::new(path));
            assert_eq!(
                result.exit_code(),
                0,
                "{path}, optimization {optimize}: {result:?}"
            );
        }
        let result = engine.run_source(
            r"
use Whim\Reflection;

$file = Reflection\reflect_file('/reflection/declarations.whim');
assert!($file is Reflection\FileReflection);
assert!($file->getPath() == '/reflection/declarations.whim');
assert!(!$file->hasTopLevelCode());
assert!($file->getOrigin() == Reflection\DeclarationOrigin::User);
$names = vec[];
foreach ($file->getSymbols() as $symbol) {
    $names[] = $symbol->getName();
    assert!($symbol->getFile()->getPath() == $file->getPath());
    assert!($symbol->getFile()->getOrigin() == $symbol->getOrigin());
}
assert!($names == vec[
    'First\\Alias', 'First\\Box', 'First\\Id', 'First\\Marker',
    'First\\State', 'First\\VALUE', 'First\\make', 'Second\\helper',
]);
assert!(length!($file->getSymbols(null)) == 8);
assert!(Reflection\reflect_file('/reflection/code.whim')->hasTopLevelCode());
assert!(Reflection\reflect_file('/reflection/code.whim')->getSymbols() == vec[]);
assert!(!Reflection\reflect_file('/reflection/empty.whim')->hasTopLevelCode());
assert!(Reflection\reflect_file('/reflection/empty.whim')->getSymbols() == vec[]);
assert!(!Reflection\reflect_file('/reflection/namespace.whim')->hasTopLevelCode());
assert!(Reflection\reflect_file('/reflection/missing.whim') == null);
$paths = vec[];
foreach (Reflection\get_loaded_files() as $loaded) { $paths[] = $loaded->getPath(); }
assert!($paths == vec[
    '/reflection/check.whim', '/reflection/code.whim',
    '/reflection/declarations.whim', '/reflection/empty.whim', '/reflection/namespace.whim',
]);
",
            Path::new("/reflection/check.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn reloaded_paths_reflect_the_latest_source_once() {
    let mut engine = Engine::new(EngineConfiguration::default());
    for source in ["class PreviousLoad {}", "1 + 2;"] {
        let result = engine.run_source(source, Path::new("/reflection/reloaded.whim"));
        assert_eq!(result.exit_code(), 0, "{result:?}");
    }
    let result = engine.run_source(
        r"
use Whim\Reflection;
assert!(Reflection\reflect_file('/reflection/reloaded.whim')->hasTopLevelCode());
assert!(Reflection\reflect_file('/reflection/reloaded.whim')->getSymbols() == vec[]);
assert!(!Reflection\reflect_class('PreviousLoad')->getFile()->hasTopLevelCode());
assert!(length!(Reflection\reflect_class('PreviousLoad')->getFile()->getSymbols()) == 1);
assert!(length!(Reflection\get_loaded_files()) == 2);
",
        Path::new("/reflection/check.whim"),
    );
    assert_eq!(result.exit_code(), 0, "{result:?}");
}

#[test]
fn anonymous_sources_keep_distinct_files_and_symbol_ownership() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let result = engine.run_source("class FirstAnonymous {}", Path::new("-"));
    assert_eq!(result.exit_code(), 0, "{result:?}");
    let result = engine.run_source(
        r"
use Whim\Reflection;
class SecondAnonymous {}
$first = Reflection\reflect_class('FirstAnonymous')->getFile();
$second = Reflection\reflect_class('SecondAnonymous')->getFile();
assert!($first is Reflection\FileReflection);
assert!($second is Reflection\FileReflection);
assert!($first->getPath() == null);
assert!($second->getPath() == null);
assert!(!$first->hasTopLevelCode());
assert!($second->hasTopLevelCode());
assert!($first->getOrigin() == Reflection\DeclarationOrigin::User);
assert!($first->getSymbols()[0]->getName() == 'FirstAnonymous');
assert!($second->getSymbols()[0]->getName() == 'SecondAnonymous');
assert!(Reflection\reflect_class('Whim\\Async\\TaskLocal')->getFile() == null);
assert!(Reflection\reflect_file('-') == null);
assert!(length!(Reflection\get_loaded_files()) == 2);
",
        Path::new("-"),
    );
    assert_eq!(result.exit_code(), 0, "{result:?}");
}
