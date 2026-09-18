use std::env;
use std::fs;
use std::io::ErrorKind;
use std::process;

use whim_runtime::artifact::ArtifactConfiguration;
use whim_runtime::artifact::SourceFile;
use whim_runtime::compiler::target::Target;
use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

fn compile(source: &str, path: &str) -> Vec<u8> {
    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .compile_artifact(
            path,
            &[SourceFile::new(path, source)],
            ArtifactConfiguration::default(),
        )
        .expect("the artifact compiles")
        .into_bytes()
}

#[test]
fn artifacts_load_in_order_and_execute_their_top_level_code() {
    let declarations = compile(
        "function artifact_answer(): int { return 42; }",
        "/artifact/declarations.whim",
    );
    let entry = compile("assert!(artifact_answer() == 42);", "/artifact/entry.whim");

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&declarations)
        .expect("the declaration artifact loads");
    engine
        .load_artifact(&entry)
        .expect("the entry artifact resolves the earlier declaration");
}

#[test]
fn artifact_file_reflection_preserves_each_source_file() {
    for optimize in [false, true] {
        let mut compiler = Engine::new(EngineConfiguration::default());
        let artifact = compiler
            .compile_artifact(
                "/reflection/bundle.whim",
                &[
                    SourceFile::new(
                        "/reflection/declarations.whim",
                        "namespace ArtifactFile; function value(): int { return 42; }",
                    ),
                    SourceFile::new("/reflection/empty.whim", ""),
                    SourceFile::new(
                        "/reflection/unreachable.whim",
                        "if (false) { write_line!('unreachable'); }",
                    ),
                    SourceFile::new(
                        "/reflection/entry.whim",
                        r"
use Whim\Reflection;
$file = Reflection\reflect_file('/reflection/declarations.whim');
assert!($file->getPath() == '/reflection/declarations.whim');
assert!($file->getOrigin() == Reflection\DeclarationOrigin::Extension);
assert!(!$file->hasTopLevelCode());
assert!(length!($file->getSymbols()) == 1);
assert!($file->getSymbols()[0]->getName() == 'ArtifactFile\\value');
assert!($file->getSymbols()[0]->getFile()->getPath() == $file->getPath());
assert!($file->getSymbols()[0]->getFile()->getOrigin() == $file->getOrigin());
assert!(Reflection\reflect_file('/reflection/empty.whim')->getSymbols() == vec[]);
assert!(!Reflection\reflect_file('/reflection/empty.whim')->hasTopLevelCode());
assert!(Reflection\reflect_file('/reflection/unreachable.whim')->hasTopLevelCode());
assert!(Reflection\reflect_file('/reflection/entry.whim')->hasTopLevelCode());
assert!(Reflection\reflect_file('/reflection/entry.whim')->getSymbols() == vec[]);
assert!(Reflection\reflect_file('/reflection/bundle.whim') == null);
assert!(length!(Reflection\get_loaded_files()) == 4);
",
                    ),
                ],
                ArtifactConfiguration {
                    optimize,
                    ..ArtifactConfiguration::default()
                },
            )
            .expect("the artifact compiles")
            .into_bytes();
        let mut engine = Engine::new(EngineConfiguration::default());
        engine
            .load_artifact(&artifact)
            .expect("file reflection works in the loaded artifact");
    }
}

#[test]
fn artifact_atoms_outlive_the_encoded_input() {
    let mut declarations = compile(
        "function artifact_owned_string(): string { return \"retained\\x00artifact\\xffbytes\"; }",
        "/artifact/owned-atoms.whim",
    );
    let entry = compile(
        "assert!(artifact_owned_string() == \"retained\\x00artifact\\xffbytes\");",
        "/artifact/owned-atoms-entry.whim",
    );

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&declarations)
        .expect("the declaration artifact loads");
    declarations.fill(0);
    drop(declarations);

    engine
        .load_artifact(&entry)
        .expect("decoded names and string literals retain independent storage");
}

