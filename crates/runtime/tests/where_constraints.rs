use std::path::Path;

use whim_runtime::artifact::ArtifactConfiguration;
use whim_runtime::artifact::SourceFile;
use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

const SOURCE: &str =
    include_str!("../../../tests/standard-library/reflection-where-constraints.whim");

#[test]
fn where_constraints_reflect_from_source_and_artifacts() {
    run_source_and_artifacts(SOURCE);
}

#[test]
fn where_constraints_enforce_from_source_and_artifacts() {
    run_source_and_artifacts(include_str!(
        "../../../tests/language/method-where-constraints.whim"
    ));
}

fn run_source_and_artifacts(source: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/reflection/where.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");

        let mut compiler = Engine::new(EngineConfiguration::default());
        let artifact = compiler
            .compile_artifact(
                "/reflection/where.whia",
                &[SourceFile::new("/reflection/where.whim", source)],
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

#[test]
fn overrides_cannot_strengthen_where_constraints() {
    for source in [
        "class Base<T> { public function check(): void {} }
         class Child<T> extends Base<T> { public function check(): void where T: int {} }",
        "class Base<T> { public function check(): void where T: int|float {} }
         class Child<T> extends Base<T> { public function check(): void where T: int {} }",
        "interface Base<T> { public function check<U>(): void where U: vec<T>; }
         class Child<T> implements Base<T> { public function check<V>(): void where V: vec<int> {} }",
        "class Base { public static function check<T>(): void {} }
         class Child extends Base { public static function check<U>(): void where U: int {} }",
        "class Base<T> { public function check<U>(): void where T: int, U: int {} }
         class Child<T> extends Base<T> { public function check<V>(): void where V: T {} }",
        "class Base<T> { public function check<U>(): void where T: U {} }
         class Child<T> extends Base<T> { public function check<V>(): void where V: T {} }",
        "interface Base<T> { public function __construct() where T: int|float; }
         class Child<T> implements Base<T> { public function __construct() where T: int {} }",
    ] {
        for optimize in [false, true] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                ..EngineConfiguration::default()
            });
            let result = engine.run_source(source, Path::new("/invalid-where-override.whim"));
            assert_ne!(result.exit_code(), 0, "{source}");
            let diagnostic = format!("{result:?}");
            assert!(diagnostic.contains("LinkerError"), "{diagnostic}");
            assert!(diagnostic.contains("stronger than the inherited contract"), "{diagnostic}");
        }
    }
}
