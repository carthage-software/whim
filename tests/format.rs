//! Command-line tests for `whim fmt`.

use std::env::temp_dir;
use std::ffi::OsStr;
use std::fs;
use std::iter::empty;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::id;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use whim_syn::parser::MAX_STRUCTURAL_DEPTH;

fn fixture(name: &str, source: &str) -> (PathBuf, PathBuf) {
    static ORDINAL: AtomicU32 = AtomicU32::new(0);

    let ordinal = ORDINAL.fetch_add(1, Ordering::Relaxed);
    let directory = temp_dir().join(format!("whim-fmt-cli-{}-{name}-{ordinal}", id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("the fixture directory is creatable");

    let path = directory.join("source.whim");
    fs::write(&path, source).expect("the fixture file is writable");

    (directory, path)
}

fn run<I, S>(arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg("fmt")
        .args(arguments)
        .output()
        .expect("the binary spawns")
}

fn run_in<I, S>(directory: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(directory)
        .arg("fmt")
        .args(arguments)
        .output()
        .expect("the binary spawns")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn entries_of(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("a readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();

    names
}

#[test]
fn a_zero_print_width_is_rejected_before_anything_is_written() {
    let (_directory, path) = fixture("zero-print-width", "$a   =   1;\n");
    let original = fs::read_to_string(&path).expect("the fixture is readable");

    let output = run(["--print-width".as_ref(), "0".as_ref(), path.as_os_str()]);

    assert!(!output.status.success(), "a zero print width is not usable");
    let message = stderr_of(&output);
    assert!(
        message.contains("print_width"),
        "the message should name the setting: {message}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("the fixture is readable"),
        original,
        "nothing should be written when the settings are rejected"
    );
}

#[test]
fn a_zero_tab_width_is_rejected() {
    let (_directory, path) = fixture("zero-tab-width", "$a = 1;\n");

    let output = run(["--tab-width".as_ref(), "0".as_ref(), path.as_os_str()]);

    assert!(!output.status.success(), "a zero tab width is not usable");
    assert!(
        stderr_of(&output).contains("tab_width"),
        "the message should name the setting"
    );
}

#[test]
fn a_tab_width_wider_than_the_print_width_is_rejected() {
    let (_directory, path) = fixture("wide-tab", "$a = 1;\n");

    let output = run([
        "--print-width".as_ref(),
        "20".as_ref(),
        "--tab-width".as_ref(),
        "40".as_ref(),
        path.as_os_str(),
    ]);

    assert!(
        !output.status.success(),
        "one indentation level would overflow every line"
    );
    assert!(stderr_of(&output).contains("tab_width"));
}

#[test]
fn an_invalid_setting_in_a_config_file_is_rejected_too() {
    let (directory, path) = fixture("bad-config", "$a = 1;\n");
    let config = directory.join("whim.toml");
    fs::write(&config, "manifest-version = 1\n[format]\nprint_width = 0\n")
        .expect("the config is writable");

    let output = run(["--config".as_ref(), config.as_os_str(), path.as_os_str()]);

    assert!(
        !output.status.success(),
        "a setting is a setting wherever it was written"
    );
    assert!(stderr_of(&output).contains("print_width"));
}

#[test]
fn the_unified_manifest_is_discovered_automatically() {
    let (directory, path) = fixture("unified-config", "$a = 1;\n");
    fs::write(
        directory.join("whim.toml"),
        "manifest-version = 1\n[format]\nprint_width = 0\n",
    )
    .expect("the config is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&directory)
        .args(["fmt".as_ref(), path.as_os_str()])
        .output()
        .expect("the binary spawns");

    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("print_width"));
}

#[test]
fn the_removed_dot_manifest_is_not_a_formatter_config() {
    let (directory, path) = fixture("removed-dot-config", "$a   =   1;\n");
    fs::write(directory.join(".whim.toml"), "print_width = 0\n")
        .expect("the old config is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&directory)
        .args(["fmt".as_ref(), path.as_os_str()])
        .output()
        .expect("the binary spawns");

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(path).expect("the source is readable"),
        "$a = 1;\n"
    );
}

#[test]
fn no_paths_formats_the_project_and_skips_vendor() {
    let (directory, root_source) = fixture("project-default", "$root   =   1;\n");
    fs::write(directory.join("whim.toml"), "manifest-version = 1\n")
        .expect("the manifest is writable");
    let source_directory = directory.join("src").join("Nested");
    let vendor_directory = directory.join("vendor").join("package");
    fs::create_dir_all(&source_directory).expect("the source directory is creatable");
    fs::create_dir_all(&vendor_directory).expect("the vendor directory is creatable");
    let source = source_directory.join("Thing.whim");
    let vendor = vendor_directory.join("Thing.whim");
    fs::write(&source, "$source   =   1;\n").expect("the source is writable");
    fs::write(&vendor, "$vendor   =   1;\n").expect("the vendor source is writable");

    let output = run_in(&source_directory, empty::<&OsStr>());

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(root_source).expect("the root source is readable"),
        "$root = 1;\n"
    );
    assert_eq!(
        fs::read_to_string(source).expect("the nested source is readable"),
        "$source = 1;\n"
    );
    assert_eq!(
        fs::read_to_string(vendor).expect("the vendor source is readable"),
        "$vendor   =   1;\n"
    );
}

#[test]
fn project_include_and_exclude_select_the_walk() {
    let (directory, root_source) = fixture("project-patterns", "$root   =   1;\n");
    fs::write(
        directory.join("whim.toml"),
        "manifest-version = 1\n[format]\ninclude = [\"src\"]\nexclude = [\"src/generated\"]\n",
    )
    .expect("the manifest is writable");
    let source_directory = directory.join("src");
    let generated_directory = source_directory.join("generated");
    let test_directory = directory.join("tests");
    fs::create_dir_all(&generated_directory).expect("the generated directory is creatable");
    fs::create_dir_all(&test_directory).expect("the test directory is creatable");
    let source = source_directory.join("App.whim");
    let generated = generated_directory.join("Table.whim");
    let test = test_directory.join("App.whim");
    fs::write(&source, "$source   =   1;\n").expect("the source is writable");
    fs::write(&generated, "$generated   =   1;\n").expect("the generated source is writable");
    fs::write(&test, "$test   =   1;\n").expect("the test source is writable");

    let output = run_in(&directory, empty::<&OsStr>());

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(root_source).expect("the root source is readable"),
        "$root   =   1;\n"
    );
    assert_eq!(
        fs::read_to_string(source).expect("the source is readable"),
        "$source = 1;\n"
    );
    assert_eq!(
        fs::read_to_string(generated).expect("the generated source is readable"),
        "$generated   =   1;\n"
    );
    assert_eq!(
        fs::read_to_string(test).expect("the test source is readable"),
        "$test   =   1;\n"
    );
}

#[test]
fn explicit_directories_ignore_include_but_honor_exclude() {
    let (directory, _root_source) = fixture("explicit-directory", "$root   =   1;\n");
    fs::write(
        directory.join("whim.toml"),
        "manifest-version = 1\n[format]\ninclude = [\"elsewhere\"]\nexclude = [\"src/generated\"]\n",
    )
    .expect("the manifest is writable");
    let source_directory = directory.join("src");
    let generated_directory = source_directory.join("generated");
    fs::create_dir_all(&generated_directory).expect("the generated directory is creatable");
    let source = source_directory.join("App.whim");
    let generated = generated_directory.join("Table.whim");
    fs::write(&source, "$source   =   1;\n").expect("the source is writable");
    fs::write(&generated, "$generated   =   1;\n").expect("the generated source is writable");

    let output = run_in(&directory, [OsStr::new("src")]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(source).expect("the source is readable"),
        "$source = 1;\n"
    );
    assert_eq!(
        fs::read_to_string(generated).expect("the generated source is readable"),
        "$generated   =   1;\n"
    );
}

#[test]
fn explicit_files_bypass_exclusions() {
    let (directory, _root_source) = fixture("explicit-file", "$root = 1;\n");
    fs::write(
        directory.join("whim.toml"),
        "manifest-version = 1\n[format]\nexclude = [\"vendor\"]\n",
    )
    .expect("the manifest is writable");
    let vendor_directory = directory.join("vendor");
    fs::create_dir(&vendor_directory).expect("the vendor directory is creatable");
    let vendor = vendor_directory.join("Thing.whim");
    fs::write(&vendor, "$vendor   =   1;\n").expect("the vendor source is writable");

    let output = run_in(&directory, [vendor.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(vendor).expect("the vendor source is readable"),
        "$vendor = 1;\n"
    );
}

#[test]
fn no_paths_without_a_manifest_fails_without_writing() {
    let (directory, source) = fixture("project-missing", "$value   =   1;\n");

    let output = run_in(&directory, empty::<&OsStr>());

    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("whim.toml"));
    assert_eq!(
        fs::read_to_string(source).expect("the source is readable"),
        "$value   =   1;\n"
    );
}

#[test]
fn a_file_named_twice_is_reported_once() {
    let (_directory, path) = fixture("duplicate-check", "$a   =   1;\n");

    let output = run(["--check".as_ref(), path.as_os_str(), path.as_os_str()]);

    assert!(!output.status.success(), "the file is not formatted");
    let reported = stdout_of(&output).matches("--- ").count();
    assert_eq!(reported, 1, "the same file should be reported once");
}

#[test]
fn a_file_named_twice_is_formatted_once_and_correctly() {
    let (directory, path) = fixture("duplicate-write", "$a   =   1;\n");

    let output = run([path.as_os_str(), path.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(&path).expect("the file is readable"),
        "$a = 1;\n"
    );
    assert_eq!(
        entries_of(&directory),
        vec!["source.whim".to_string()],
        "no temporary file should be left behind"
    );
}

#[test]
fn the_same_file_reached_by_two_spellings_is_formatted_once() {
    let (directory, path) = fixture("duplicate-spelling", "$a   =   1;\n");
    let indirect = directory.join(".").join("source.whim");

    let output = run([path.as_os_str(), indirect.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(&path).expect("the file is readable"),
        "$a = 1;\n"
    );
}

#[test]
fn a_rewrite_leaves_no_temporary_file_behind() {
    let (directory, path) = fixture("no-leftovers", "$a   =   1;\n$b   =   2;\n");

    let output = run([path.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        entries_of(&directory),
        vec!["source.whim".to_string()],
        "the temporary file should have been renamed over the target"
    );
}

#[test]
fn a_file_that_does_not_parse_is_left_untouched() {
    let (_directory, path) = fixture("unparsable", "$a = = ;\n");
    let original = fs::read_to_string(&path).expect("the fixture is readable");

    let output = run([path.as_os_str()]);

    assert!(!output.status.success(), "the file does not parse");
    assert_eq!(
        fs::read_to_string(&path).expect("the fixture is readable"),
        original,
        "a file that could not be formatted must keep its contents"
    );
}

#[test]
fn colors_control_source_diagnostics() {
    let (_directory, path) = fixture("diagnostic-colors", "$a = = ;\n");
    let run = |colors, log| {
        Command::new(env!("CARGO_BIN_EXE_whim"))
            .env("WHIM_LOG", log)
            .args(["--colors", colors, "fmt"])
            .arg(&path)
            .output()
            .expect("the binary spawns")
    };

    let colored = run("always", "error");
    let plain = run("never", "error");
    let silent = run("always", "off");

    assert!(!colored.status.success());
    assert!(!plain.status.success());
    assert!(!silent.status.success());
    assert!(colored.stderr.contains(&0x1b));
    assert!(!colored.stderr.windows(4).any(|bytes| bytes == b"\\x1b"));
    assert!(!plain.stderr.contains(&0x1b));
    assert!(silent.stderr.is_empty());
}

#[test]
fn a_rewrite_replaces_the_file_rather_than_writing_through_it() {
    use std::os::unix::fs::MetadataExt;

    let (directory, path) = fixture("atomic-replace", "$a   =   1;\n");
    let link = directory.join("witness.whim");
    fs::hard_link(&path, &link).expect("a hard link is creatable");

    let before = fs::metadata(&path).expect("the file is readable").ino();

    let output = run([path.as_os_str()]);
    assert!(output.status.success(), "{}", stderr_of(&output));

    let after = fs::metadata(&path).expect("the file is readable").ino();
    assert_ne!(
        before, after,
        "the path should point at a new file, not at the one that was rewritten in place"
    );
    assert_eq!(
        fs::read_to_string(&link).expect("the link is readable"),
        "$a   =   1;\n",
        "the original file object should be untouched, which is what makes the \
         replacement atomic: a reader holds either the whole old file or the whole new one"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("the file is readable"),
        "$a = 1;\n"
    );
}

#[test]
fn formatting_through_a_symbolic_link_keeps_the_link_and_rewrites_its_target() {
    let (directory, path) = fixture("symlink-target", "$a   =   1;\n");
    let link = directory.join("link.whim");
    symlink(&path, &link).expect("a symbolic link is creatable");

    let output = run([link.as_os_str()]);
    assert!(output.status.success(), "{}", stderr_of(&output));

    assert!(
        fs::symlink_metadata(&link)
            .expect("the link is readable")
            .file_type()
            .is_symlink(),
        "the link must still be a symbolic link, not a regular file that replaced it"
    );
    assert_eq!(
        fs::read_link(&link).expect("the link is readable"),
        path,
        "the link must still resolve to the same file"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("the file is readable"),
        "$a = 1;\n",
        "the file the link names is the one that should have been formatted"
    );
}

#[test]
fn a_symbolic_link_and_its_target_are_one_file() {
    let (directory, path) = fixture("symlink-dedup", "$a   =   1;\n");
    let link = directory.join("link.whim");
    symlink(&path, &link).expect("a symbolic link is creatable");

    let output = run([link.as_os_str(), path.as_os_str()]);
    assert!(output.status.success(), "{}", stderr_of(&output));

    assert!(
        fs::symlink_metadata(&link)
            .expect("the link is readable")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&path).expect("the file is readable"),
        "$a = 1;\n"
    );
    assert_eq!(
        entries_of(&directory),
        vec!["link.whim".to_owned(), "source.whim".to_owned()],
        "no temporary file should be left behind"
    );
}

#[test]
fn a_broken_symbolic_link_is_reported_and_left_alone() {
    let (directory, _path) = fixture("symlink-broken", "$a   =   1;\n");
    let link = directory.join("broken.whim");
    symlink(directory.join("absent.whim"), &link).expect("a symbolic link is creatable");

    let output = run([link.as_os_str()]);
    assert!(!output.status.success());
    assert!(
        stderr_of(&output).contains("broken.whim"),
        "the report should name the path as it was given: {}",
        stderr_of(&output)
    );
    assert!(
        fs::symlink_metadata(&link)
            .expect("the link is readable")
            .file_type()
            .is_symlink(),
        "a link that could not be followed must be left alone"
    );
}

#[test]
fn a_rewrite_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;

    let (_directory, path) = fixture("keep-mode", "$a   =   1;\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("the mode is settable");

    let output = run([path.as_os_str()]);
    assert!(output.status.success(), "{}", stderr_of(&output));

    let mode = fs::metadata(&path)
        .expect("the file is readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640, "the replaced file should keep the mode it had");
}

#[test]
fn a_long_flat_operator_chain_is_formatted_not_aborted() {
    let links = MAX_STRUCTURAL_DEPTH / 2 - 8;
    let mut source = String::from("$x = 0");
    for _ in 0..links {
        source.push_str("+1");
    }
    source.push_str(";\n");

    let (_directory, path) = fixture("flat-chain", &source);
    let output = run([path.as_os_str()]);
    assert!(
        output.status.success(),
        "a long chain must format, got {:?}: {}",
        output.status,
        stderr_of(&output)
    );

    let formatted = fs::read_to_string(&path).expect("the file is readable");
    let mut expected = String::from("$x =\n  0");
    for _ in 0..links {
        expected.push_str("\n  + 1");
    }
    expected.push_str(";\n");
    assert_eq!(formatted, expected);
}

#[test]
fn a_tree_past_the_structural_limit_is_a_diagnostic() {
    let mut source = String::from("$x = $y");
    for _ in 0..MAX_STRUCTURAL_DEPTH.div_ceil(4) {
        source.push_str("()");
    }
    source.push_str(";\n");

    let (_directory, path) = fixture("too-deep", &source);
    let output = run([path.as_os_str()]);
    let errors = stderr_of(&output);
    assert!(
        errors.contains("levels deep"),
        "expected the structural-depth diagnostic, got: {errors}"
    );
    assert!(!output.status.success(), "a refused file must fail the run");

    let left_alone = fs::read_to_string(&path).expect("the file is readable");
    assert_eq!(
        left_alone, source,
        "a file the formatter refuses must be left as it was"
    );
}

#[test]
fn a_long_partial_application_chain_is_formatted() {
    let mut source = String::from("$g = f(?)");
    for _ in 0..MAX_STRUCTURAL_DEPTH / 4 - 8 {
        source.push_str("(?)");
    }
    source.push_str(";\n");

    let (_directory, path) = fixture("partial-chain", &source);
    let output = run([path.as_os_str()]);
    assert!(
        output.status.success(),
        "a long partial-application chain must format, got {:?}: {}",
        output.status,
        stderr_of(&output)
    );
}