#[test]
fn artifact_file_attributes_remain_separate_and_keep_initializer_thunks() {
    for optimize in [false, true] {
        let mut compiler = Engine::new(EngineConfiguration::default());
        let artifact = compiler
            .compile_artifact(
                "/attributes/bundle.whim",
                &[
                    SourceFile::new(
                        "/attributes/first.whim",
                        r"
namespace Metadata;
use Whim\Attribute\Attribute;
#[Attribute(Attribute::TARGET_FILE)]
class Tag { public function __construct(public string $name, public mixed $value = null) {} }
#![Tag('first', fn(): int => 42)]
",
                    ),
                    SourceFile::new(
                        "/attributes/second.whim",
                        r"
use Metadata\Tag as Label;
#![Label(name: 'second' . ' file', value: vec[1, 2])]
",
                    ),
                    SourceFile::new(
                        "/attributes/check.whim",
                        r"
use Whim\Reflection;
$first = Reflection\reflect_class('Metadata\\Tag')->getFile();
$second = Reflection\reflect_file('/attributes/second.whim');
assert!(!$first->hasTopLevelCode());
assert!(!$second->hasTopLevelCode());
assert!($first->getOrigin() == Reflection\DeclarationOrigin::Extension);
assert!($first->getLocation() == null);
assert!(length!($first->getAttributes()) == 1);
assert!(length!($second->getAttributes()) == 1);
$firstAttribute = $first->getAttributes()[0];
assert!($firstAttribute->newInstance()->name == 'first');
$callback = $firstAttribute->newInstance()->value;
assert!($callback() == 42);
assert!($firstAttribute->getTarget()->getPath() == '/attributes/first.whim');
assert!($firstAttribute->getLocation() == null);
$secondAttribute = $second->getAttributes()[0];
assert!($secondAttribute->newInstance()->name == 'second file');
assert!($secondAttribute->newInstance()->value == vec[1, 2]);
assert!($secondAttribute->getTarget()->getPath() == '/attributes/second.whim');
",
                    ),
                ],
                ArtifactConfiguration {
                    optimize,
                    ..ArtifactConfiguration::default()
                },
            )
            .expect("file attributes compile into the artifact")
            .into_bytes();
        let mut engine = Engine::new(EngineConfiguration::default());
        engine
            .load_artifact(&artifact)
            .expect("file attributes reflect from the artifact");
    }
}

#[test]
fn artifact_loading_rejects_trailing_bytes() {
    let mut artifact = compile("", "/artifact/empty.whim");
    artifact.push(0);

    let mut engine = Engine::new(EngineConfiguration::default());
    let error = engine
        .load_artifact(&artifact)
        .expect_err("trailing bytes are rejected");
    assert!(error.to_string().contains("trailing bytes"));
}

#[test]
fn artifact_loading_reports_truncated_atom_payloads() {
    let mut artifact = compile("", "/artifact/truncated-atom.whim");
    let bytecode_length = u64::from_le_bytes(
        artifact[20..28]
            .try_into()
            .expect("the header contains the bytecode length"),
    );
    let bytecode_start = artifact.len()
        - usize::try_from(bytecode_length).expect("the bytecode length fits in memory");
    // The first bytecode field is the unit path: keep its length and one byte.
    artifact.truncate(bytecode_start + 9);
    artifact[20..28].copy_from_slice(&9_u64.to_le_bytes());

    let mut engine = Engine::new(EngineConfiguration::default());
    let error = engine
        .load_artifact(&artifact)
        .expect_err("a truncated atom must be rejected");
    assert!(
        error
            .to_string()
            .contains("artifact bytecode is invalid: io error: unexpected end of file")
    );
}

#[test]
fn artifact_loading_rejects_impossible_source_file_counts() {
    let mut artifact = compile("null;", "/artifact/impossible-count.whim");
    artifact[28..32].copy_from_slice(&u32::MAX.to_le_bytes());

    let mut engine = Engine::new(EngineConfiguration::default());
    let error = engine
        .load_artifact(&artifact)
        .expect_err("an impossible source-file count must be rejected");

    assert!(
        error
            .to_string()
            .contains("artifact source-file count exceeds its metadata")
    );
}

