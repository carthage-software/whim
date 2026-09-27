use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn scalar_alias_proofs_preserve_bounds_and_autoloads() {
    let source = include_str!("../../../tests/_fixtures/scalar-alias-proofs.whim");
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/scalar-alias-proofs.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
