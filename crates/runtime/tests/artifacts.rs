use std::env;
use std::fs;
use std::io::ErrorKind;
use std::process;

use whim_runtime::artifact::ArtifactConfiguration;
use whim_runtime::artifact::SourceFile;
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