#[test]
fn verified_artifacts_load_and_execute() {
    let artifact = compile("assert!(40 + 2 == 42);", "/artifact/verified.whim");

    let mut engine = Engine::new(EngineConfiguration::default());
    // SAFETY: `compile` returned these bytes unchanged from this runtime build.
    unsafe { engine.load_verified_artifact(&artifact) }
        .expect("the verified artifact loads and executes");
}

#[test]
fn artifact_compilation_validates_stub_declarations() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let Err(error) = engine.compile_artifact(
        "/artifact/stub.whim",
        &[SourceFile::new(
            "/artifact/stub.whim",
            "namespace Example; use Whim\\Marker\\Stub; #[Stub] function missing(): void {}",
        )],
        ArtifactConfiguration::default(),
    ) else {
        panic!("a missing stub provider is rejected");
    };
    assert!(error.to_string().contains("Example\\missing"));
}

#[test]
fn artifact_stubs_preserve_core_reflection_when_cross_compiling() {
    for target in [None, Some(Target::NATIVE)] {
        let mut compiler = Engine::new(EngineConfiguration::default());
        let artifact = compiler
            .compile_artifact(
                "/artifact/core-stubs.whim",
                &[SourceFile::new(
                    "/artifact/core-stubs.whim",
                    r"
namespace Whim\Type;
use Whim\Marker\Stub;
#[Stub]
newtype TypeId = 0..;
namespace Whim\Math;
use Whim\Marker\Stub;
use Whim\Reflection;
#[Stub]
const PI = PI;
foreach (vec[
    Reflection\reflect_newtype('Whim\\Type\\TypeId'),
    Reflection\reflect_constant('Whim\\Math\\PI'),
] as $symbol) {
    assert!($symbol->getOrigin() == Reflection\DeclarationOrigin::Core);
    assert!($symbol->getLocation() == null);
    assert!($symbol->getFile() == null);
    assert!($symbol->getAttributes() == vec[]);
}
",
                )],
                ArtifactConfiguration {
                    target,
                    ..ArtifactConfiguration::default()
                },
            )
            .expect("the native stubs compile")
            .into_bytes();
        let mut engine = Engine::new(EngineConfiguration::default());
        engine
            .load_artifact(&artifact)
            .expect("core metadata survives the artifact");
    }
}

#[test]
fn artifact_diagnostics_retain_the_originating_source_file() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let artifact = engine
        .compile_artifact(
            "/artifact/bundle.whim",
            &[
                SourceFile::new(
                    "/artifact/declarations.whim",
                    "function fail_from_artifact(): void { throw new Whim\\Unwind\\Exception('no'); }",
                ),
                SourceFile::new(
                    "/artifact/entry.whim",
                    "fail_from_artifact();",
                ),
            ],
            ArtifactConfiguration::default(),
        )
        .expect("the artifact compiles")
        .into_bytes();

    let mut engine = Engine::new(EngineConfiguration::default());
    let error = engine
        .load_artifact(&artifact)
        .expect_err("the top-level throw escapes artifact initialization");
    assert!(error.to_string().contains("/artifact/declarations.whim"));
    assert!(error.to_string().contains("/artifact/entry.whim"));
}

#[test]
fn artifact_source_files_preserve_independent_namespace_scopes() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let declarations = engine
        .compile_artifact(
            "/artifact/bundle.whim",
            &[
                SourceFile::new(
                    "/artifact/namespaced.whim",
                    "namespace Artifact\\Namespaced; function scoped(): int { return 1; }",
                ),
                SourceFile::new(
                    "/artifact/global.whim",
                    "function global_from_second(): int { return 2; }",
                ),
            ],
            ArtifactConfiguration::default(),
        )
        .expect("the declaration artifact compiles")
        .into_bytes();
    let entry = compile(
        "assert!(global_from_second() == 2);",
        "/artifact/entry.whim",
    );

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&declarations)
        .expect("the declaration artifact loads");
    engine
        .load_artifact(&entry)
        .expect("the second source file retains the global namespace");
}

