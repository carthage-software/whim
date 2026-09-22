use std::path::Path;

use whim_runtime::artifact::ArtifactConfiguration;
use whim_runtime::artifact::SourceFile;
use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

const SOURCE: &str =
    include_str!("../../../tests/standard-library/reflection-where-constraints.whim");

#[test]
fn where_constraints_reflect_from_source_and_artifacts() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(SOURCE, Path::new("/reflection/where.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");

        let mut compiler = Engine::new(EngineConfiguration::default());
        let artifact = compiler
            .compile_artifact(
                "/reflection/where.whia",
                &[SourceFile::new("/reflection/where.whim", SOURCE)],
                ArtifactConfiguration {
                    optimize,
                    ..ArtifactConfiguration::default()
                },
            )
            .unwrap()
            .into_bytes();
        let mut loader = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        loader.load_artifact(&artifact).unwrap();
    }
}

#[test]
fn where_bounds_expand_aliases_from_other_artifact_sources() {
    for optimize in [false, true] {
        let mut compiler = Engine::new(EngineConfiguration::default());
        let artifact = compiler
            .compile_artifact(
                "/reflection/aliases.whia",
                &[
                    SourceFile::new(
                        "/reflection/method.whim",
                        r"namespace Methods;
use Bounds\Sequence as Items;
class Box<T> { public function inspect<U>(): void where U: Items<T> {} }",
                    ),
                    SourceFile::new(
                        "/reflection/bound.whim",
                        r"namespace Bounds; type Sequence<T> = vec<T>;",
                    ),
                    SourceFile::new(
                        "/reflection/check.whim",
                        r"use Whim\Reflection;
$method = Reflection\reflect_class('Methods\\Box')->getMethod('inspect');
$bound = $method->getWhereConstraints()[0]->getBound();
assert!($bound->toString() == 'vec<T>');
$environment = Reflection\reflect_object(new Methods\Box::<int>())->getTypeEnvironment();
assert!($bound->resolve($environment)->toString() == 'vec<int>');",
                    ),
                ],
                ArtifactConfiguration {
                    optimize,
                    ..ArtifactConfiguration::default()
                },
            )
            .unwrap()
            .into_bytes();
        let mut loader = Engine::new(EngineConfiguration::default());
        loader.load_artifact(&artifact).unwrap();
    }
}
