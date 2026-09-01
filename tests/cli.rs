use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::process::id;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

fn fixture(name: &str) -> PathBuf {
    static ORDINAL: AtomicU32 = AtomicU32::new(0);

    let ordinal = ORDINAL.fetch_add(1, Ordering::Relaxed);
    let directory = env::temp_dir().join(format!("whim-cli-{}-{name}-{ordinal}", id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("the fixture directory is creatable");
    directory
}

fn run<I, S>(arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_whim"))
        .args(arguments)
        .output()
        .expect("the unified CLI spawns")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn colors_control_log_output() {
    let missing = OsStr::new("/definitely/missing/whim-program.whim");

    let colored = run([OsStr::new("--colors"), OsStr::new("always"), missing]);
    let plain = run([OsStr::new("--colors"), OsStr::new("never"), missing]);

    assert!(!colored.status.success());
    assert!(!plain.status.success());
    assert!(stderr_of(&colored).contains("\u{1b}["));
    assert!(!stderr_of(&plain).contains("\u{1b}["));
}

#[test]
fn colors_control_engine_diagnostics() {
    let directory = fixture("diagnostic-colors");
    let program = directory.join("main.whim");
    fs::write(&program, "write_line!(1 + vec[]);\n").expect("the program is writable");

    let colored = run([
        OsStr::new("--colors"),
        OsStr::new("always"),
        program.as_os_str(),
    ]);
    let plain = run([
        OsStr::new("--colors"),
        OsStr::new("never"),
        program.as_os_str(),
    ]);

    assert!(!colored.status.success());
    assert!(!plain.status.success());
    assert!(stderr_of(&colored).contains("\u{1b}["));
    assert!(!stderr_of(&plain).contains("\u{1b}["));
}

#[test]
fn colors_control_help_output() {
    let colored = run(["--colors", "always", "--help"]);
    let plain = run(["--colors", "never", "--help"]);

    assert!(colored.status.success(), "{}", stderr_of(&colored));
    assert!(plain.status.success(), "{}", stderr_of(&plain));
    assert!(stdout_of(&colored).contains("\u{1b}["));
    assert!(!stdout_of(&plain).contains("\u{1b}["));
}

#[test]
fn colors_control_argument_errors() {
    let colored = run(["--colors=always", "--definitely-invalid"]);
    let plain = run(["--colors=never", "--definitely-invalid"]);

    assert_eq!(colored.status.code(), Some(2));
    assert_eq!(plain.status.code(), Some(2));
    assert!(stderr_of(&colored).contains("\u{1b}["));
    assert!(!stderr_of(&plain).contains("\u{1b}["));
}

#[test]
fn color_environment_controls_clap_output() {
    let forced_help = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env("FORCE_COLOR", "1")
        .env_remove("NO_COLOR")
        .arg("--help")
        .output()
        .expect("the unified CLI spawns");
    let plain_help = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1")
        .arg("--help")
        .output()
        .expect("the unified CLI spawns");
    let forced_error = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env("FORCE_COLOR", "1")
        .env_remove("NO_COLOR")
        .arg("--definitely-invalid")
        .output()
        .expect("the unified CLI spawns");
    let plain_error = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1")
        .arg("--definitely-invalid")
        .output()
        .expect("the unified CLI spawns");

    assert!(stdout_of(&forced_help).contains("\u{1b}["));
    assert!(!stdout_of(&plain_help).contains("\u{1b}["));
    assert!(stderr_of(&forced_error).contains("\u{1b}["));
    assert!(!stderr_of(&plain_error).contains("\u{1b}["));
}

#[test]
fn a_path_is_shorthand_for_run() {
    let directory = fixture("shorthand");
    let program = directory.join("main.whim");
    fs::write(&program, "write_line!('shorthand');\n").expect("the program is writable");

    let output = run([program.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "shorthand\n");
}

#[test]
fn run_executes_a_program_explicitly() {
    let directory = fixture("explicit-run");
    let program = directory.join("main.whim");
    fs::write(&program, "write_line!('explicit');\n").expect("the program is writable");

    let output = run([OsStr::new("run"), program.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "explicit\n");
}

#[test]
fn both_run_forms_forward_every_argument_after_the_file() {
    let directory = fixture("arguments");
    let program = directory.join("main.whim");
    fs::write(
        &program,
        "foreach (Whim\\Env\\get_arguments() as $argument) { write_line!($argument); }\n",
    )
    .expect("the program is writable");

    for prefix in [vec![], vec![OsStr::new("run")]] {
        let output = Command::new(env!("CARGO_BIN_EXE_whim"))
            .args(prefix)
            .arg(&program)
            .args(["--future-option", "fmt", "plain"])
            .output()
            .expect("the unified CLI spawns");

        assert!(output.status.success(), "{}", stderr_of(&output));
        assert_eq!(stdout_of(&output), "--future-option\nfmt\nplain\n");
    }
}

#[test]
fn global_config_works_in_both_run_forms() {
    let directory = fixture("global-config");
    let program = directory.join("main.whim");
    let config = directory.join("settings.toml");
    fs::write(&program, "write_line!('ran');\n").expect("the program is writable");
    fs::write(
        &config,
        "manifest-version = 1\n[runtime]\noptimizations = \"off\"\ncall-depth = 50\ncycle-threshold = 8\nfull-trace = true\n",
    )
    .expect("the config is writable");

    let shorthand = run([
        OsStr::new("--config"),
        config.as_os_str(),
        program.as_os_str(),
    ]);
    let explicit = run([
        OsStr::new("--config"),
        config.as_os_str(),
        OsStr::new("run"),
        program.as_os_str(),
    ]);

    assert!(shorthand.status.success(), "{}", stderr_of(&shorthand));
    assert!(explicit.status.success(), "{}", stderr_of(&explicit));
    assert_eq!(shorthand.stdout, explicit.stdout);
}

#[test]
fn runtime_config_and_environment_control_disassembly() {
    let directory = fixture("configured-disassembly");
    let program = directory.join("main.whim");
    let config = directory.join("whim.toml");
    fs::write(&program, "$value = 1 + 2;\nwrite_line!($value);\n")
        .expect("the program is writable");
    fs::write(
        &config,
        "manifest-version = 1\n[runtime]\noptimizations = \"off\"\n",
    )
    .expect("the config is writable");

    let unoptimized = run([
        OsStr::new("--config"),
        config.as_os_str(),
        OsStr::new("disassemble"),
        program.as_os_str(),
    ]);
    let optimized = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env("WHIM_OPTIMIZATIONS", "on")
        .args([OsStr::new("--config"), config.as_os_str()])
        .args([OsStr::new("disassemble"), program.as_os_str()])
        .output()
        .expect("the unified CLI spawns");

    assert!(unoptimized.status.success(), "{}", stderr_of(&unoptimized));
    assert!(optimized.status.success(), "{}", stderr_of(&optimized));
    assert!(stdout_of(&unoptimized).contains(" AddImmediate "));
    assert!(!stdout_of(&optimized).contains(" AddImmediate "));
}

#[test]
fn removed_runtime_options_are_rejected() {
    let directory = fixture("removed-runtime-options");
    let program = directory.join("main.whim");
    fs::write(&program, "write_line!('must not run');\n").expect("the program is writable");

    for option in [
        "--no-optimize",
        "--call-depth",
        "--cycle-threshold",
        "--full-trace",
    ] {
        let output = run([OsStr::new(option), program.as_os_str()]);
        assert!(!output.status.success(), "{option} must be removed");
        assert!(!stdout_of(&output).contains("must not run"));
    }
}

#[test]
fn runtime_environment_overrides_the_manifest() {
    let directory = fixture("runtime-environment");
    let program = directory.join("main.whim");
    let config = directory.join("whim.toml");
    fs::write(
        &program,
        "#[Whim\\Marker\\TraceBoundary]\nfunction hidden(): never { throw new Whim\\Unwind\\Exception('failed'); }\nhidden();\n",
    )
    .expect("the program is writable");
    fs::write(
        &config,
        "manifest-version = 1\n[runtime]\nfull-trace = false\n",
    )
    .expect("the config is writable");

    let ordinary = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&directory)
        .arg(&program)
        .output()
        .expect("the unified CLI spawns");
    let overridden = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&directory)
        .env("WHIM_FULL_TRACE", "true")
        .arg(&program)
        .output()
        .expect("the unified CLI spawns");

    assert!(!stderr_of(&ordinary).contains("  hidden\n"));
    assert!(stderr_of(&overridden).contains("  hidden\n"));
}

#[test]
fn invalid_runtime_environment_is_rejected() {
    let directory = fixture("invalid-runtime-environment");
    let program = directory.join("main.whim");
    fs::write(&program, "write_line!('must not run');\n").expect("the program is writable");
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env("WHIM_CALL_DEPTH", "many")
        .arg(&program)
        .output()
        .expect("the unified CLI spawns");

    assert!(!output.status.success());
    let error = stderr_of(&output);
    assert!(error.contains("WHIM_CALL_DEPTH"));
    assert!(error.contains("many"));
    assert!(!stdout_of(&output).contains("must not run"));

    let formatted = directory.join("formatted.whim");
    fs::write(&formatted, "$value = 1;\n").expect("the formatted source is writable");
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env("WHIM_CALL_DEPTH", "many")
        .args([OsStr::new("fmt"), OsStr::new("--check")])
        .arg(&formatted)
        .output()
        .expect("the unified CLI spawns");

    assert!(output.status.success(), "{}", stderr_of(&output));
}

#[test]
fn disassemble_prints_bytecode_without_running_the_program() {
    let directory = fixture("disassemble");
    let program = directory.join("main.whim");
    fs::write(
        &program,
        "function helper(int $value): int { return $value + 1; }\n\
         write_line!('must not run');\n\
         exit!(7);\n",
    )
    .expect("the program is writable");

    let output = run([OsStr::new("disassemble"), program.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = stdout_of(&output);
    assert!(stdout.contains("== main =="));
    assert!(stdout.contains("== helper =="));
    assert!(!stdout.contains("must not run\n"));
}

#[test]
fn disassemble_accepts_stdin_and_rejects_unknown_options() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_whim"))
        .args(["disassemble", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the unified CLI spawns");
    child
        .stdin
        .take()
        .expect("standard input is piped")
        .write_all(b"$value = 1;\n")
        .expect("the source is writable");
    let stdin = child.wait_with_output().expect("the unified CLI exits");
    let unknown = run(["disassemble", "--definitely-invalid"]);

    assert!(stdin.status.success(), "{}", stderr_of(&stdin));
    assert!(stdout_of(&stdin).contains("== main =="));
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr_of(&unknown).contains("unexpected argument"));
}

#[test]
fn run_reports_non_broken_pipe_output_failures() {
    let directory = fixture("failed-output");
    let program = directory.join("main.whim");
    fs::write(&program, "write_line!('output');\n").expect("the program is writable");
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg(&program)
        .stdout(Stdio::from(
            fs::File::open("/dev/null").expect("the null device is readable"),
        ))
        .stderr(Stdio::piped())
        .output()
        .expect("the unified CLI spawns");

    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("could not write command output"));
}

#[test]
fn disassemble_reports_non_broken_pipe_output_failures() {
    let directory = fixture("failed-disassembly-output");
    let program = directory.join("main.whim");
    fs::write(&program, "$value = 1;\n").expect("the program is writable");
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .args([OsStr::new("disassemble"), program.as_os_str()])
        .stdout(Stdio::from(
            fs::File::open("/dev/null").expect("the null device is readable"),
        ))
        .stderr(Stdio::piped())
        .output()
        .expect("the unified CLI spawns");

    assert!(!output.status.success());
    assert!(stderr_of(&output).contains("could not write the disassembly"));
}

#[test]
fn a_command_name_can_still_be_run_as_a_path_after_dashdash() {
    let directory = fixture("command-name-path");
    let program = directory.join("fmt");
    fs::write(&program, "write_line!('file');\n").expect("the program is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&directory)
        .args(["--", "fmt"])
        .output()
        .expect("the unified CLI spawns");

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "file\n");
}

#[test]
fn fmt_formats_one_file() {
    let directory = fixture("format-file");
    let source = directory.join("source.whim");
    fs::write(&source, "$value   =   1;\n").expect("the source is writable");

    let output = run([OsStr::new("fmt"), source.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(&source).expect("the source is readable"),
        "$value = 1;\n"
    );
}

#[test]
fn fmt_walks_directories_and_only_formats_whim_files() {
    let directory = fixture("format-directory");
    let nested = directory.join("nested");
    fs::create_dir(&nested).expect("the nested directory is creatable");
    let first = directory.join("first.whim");
    let second = nested.join("second.whim");
    let unrelated = nested.join("notes.txt");
    fs::write(&first, "$first   =   1;\n").expect("the source is writable");
    fs::write(&second, "$second   =   2;\n").expect("the source is writable");
    fs::write(&unrelated, "$not   =   formatted;\n").expect("the text is writable");

    let output = run([OsStr::new("fmt"), directory.as_os_str()]);

    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        fs::read_to_string(&first).expect("the source is readable"),
        "$first = 1;\n"
    );
    assert_eq!(
        fs::read_to_string(&second).expect("the source is readable"),
        "$second = 2;\n"
    );
    assert_eq!(
        fs::read_to_string(&unrelated).expect("the text is readable"),
        "$not   =   formatted;\n"
    );
}

#[test]
fn fmt_check_reports_but_does_not_rewrite() {
    let directory = fixture("format-check");
    let source = directory.join("source.whim");
    let original = "$value   =   1;\n";
    fs::write(&source, original).expect("the source is writable");

    let output = run([OsStr::new("fmt"), OsStr::new("--check"), source.as_os_str()]);

    assert!(!output.status.success());
    assert!(stdout_of(&output).contains("--- "));
    assert_eq!(
        fs::read_to_string(&source).expect("the source is readable"),
        original
    );
}
