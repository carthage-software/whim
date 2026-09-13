use std::env::temp_dir;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::process::id;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

struct Project(PathBuf);

impl Project {
    fn new(manifest: &str) -> Self {
        static ORDINAL: AtomicU32 = AtomicU32::new(0);
        let root = temp_dir().join(format!(
            "whim-lint-{}-{}",
            id(),
            ORDINAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let project = Self(root);
        project.write("whim.toml", &format!("manifest-version = 1\n{manifest}"));
        project
    }

    fn write(&self, name: &str, source: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn lint(&self, paths: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_whim"))
            .current_dir(&self.0)
            .env("WHIM_LOG", "error")
            .args(["--colors", "never", "lint"])
            .args(paths)
            .output()
            .unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lint_reports_without_rewriting_and_uses_mago_severities() {
    let project = Project::new("");
    let source = "// TODO: track this\n$password   =   'secret';\n";
    project.write("source.whim", source);
    let output = project.lint(&[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("error[no-literal-password]"), "{text}");
    assert!(text.contains("warning[tagged-todo]"), "{text}");
    assert!(text.contains("source.whim:2:"), "{text}");
    assert!(
        text.contains("literal value stored in source code"),
        "{text}"
    );
    assert!(text.contains("this name suggests sensitive data"), "{text}");
    assert!(
        text.contains("help: Load the value from an environment variable"),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(project.0.join("source.whim")).unwrap(),
        source
    );
    project.write("source.whim", "// TODO: track this\n");
    assert!(project.lint(&[]).status.success());
    project.write("source.whim", "write_line!('clean');\n");
    let output = project.lint(&[]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn lint_shares_format_discovery_and_applies_rule_options() {
    let project = Project::new(
        r#"
[lint]
include = ["src"]
exclude = ["src/generated"]
minimum_fail_level = "help"
[lint.rules.tagged-todo]
exclude = ["src/ignored.whim"]
[lint.rules.yoda-conditions]
mode = "deny"
[lint.rules.no-literal-password]
enabled = false
"#,
    );
    for path in [
        "src/main.whim",
        "src/generated/code.whim",
        "src/ignored.whim",
        "vendor/pkg/file.whim",
        "other/file.whim",
    ] {
        project.write(path, "// TODO: track this\n");
    }
    project.write(
        "src/yoda.whim",
        "if (5 == $count) {}\n$password = 'secret';\n",
    );
    let output = project.lint(&[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.matches("warning[tagged-todo]").count(), 1, "{text}");
    assert!(text.contains("help[yoda-conditions]"), "{text}");
    assert!(!text.contains("no-literal-password"));
    let output = project.lint(&["other"]);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("other/file.whim")
    );
    let output = project.lint(&["src/generated/code.whim", "src/generated/code.whim"]);
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .matches("warning[tagged-todo]")
            .count(),
        1
    );
}

#[test]
fn syntax_errors_fail_and_valid_files_still_get_linted() {
    let project = Project::new("");
    project.write("broken.whim", "$x = ;\n");
    project.write("valid.whim", "// TODO: track this\n");
    let output = project.lint(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("broken.whim")
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("warning[tagged-todo]")
    );
    assert_eq!(
        fs::read_to_string(project.0.join("broken.whim")).unwrap(),
        "$x = ;\n"
    );
}

#[test]
fn diagnostic_levels_respect_failure_thresholds() {
    for (level, threshold, success) in [
        ("error", "info", false),
        ("warning", "info", false),
        ("info", "error", true),
        ("info", "warning", true),
        ("info", "info", false),
        ("info", "help", false),
        ("info", "note", false),
        ("help", "info", true),
        ("note", "info", true),
        ("note", "help", true),
        ("note", "note", false),
        ("help", "note", false),
        ("help", "warning", true),
    ] {
        let project = Project::new(&format!(
            "[lint]\nminimum_fail_level = '{threshold}'\n[lint.rules.tagged-todo]\nlevel = '{level}'\n"
        ));
        project.write("source.whim", "// TODO: track this\n");
        let output = project.lint(&[]);
        assert_eq!(
            output.status.success(),
            success,
            "{level} against {threshold}: {output:?}"
        );
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains(&format!("{level}[tagged-todo]"))
        );
    }
}

#[test]
fn logs_count_findings_and_failures_and_keep_worker_context() {
    let project = Project::new(
        "[lint.rules.tagged-fixme]\nlevel = 'note'\n[lint.rules.yoda-conditions]\nlevel = 'info'\n",
    );
    for (path, source) in [
        ("clean.whim", "write_line!('clean');\n"),
        ("error.whim", "$password = 'do-not-log-this-value';\n"),
        ("warning.whim", "// TODO: track this\n"),
        ("info.whim", "if ($total == 1) {}\n"),
        ("note.whim", "// FIXME: track this\n"),
        ("help.whim", "$total = $total + 1;\n"),
        ("syntax.whim", "$x = ;\n"),
    ] {
        project.write(path, source);
    }
    fs::write(project.0.join("encoding.whim"), [0xff]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&project.0)
        .env("WHIM_LOG", "whim=trace")
        .env("RAYON_NUM_THREADS", "2")
        .args(["--colors", "never", "lint"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let logs = String::from_utf8(output.stderr).unwrap();
    let summary = logs
        .lines()
        .find(|line| line.contains("finished processing files"))
        .unwrap();
    for field in [
        "processed=8",
        "clean=1",
        "file_errors=2",
        "errors=1",
        "warnings=1",
        "info=1",
        "notes=1",
        "help=1",
        "failed=true",
    ] {
        assert!(summary.contains(field), "{summary}");
    }
    for phase in ["read", "parse", "lint", "render"] {
        assert!(
            logs.lines()
                .any(|line| line.contains(&format!("phase=\"{phase}\""))
                    && line.contains("lint{")
                    && line.contains("pipeline{")
                    && line.contains("file{path=")),
            "{logs}"
        );
    }
    assert!(logs.contains("phase=\"read\" error="), "{logs}");
    assert!(!logs.contains("do-not-log-this-value"));
}

#[test]
fn diagnostics_keep_paths_and_rule_exclusions_from_a_nested_directory() {
    let project = Project::new("[lint.rules.tagged-todo]\nexclude = ['src/ignored.whim']\n");
    project.write("src/source.whim", "// TODO: track this\n");
    project.write("src/ignored.whim", "// TODO: track this\n");
    for paths in [&[][..], &["source.whim", "ignored.whim"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_whim"))
            .current_dir(project.0.join("src"))
            .env("WHIM_LOG", "error")
            .args(["--colors", "never", "lint"])
            .args(paths)
            .output()
            .unwrap();
        assert!(output.status.success());
        let diagnostics = String::from_utf8(output.stdout).unwrap();
        assert_eq!(diagnostics.matches("warning[tagged-todo]").count(), 1);
        let path = if paths.is_empty() {
            project.0.join("src/source.whim").canonicalize().unwrap()
        } else {
            PathBuf::from("source.whim")
        };
        assert!(
            diagnostics.contains(&format!("{}:1:", path.display())),
            "{diagnostics}"
        );
    }
}

#[test]
fn file_commands_check_buffered_output_errors_and_closed_pipes() {
    let project = Project::new("");
    project.write("source.whim", "$password='secret';\n");
    for args in [&["lint"][..], &["fmt", "--check"][..]] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_whim"));
        command
            .current_dir(&project.0)
            .env("WHIM_LOG", "whim=debug")
            .args(["--colors", "never"])
            .args(args)
            .stderr(Stdio::piped());
        let failed = command
            .stdout(Stdio::from(fs::File::open("/dev/null").unwrap()))
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(1));
        let logs = String::from_utf8(failed.stderr).unwrap();
        assert!(logs.contains("could not write command output"), "{logs}");
        assert!(!logs.contains("finished processing files"));
        let mut child = command.stdout(Stdio::piped()).spawn().unwrap();
        drop(child.stdout.take());
        let closed = child.wait_with_output().unwrap();
        assert!(closed.status.success());
        let logs = String::from_utf8(closed.stderr).unwrap();
        assert!(
            logs.contains("stopped because standard output closed"),
            "{logs}"
        );
        assert!(!logs.contains("finished processing files"));
    }
}

#[test]
fn parallel_batches_keep_diagnostic_order_and_explain_skipped_paths() {
    let project = Project::new("[lint]\nexclude = ['src/generated']\n");
    for index in 0..70 {
        project.write(&format!("src/{index:02}.whim"), "// TODO: track this\n");
    }
    project.write("src/generated/ignored.whim", "// TODO: track this\n");
    let output = Command::new(env!("CARGO_BIN_EXE_whim"))
        .current_dir(&project.0)
        .env("WHIM_LOG", "whim=trace")
        .env("RAYON_NUM_THREADS", "2")
        .args(["--colors", "never", "lint", "src", "src"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let diagnostics = String::from_utf8(output.stdout).unwrap();
    let offsets: Vec<_> = (0..70)
        .map(|index| {
            diagnostics
                .find(&format!("src/{index:02}.whim:1:"))
                .unwrap()
        })
        .collect();
    assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(diagnostics.matches("warning[tagged-todo]").count(), 70);
    let logs = String::from_utf8(output.stderr).unwrap();
    for field in [
        "duplicates=70",
        "processed=70",
        "warnings=70",
        "reason=\"exclude filter\"",
        "reason=\"duplicate\"",
        "batch{index=1 files=6}",
    ] {
        assert!(logs.contains(field), "{logs}");
    }
}
