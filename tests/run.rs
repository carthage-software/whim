use std::env;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::id;

#[cfg(target_os = "linux")]
use whim_runtime::path::path_bytes;
use whim_syn::parser::MAX_STRUCTURAL_DEPTH;

fn write_fixture(name: &str, source: &str) -> PathBuf {
    let directory = env::temp_dir().join(format!("whim-cli-{}", id()));
    fs::create_dir_all(&directory).expect("the fixture directory is creatable");
    let path = directory.join(name);
    fs::write(&path, source).expect("the fixture file is writable");
    path
}

fn run_program(name: &str, source: &str, flags: &[&str]) -> Output {
    let path = write_fixture(name, source);
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .args(flags)
        .arg(&path)
        .output()
        .expect("the binary spawns");
    let _ = fs::remove_file(&path);
    output
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code_of(output: &Output) -> i32 {
    output.status.code().expect("the process exits with a code")
}

#[cfg(target_os = "linux")]
#[test]
fn environment_paths_round_trip_non_utf8_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let root = env::current_dir()
        .expect("the workspace is available")
        .join("target")
        .join(format!("whim-path-{}", id()));
    let directory = root.join(OsString::from_vec(b"raw-\xFF".to_vec()));
    fs::create_dir_all(&directory).expect("the non-UTF-8 directory is creatable");
    let script = directory.join("main.whim");
    fs::write(
        &script,
        "write!(Whim\\Env\\current_directory());\n\
         write!('|');\n\
         write!(Whim\\Env\\current_script());\n",
    )
    .expect("the fixture is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&directory)
        .arg(&script)
        .output()
        .expect("the binary spawns");
    let mut expected =
        path_bytes(&fs::canonicalize(&directory).expect("the directory canonicalizes"));
    expected.push(b'|');
    expected.extend(path_bytes(&script));

    assert_eq!(output.stdout, expected);
    assert_eq!(output.stderr, b"");
    assert_eq!(code_of(&output), 0);
    fs::remove_dir_all(&root).expect("the fixture directory is removable");
}

#[test]
fn current_script_preserves_the_relative_symlink_spelling() {
    let target = write_fixture(
        "current-script-target.whim",
        "write_line!(Whim\\Env\\current_script());\n",
    );
    let directory = target.parent().expect("the fixture has a directory");
    let link = directory.join("current-script-link.whim");
    let _ = fs::remove_file(&link);
    symlink(&target, &link).expect("the symbolic link is creatable");

    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(directory)
        .arg("current-script-link.whim")
        .output()
        .expect("the binary spawns");

    assert_eq!(output.stdout, b"current-script-link.whim\n");
    assert_eq!(output.stderr, b"");
    assert_eq!(code_of(&output), 0);

    fs::remove_file(&link).expect("the symbolic link is removable");
    fs::remove_file(&target).expect("the fixture is removable");
}

#[test]
fn hello_world_prints_and_exits_zero() {
    let output = run_program("hello.whim", "write_line!('hello world');\n", &[]);
    assert_eq!(stdout_of(&output), "hello world\n");
    assert_eq!(stderr_of(&output), "");
    assert_eq!(code_of(&output), 0);
}

#[test]
fn the_exit_code_passes_through() {
    let output = run_program("exiting.whim", "write_line!('before');\nexit!(3);\n", &[]);
    assert_eq!(stdout_of(&output), "before\n");
    assert_eq!(code_of(&output), 3);
}

#[test]
fn panic_exits_255_with_a_redacted_trace_and_bypasses_handlers() {
    let source = "function visible(\n\
                    string $label,\n\
                    #[Whim\\Marker\\SensitiveParameter] string $secret,\n\
                  ): never {\n\
                    panic!('an impossible state was reached');\n\
                  }\n\
                  #[Whim\\Marker\\TraceBoundary]\n\
                  function hidden(): never {\n\
                    visible('public-marker', 'secret-marker');\n\
                  }\n\
                  try {\n\
                    hidden();\n\
                  } catch (Whim\\Unwind\\Error $_) {\n\
                    write_line!('caught');\n\
                  } finally {\n\
                    write_line!('finally');\n\
                  }\n";
    let output = run_program("panic.whim", source, &[]);
    let errors = stderr_of(&output);

    assert_eq!(stdout_of(&output), "");
    assert_eq!(code_of(&output), 255);
    assert!(errors.starts_with("panic: an impossible state was reached\n"));
    assert!(errors.contains("Stack backtrace:"));
    assert!(errors.contains("visible called with"));
    assert!(errors.contains("public-marker"));
    assert!(errors.contains("Whim\\Marker\\SensitiveParameterValue"));
    assert!(!errors.contains("secret-marker"));
    assert!(!errors.contains("  hidden"));
    assert!(!errors.contains("uncaught"));

    let path = write_fixture("panic-full-trace.whim", source);
    let full = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env("WHIM_FULL_TRACE", "true")
        .env("WHIM_OPTIMIZATIONS", "off")
        .arg(&path)
        .output()
        .expect("the binary spawns");
    let _ = fs::remove_file(path);

    assert_eq!(code_of(&full), 255);
    let errors = stderr_of(&full);
    assert!(errors.contains("  hidden"));
    assert!(!errors.contains("secret-marker"));
}

#[test]
fn an_uncaught_error_exits_255_with_its_trace() {
    let output = run_program(
        "uncaught.whim",
        "function detonate(): void {\n    throw new Whim\\Unwind\\Error('boom', 9);\n}\ndetonate();\n",
        &[],
    );
    let errors = stderr_of(&output);
    assert!(errors.contains("uncaught Whim\\Unwind\\Error: boom (code 9)"));
    assert!(errors.contains("\n 0  detonate"));
    assert!(errors.contains("\n 1  {main}"));
    assert!(errors.contains("error: the exception was thrown here"));
    assert!(errors.contains("throw new Whim\\Unwind\\Error('boom', 9);"));
    assert!(!errors.contains("\u{1b}["));
    assert_eq!(code_of(&output), 255);
}

#[test]
fn an_error_inside_a_required_file_uses_that_files_source() {
    let main = write_fixture(
        "require-diagnostic-main.whim",
        "require!('required-broken.whim');\n",
    );
    let directory = main.parent().expect("the fixture has a directory");
    let required = directory.join("required-broken.whim");
    fs::write(&required, "$value = 1").expect("the required fixture is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg(&main)
        .output()
        .expect("the binary spawns");
    let errors = stderr_of(&output);
    assert!(errors.contains("uncaught Whim\\Unwind\\ParserError:"));
    assert!(errors.contains("required-broken.whim:1:11"));
    assert!(errors.contains("1 | $value = 1"));
    let main_frame = concat!("\n 0  {", "main", "}");
    assert!(errors.contains(main_frame));
    assert!(errors.contains("require-diagnostic-main.whim:1"));
    assert_eq!(code_of(&output), 255);

    fs::remove_file(&required).expect("the required fixture is removable");
    fs::remove_file(&main).expect("the main fixture is removable");
}

#[test]
fn a_missing_required_file_highlights_the_require_site() {
    let output = run_program(
        "require-missing.whim",
        "require!('does-not-exist.whim');\n",
        &[],
    );
    let errors = stderr_of(&output);
    assert!(errors.contains("uncaught Whim\\Unwind\\RequireError:"));
    assert!(errors.contains("error: the exception originated here"));
    assert!(errors.contains("1 | require!('does-not-exist.whim');"));
    assert_eq!(code_of(&output), 255);
}

#[test]
fn disassemble_prints_bytecode_without_executing() {
    let output = run_program(
        "dumped.whim",
        "function helper(int $n): int {\n    return $n + 1;\n}\n\
         class Greeter {\n    public function greet(): string { return 'hi'; }\n}\n\
         write_line!(helper(1));\nexit!(7);\n",
        &["disassemble"],
    );
    let printed = stdout_of(&output);
    assert!(printed.contains("== main =="));
    assert!(printed.contains("== helper =="));
    assert!(printed.contains("== Greeter::greet =="));
    assert_eq!(stderr_of(&output), "");
    assert_eq!(code_of(&output), 0);
}

#[test]
fn disassemble_optimizes_against_the_loaded_standard_library() {
    let output = run_program(
        "dumped-standard.whim",
        "$bits = Whim\\Float\\to_bits(1.0);\n",
        &["disassemble"],
    );
    let printed = stdout_of(&output);
    assert!(printed.contains("Whim\\_Private\\float_to_bits"));
    assert_eq!(stderr_of(&output), "");
    assert_eq!(code_of(&output), 0);
}

#[test]
fn full_trace_restores_trace_boundary_frames() {
    let source = "#[Whim\\Marker\\TraceBoundary]\n\
                  function hidden(): never { throw new Whim\\Unwind\\Exception('failed'); }\n\
                  hidden();\n";
    let ordinary = run_program("trace-boundary.whim", source, &[]);
    assert!(!stderr_of(&ordinary).contains("  hidden\n"));

    let path = write_fixture("trace-boundary-full.whim", source);
    let full = Command::new(env!("CARGO_BIN_EXE_whim"))
        .env("WHIM_FULL_TRACE", "true")
        .arg(&path)
        .output()
        .expect("the binary spawns");
    let _ = fs::remove_file(path);
    assert!(stderr_of(&full).contains("  hidden\n"));
}

#[test]
fn a_missing_file_reports_clearly() {
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg("definitely_not_here.whim")
        .output()
        .expect("the binary spawns");
    let errors = stderr_of(&output);
    assert!(errors.contains("definitely_not_here.whim"));
    assert!(errors.contains("could not read"));
    assert_ne!(code_of(&output), 0);
}

#[test]
fn the_version_matches_the_runtime() {
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg("--version")
        .output()
        .expect("the binary spawns");
    assert_eq!(
        stdout_of(&output),
        format!("whim {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(code_of(&output), 0);
}

#[test]
fn a_closed_pipe_ends_the_run_quietly() {
    use std::io::Read;
    use std::process::Stdio;

    let path = write_fixture(
        "broken-pipe.whim",
        "$i = 0;\n\
         while ($i < 200000) { write_line!('line ' . $i); $i = $i + 1; }\n\
         write_error_line!('finished all iterations');\n",
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the runner starts");

    // Read a little, then close the pipe, which is what `head` does.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut head = [0_u8; 64];
    let _ = stdout.read(&mut head).expect("the first bytes arrive");
    drop(stdout);

    let output = child.wait_with_output().expect("the runner exits");
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        !errors.contains("panicked") && !errors.contains("failed printing"),
        "a closed pipe must not panic or report a Rust error, got: {errors}"
    );
    assert!(
        !errors.contains("finished all iterations"),
        "execution continued after the downstream pipe closed"
    );
    assert!(
        output.status.success(),
        "a closed pipe is a normal end to a pipeline, got {:?}",
        output.status
    );
}

#[test]
fn a_closed_pipe_ends_disassembly_quietly() {
    use std::io::Read;
    use std::process::Stdio;

    let mut source = String::new();
    for _ in 0..20000 {
        source.push_str("$a = 1;\n");
    }
    let path = write_fixture("broken-pipe-dump.whim", &source);
    let mut child = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg("disassemble")
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the runner starts");

    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut head = [0_u8; 64];
    let _ = stdout.read(&mut head).expect("the first bytes arrive");
    drop(stdout);

    let output = child.wait_with_output().expect("the runner exits");
    let _ = fs::remove_file(&path);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        !errors.contains("panicked") && !errors.contains("failed printing"),
        "a closed pipe must not panic or report a Rust error, got: {errors}"
    );
    assert!(
        output.status.success(),
        "a closed pipe is a normal end to a pipeline, got {:?}",
        output.status
    );
}

#[test]
fn arguments_after_the_file_reach_the_program_verbatim() {
    let path = write_fixture(
        "arguments.whim",
        "foreach (Whim\\Env\\get_arguments() as $argument) { write_line!($argument); }\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg(&path)
        .args(["--future-flag", "-x", "plain", "--future-limit"])
        .output()
        .expect("the binary spawns");
    assert_eq!(
        stdout_of(&output),
        "--future-flag\n-x\nplain\n--future-limit\n"
    );
    assert_eq!(code_of(&output), 0);
}

#[test]
fn non_utf8_arguments_reach_the_program_verbatim() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let path = write_fixture(
        "non-utf8-argument.whim",
        "assert!(Whim\\Env\\get_arguments()[0] == \"A\\xFFB\");\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg(&path)
        .arg(OsString::from_vec(vec![b'A', 0xFF, b'B']))
        .output()
        .expect("the binary spawns");

    assert_eq!(output.stdout, b"");
    assert_eq!(stderr_of(&output), "");
    assert_eq!(code_of(&output), 0);
}

#[test]
fn global_options_precede_the_file_and_dashdash_is_optional() {
    let path = write_fixture(
        "separated.whim",
        "write_line!(length!(Whim\\Env\\get_arguments()));\n",
    );
    let direct = Command::new(env!("CARGO_BIN_EXE_whim"))
        .args(["--colors", "never"])
        .arg(&path)
        .arg("one")
        .output()
        .expect("the binary spawns");
    assert_eq!(stdout_of(&direct), "1\n");

    let separated = Command::new(env!("CARGO_BIN_EXE_whim"))
        .args(["--colors", "never", "--"])
        .arg(&path)
        .arg("one")
        .output()
        .expect("the binary spawns");
    assert_eq!(stdout_of(&separated), "1\n");
}

#[test]
fn dashdash_introduces_a_dash_prefixed_entry_file() {
    let path = write_fixture("-program.whim", "write_line!('ran');\n");
    let directory = path.parent().expect("the fixture has a directory");
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(directory)
        .args(["--", "-program.whim"])
        .output()
        .expect("the binary spawns");

    assert_eq!(stdout_of(&output), "ran\n");
    assert_eq!(stderr_of(&output), "");
    assert_eq!(code_of(&output), 0);
    fs::remove_file(&path).expect("the fixture is removable");
}

#[test]
fn a_dash_reads_the_program_from_standard_input() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg("-")
        .arg("given")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary spawns");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(
            b"write_line!(Whim\\Env\\current_script());\n\
              write_line!(Whim\\Env\\get_arguments()[0]);\n",
        )
        .expect("the program is written");
    let output = child.wait_with_output().expect("the runner exits");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "-\ngiven\n");
    assert!(output.status.success());
}

#[test]
fn a_program_from_standard_input_cannot_embed_a_file() {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary spawns");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"write!(embed!('./payload.bin'));\n")
        .expect("the program is written");
    let output = child.wait_with_output().expect("the runner exits");

    assert_eq!(code_of(&output), 255);
    assert!(
        stderr_of(&output).contains("cannot resolve a path in source read from standard input")
    );
}

#[test]
fn a_shebang_line_is_skipped() {
    let output = run_program(
        "shebang.whim",
        "#!/usr/bin/env whim\nwrite_line!('after shebang');\n",
        &[],
    );
    assert_eq!(stdout_of(&output), "after shebang\n");
    assert_eq!(code_of(&output), 0);
}

#[test]
fn the_exit_codes_are_the_documented_ones() {
    assert_eq!(
        code_of(&run_program("ok.whim", "write_line!('x');\n", &[])),
        0
    );
    assert_eq!(code_of(&run_program("three.whim", "exit!(3);\n", &[])), 3);
    // 260 masked to eight bits is 4.
    assert_eq!(code_of(&run_program("wide.whim", "exit!(260);\n", &[])), 4);
    assert_eq!(
        code_of(&run_program(
            "throwing.whim",
            "throw new Whim\\Unwind\\Error('boom', 0);\n",
            &[]
        )),
        255
    );
    assert_eq!(
        code_of(&run_program("broken.whim", "this is not whim @@@\n", &[])),
        255
    );
    let missing = Command::new(env!("CARGO_BIN_EXE_whim"))
        .arg("definitely_not_here.whim")
        .output()
        .expect("the binary spawns");
    assert_eq!(code_of(&missing), 1);
}

#[test]
fn a_function_past_the_register_limit_is_a_compile_error_not_a_panic() {
    let mut source = String::new();
    for index in 0..=65_536_u32 {
        assert!(writeln!(source, "$v{index} = {index};").is_ok());
    }

    let output = run_program("register-limit.whim", &source, &[]);

    assert_eq!(
        code_of(&output),
        255,
        "a file that does not compile exits 255: {}",
        stderr_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(
        !stderr.contains("panicked"),
        "the limit must be reported, not panicked: {stderr}"
    );
    assert!(
        stderr.contains("locals and live temporaries"),
        "the diagnostic should say what ran out: {stderr}"
    );
}

#[test]
fn a_flat_chain_past_the_register_file_is_a_diagnostic() {
    let mut source = String::from("$x = 0");
    for _ in 0..200_000 {
        source.push_str("+1");
    }
    source.push_str(";\n");

    let output = run_program("huge-chain.whim", &source, &[]);
    assert!(
        stderr_of(&output).contains("levels deep"),
        "expected the structural-depth diagnostic, got: {}",
        stderr_of(&output)
    );
    assert_eq!(code_of(&output), 255);
}

fn chain_deeper_than(levels: usize) -> String {
    let mut source = String::from("$x = $y");
    for _ in 0..levels.div_ceil(4) {
        source.push_str("()");
    }
    source.push_str(";\n");

    source
}

#[test]
fn a_tree_past_the_structural_limit_is_a_diagnostic() {
    let source = chain_deeper_than(MAX_STRUCTURAL_DEPTH);

    let output = run_program("too-deep.whim", &source, &[]);
    let errors = stderr_of(&output);
    assert!(
        errors.contains("levels deep"),
        "expected the structural-depth diagnostic, got: {errors}"
    );
    assert!(
        errors.contains("-->"),
        "the diagnostic must be anchored in the source: {errors}"
    );
    assert_eq!(code_of(&output), 255);
}
