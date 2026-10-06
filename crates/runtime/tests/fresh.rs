use std::path::Path;

use whim_runtime::artifact::ArtifactConfiguration;
use whim_runtime::artifact::SourceFile;
use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

const IDENTITIES: &str = r"
use Whim\Marker\NeverInline;
type Identity = fresh;
newtype Ticket = fresh;
newtype OtherTicket = fresh;
#[NeverInline]
function identity<T: fresh>(T $value): T { return $value; }
#[NeverInline]
function generate(): fresh { return fresh!(); }
$a = fresh!();
$copy = $a;
$b = generate();
assert!($a is fresh);
assert!($a is Identity);
assert!($a == $copy);
assert!($a != $b);
assert!(identity::<fresh>($a) == $a);
assert!($a != ($a as uint));
assert!(($copy as uint) == ($a as uint));
assert!(($b as uint) != ($a as uint));
assert!(($a as fresh) == $a);
assert!(($a ?as int) == null);
assert!(($a ?as float) == null);
assert!(($a ?as string) == null);
assert!((($a as uint) ?as fresh) == null);
foreach (vec[0, 0u, false, null, 'fresh', vec[], dict[]] as $value) {
    assert!(($value ?as fresh) == null);
}
type Unsigned = uint;
assert!(($a as Unsigned) == ($a as uint));
assert!(($a as (int|uint)) == ($a as uint));
assert!(($a as (fresh|uint)) == $a);
$closure = fn(): fresh => $a;
assert!($closure() == $a);
$values = vec[$a, $b];
$saved = $values;
$values[0] = fresh!();
assert!($saved[0] == $a);
assert!($values[0] != $a);
assert!(($a, $b) == ($copy, $b));
$keys = dict[$a => 1, $b => 2, ($a as uint) => 3,
    Ticket($a) => 4, OtherTicket($a) => 5];
assert!(length!($keys) == 5u);
assert!($keys[$copy] == 1);
assert!($keys[$a as uint] == 3);
assert!($keys[Ticket($copy)] == 4);
assert!($keys[OtherTicket($copy)] == 5);
$keys[$a] += 5;
assert!($keys[$copy] == 6);
assert!(remove!($keys, $b) == 2);
assert!(!contains_key!($keys, $b));
$seen = dict[];
for ($i = 0; $i < 1000; $i++) {
    $key = fresh!();
    assert!(!contains_key!($seen, $key));
    $seen[$key] = $i;
}
assert!(length!($seen) == 1000u);
assert!($seen is dict<fresh, int>);
function keys(dict $values): void {
    foreach ($values as $key => $_) {
        assert!($key is fresh);
        assert!(!($key is uint));
    }
}
keys($seen);
";

const REFLECTION: &str = include_str!("../../../tests/standard-library/reflection-fresh.whim");

#[test]
fn fresh_reflection_preserves_types_and_values() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let outcome = engine.run_source(REFLECTION, Path::new("/fresh-reflection.whim"));
        assert_eq!(
            outcome.exit_code(),
            0,
            "optimization {optimize}: {outcome:?}"
        );
    }
}

#[test]
fn artifacts_preserve_fresh_reflection() {
    for optimize in [false, true] {
        let mut compiler = Engine::new(EngineConfiguration::default());
        let artifact = compiler
            .compile_artifact(
                "/fresh-reflection.whim",
                &[SourceFile::new("/fresh-reflection.whim", REFLECTION)],
                ArtifactConfiguration {
                    optimize,
                    ..ArtifactConfiguration::default()
                },
            )
            .unwrap()
            .into_bytes();
        let mut engine = Engine::new(EngineConfiguration::default());
        engine.load_artifact(&artifact).unwrap();
    }
}

#[test]
fn fresh_identities_survive_copies_casts_and_dictionary_operations() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let outcome = engine.run_source(IDENTITIES, Path::new("/fresh.whim"));
        assert_eq!(
            outcome.exit_code(),
            0,
            "optimization {optimize}: {outcome:?}"
        );
    }
}

#[test]
fn artifacts_generate_identities_at_execution_and_share_the_engine_counter() {
    for optimize in [false, true] {
        let mut compiler = Engine::new(EngineConfiguration::default());
        let source = format!("$first = fresh!(); assert!($first != Saved::$token);\n{IDENTITIES}");
        let artifact = compiler
            .compile_artifact(
                "/fresh-artifact.whim",
                &[SourceFile::new("/fresh-artifact.whim", &source)],
                ArtifactConfiguration {
                    optimize,
                    ..ArtifactConfiguration::default()
                },
            )
            .unwrap()
            .into_bytes();
        let mut engine = Engine::new(EngineConfiguration::default());
        let first = engine.run_source(
            "class Saved { public static null|fresh $token = null; } Saved::$token = fresh!();",
            Path::new("/first.whim"),
        );
        assert_eq!(first.exit_code(), 0, "{first:?}");
        engine.load_artifact(&artifact).unwrap();
        let last = engine.run_source(
            "assert!(fresh!() != Saved::$token);",
            Path::new("/last.whim"),
        );
        assert_eq!(last.exit_code(), 0, "{last:?}");
    }
}

#[test]
fn indirect_constant_generation_fails() {
    for optimize in [false, true] {
        for source in [
            "function generate(): fresh { return fresh!(); } const TOKEN = generate(); TOKEN;",
            "function generate(): fresh { return fresh!(); } class C { public const fresh TOKEN = generate(); } C::TOKEN;",
            "function generate(): null {
                 Whim\\_Private\\defer_task(fn(): void { fresh!(); });
                 Whim\\Async\\drain(); return null;
             } const TOKEN = generate(); TOKEN;",
        ] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                ..EngineConfiguration::default()
            });
            let outcome = engine.run_source(source, Path::new("/fresh-constant.whim"));
            assert_ne!(
                outcome.exit_code(),
                0,
                "optimization {optimize}: {outcome:?}"
            );
            assert!(
                format!("{outcome:?}").contains("fresh!() cannot run in a constant expression"),
                "{outcome:?}"
            );
            let recovery = engine.run_source("fresh!();", Path::new("/fresh-recovery.whim"));
            assert_eq!(recovery.exit_code(), 0, "{recovery:?}");
        }
    }
}