#[test]
fn artifact_source_files_have_distinct_closure_identities() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let declarations = engine
        .compile_artifact(
            "/artifact/closures.whim",
            &[
                SourceFile::new(
                    "/artifact/first.whim",
                    "function first_closure(): fn(): int { return fn(): int => 1; }",
                ),
                SourceFile::new(
                    "/artifact/second.whim",
                    "function second_closure(): fn(): int { return fn(): int => 2; }",
                ),
            ],
            ArtifactConfiguration::default(),
        )
        .expect("the closure artifact compiles")
        .into_bytes();
    let entry = compile(
        "$first = first_closure(); $second = second_closure(); assert!($first() == 1); assert!($second() == 2);",
        "/artifact/entry.whim",
    );

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&declarations)
        .expect("the closure artifact loads");
    engine
        .load_artifact(&entry)
        .expect("each source file retains its own closure prototype");
}

#[test]
fn artifact_source_files_rebase_main_chunks_and_isolate_locals() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let artifact = engine
        .compile_artifact(
            "/artifact/main-chunks.whim",
            &[
                SourceFile::new(
                    "/artifact/first-main.whim",
                    "$local = 'first'; assert!($local is string);",
                ),
                SourceFile::new(
                    "/artifact/second-main.whim",
                    "$undefined = false; try { discard!($local); } catch (Whim\\Unwind\\UndefinedVariableError $_) { $undefined = true; } assert!($undefined); $value = 'second'; $selected = match ($value) { 'second' => 2, $_ => 0 }; assert!($selected == 2);",
                ),
            ],
            ArtifactConfiguration::default(),
        )
        .expect("the main-chunk artifact compiles")
        .into_bytes();

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&artifact)
        .expect("rebased main chunks execute with file-local variables");
}

#[test]
fn artifact_source_files_rebase_later_main_side_tables() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let artifact = engine
        .compile_artifact(
            "/artifact/main-side-tables.whim",
            &[
                SourceFile::new(
                    "/artifact/first-tables.whim",
                    "final class FirstTableHolder { public int $other = 0; } $first = new FirstTableHolder(); $first->other = 1; $typed = dict[] as dict<string, int>; $string = match ('first') { 'first' => 1, $_ => 0 }; assert!($string == 1 && length!($typed) == 0);",
                ),
                SourceFile::new(
                    "/artifact/second-tables.whim",
                    "final class SecondTableHolder { public dict<int, int> $items = dict[]; } $second = new SecondTableHolder(); $second->items[0] = 42; assert!($second->items[0] == 42); $boolean = match (true) { true => 1, false => 0 }; $range = match (5) { 0..=10 => 2, $_ => 0 }; assert!($boolean == 1 && $range == 2); if (false) { panic!('must not run'); }",
                ),
            ],
            ArtifactConfiguration::default(),
        )
        .expect("all side tables in later main chunks are rebased")
        .into_bytes();

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&artifact)
        .expect("the rebased main chunks execute");
}

#[test]
fn optimized_string_returns_accept_inline_strings() {
    let artifact = compile(
        "function short_artifact_string(): string { return 'short'; } assert!(short_artifact_string() == 'short');",
        "/artifact/short-string.whim",
    );

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&artifact)
        .expect("an optimized string return may use inline storage");
}

#[test]
fn recursive_aliases_keep_one_reified_shape_after_artifact_loading() {
    let artifact = compile(
        "type Datum = null|int|vec<Datum>|dict<string, Datum>; type Data = dict<string, Datum>; final readonly class Box<T> { public function __construct(public T $value) {} } function accept_box(Box<Data> $box): int { return length!($box->value); } $box = new Box::<Data>(dict[]); assert!(accept_box($box) == 0);",
        "/artifact/recursive-alias.whim",
    );

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&artifact)
        .expect("artifact loading preserves recursive reified aliases");
}

#[test]
fn verified_static_artifacts_keep_expanded_alias_declarations() {
    let artifact = compile(
        "type Identifier = int; final readonly class Entry { public function __construct(public Identifier $id) {} } function identifier(Entry $entry): Identifier { return $entry->id; } assert!(identifier(new Entry(42)) == 42);",
        "/artifact/static-alias.whim",
    );
    let artifact = Box::leak(artifact.into_boxed_slice());

    let mut engine = Engine::new(EngineConfiguration::default());
    // SAFETY: `compile` returned these bytes unchanged from this runtime build.
    unsafe { engine.load_verified_static_artifact(artifact) }
        .expect("the verified static artifact keeps expanded aliases");
}

#[test]
fn artifacts_retain_embedded_file_bytes() {
    let directory = env::temp_dir().join(format!("whim-artifact-embed-{}", process::id()));
    match fs::remove_dir_all(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => panic!("the old test directory could not be removed: {error}"),
    }
    fs::create_dir(&directory).expect("the test directory is creatable");
    fs::write(directory.join("asset.bin"), b"first\0second")
        .expect("the embedded asset is writable");
    let source_path = directory.join("source.whim");
    let source_path = source_path
        .to_str()
        .expect("the temporary source path is UTF-8");

    let mut engine = Engine::new(EngineConfiguration::default());
    let artifact = engine
        .compile_artifact(
            "/artifact/embedded.whim",
            &[SourceFile::new(
                source_path,
                "const EMBEDDED = embed!('./asset.bin'); assert!(EMBEDDED == \"first\\x00second\");",
            )],
            ArtifactConfiguration::default(),
        )
        .expect("the artifact embeds the asset")
        .into_bytes();
    fs::remove_dir_all(&directory).expect("the source tree is removable after compilation");

    let mut engine = Engine::new(EngineConfiguration::default());
    engine
        .load_artifact(&artifact)
        .expect("the artifact runs without its source asset");
}

#[test]
fn artifacts_preserve_object_shapes_and_public_property_patterns() {
    let artifact = compile(
        r"
        type Shape<T> = #{ value: T, ... };
        class Base {
            private string $value = 'private';
            public function extract(): mixed { return match ($this) { #{ $value } => $value }; }
        }
        class Child extends Base { public int $value = 42; }
        function identity(Shape<int> $value): Shape<int> { return $value; }
        $value = new Child();
        assert!($value is #{ value: int });
        assert!(!($value is #{}));
        assert!(identity($value) == $value);
        assert!($value->extract() == 42);
        ",
        "/artifact/object-shapes.whim",
    );
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        engine
            .load_artifact(&artifact)
            .expect("object shapes survive serialization");
    }
}

#[test]
fn artifacts_preserve_boolean_dictionary_shape_keys() {
    let artifact = compile(
        r"
        type Partition<T> = dict[true => T, false => T];
        function swap<T>(Partition<T> $value): Partition<T> {
            return match ($value) {
                dict[true => $yes, false => $no] => dict[true => $no, false => $yes],
            };
        }
        function classify(mixed $value): int {
            return match ($value) {
                dict[true => 1, false => string] => 1,
                dict[true => 2, false => string] => 2,
                dict[true => int, false => string] => 3,
                $_ => 0,
            };
        }
        assert!(swap::<int>(dict[true => 1, false => 2]) == dict[true => 2, false => 1]);
        assert!(!(dict[1 => 1, 0 => 2] is Partition<int>));
        assert!(classify(dict[true => 1, false => 'no']) == 1);
        assert!(classify(dict[true => 2, false => 'no']) == 2);
        assert!(classify(dict[true => 3, false => 'no']) == 3);
        assert!(classify(dict[true => 1, false => 2]) == 0);
        assert!(classify(dict[1 => 1, 0 => 'no']) == 0);
        ",
        "/artifact/boolean-shape-keys.whim",
    );
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        engine
            .load_artifact(&artifact)
            .expect("boolean shape keys survive serialization");
    }
}
