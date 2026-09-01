//! Command-line tests for Git dependencies.

use std::env::temp_dir;
#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::num::NonZeroUsize;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::process::id;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

use url::Url;

struct Fixture {
    directory: PathBuf,
    application: PathBuf,
    remotes: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        static ORDINAL: AtomicU32 = AtomicU32::new(0);

        let ordinal = ORDINAL.fetch_add(1, Ordering::Relaxed);
        let directory = temp_dir().join(format!("whim-package-{name}-{}-{ordinal}", id()));
        let _ = fs::remove_dir_all(&directory);
        let application = directory.join("application");
        let remotes = directory.join("remotes");
        fs::create_dir_all(&application).expect("the application directory is creatable");
        fs::create_dir_all(&remotes).expect("the remote directory is creatable");
        Self {
            directory,
            application,
            remotes,
        }
    }

    fn repository(&self, name: &str, version: &str, namespace: &str) -> PathBuf {
        self.repository_with_dependencies(name, version, namespace, &[])
    }

    fn repository_with_dependencies(
        &self,
        name: &str,
        version: &str,
        namespace: &str,
        dependencies: &[(&str, &str)],
    ) -> PathBuf {
        let work = self.directory.join(format!("{name}-work"));
        fs::create_dir_all(work.join("src")).expect("the source directory is creatable");
        git(None, ["init", work.to_str().expect("UTF-8 fixture path")]);
        git(Some(&work), ["config", "user.email", "test@whim.invalid"]);
        git(Some(&work), ["config", "user.name", "Whim Test"]);
        let mut manifest = format!(
            "manifest-version = 1\n\n[autoload.namespaces]\n\"{namespace}\\\\\" = \"src/\"\n"
        );
        if !dependencies.is_empty() {
            manifest.push_str("\n[dependencies]\n");
            for (dependency, requirement) in dependencies {
                assert!(
                    writeln!(
                        manifest,
                        "\"git+https://fixtures.invalid/{dependency}\" = \"{requirement}\""
                    )
                    .is_ok()
                );
            }
        }
        fs::write(work.join("whim.toml"), manifest).expect("the package manifest is writable");
        fs::write(
            work.join("src/Thing.whim"),
            format!(
                "namespace {namespace};\n\nfinal class Thing {{\n  public function value(): string {{\n    return 'installed';\n  }}\n}}\n"
            ),
        )
        .expect("the package source is writable");
        git(Some(&work), ["add", "."]);
        git(Some(&work), ["commit", "--no-gpg-sign", "-m", "fixture"]);
        git(Some(&work), ["tag", &format!("v{version}")]);
        git(
            None,
            [
                "clone",
                "--bare",
                work.to_str().expect("UTF-8 fixture path"),
                self.remotes
                    .join(format!("{name}.git"))
                    .to_str()
                    .expect("UTF-8 fixture path"),
            ],
        );
        git(
            Some(&work),
            [
                "remote",
                "add",
                "fixture",
                self.remotes
                    .join(format!("{name}.git"))
                    .to_str()
                    .expect("UTF-8 fixture path"),
            ],
        );
        work
    }

    fn tag(work: &Path, version: &str) {
        git(Some(work), ["tag", &format!("v{version}")]);
        git(Some(work), ["push", "fixture", &format!("v{version}")]);
    }

    fn release(work: &Path, version: &str) {
        fs::write(work.join(format!("release-{version}")), version)
            .expect("the release marker is writable");
        git(Some(work), ["add", "."]);
        git(
            Some(work),
            [
                "commit",
                "--no-gpg-sign",
                "-m",
                &format!("release {version}"),
            ],
        );
        Self::tag(work, version);
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.command(arguments)
            .output()
            .expect("the Whim CLI starts")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        self.command_in(&self.application, arguments)
    }

    fn command_in(&self, directory: &Path, arguments: &[&str]) -> Command {
        let remote_base = format!("file://{}/", self.remotes.display());
        let mut command = Command::new(env!("CARGO_BIN_EXE_whim"));
        command
            .current_dir(directory)
            .args(arguments)
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", format!("url.{remote_base}.insteadOf"))
            .env("GIT_CONFIG_VALUE_0", "https://fixtures.invalid/")
            .env("GIT_CONFIG_KEY_1", "protocol.file.allow")
            .env("GIT_CONFIG_VALUE_1", "always");
        command
    }
}

struct WaitingCommand {
    child: Child,
    waiting: Receiver<()>,
    stderr: JoinHandle<Vec<u8>>,
}

impl WaitingCommand {
    fn spawn(mut command: Command) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the Whim CLI starts");
        let stderr = child.stderr.take().expect("standard error is piped");
        let (sender, waiting) = sync_channel(1);
        let stderr = thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut output = Vec::new();
            let mut line = Vec::new();
            let mut reported = false;
            loop {
                line.clear();
                let read = reader
                    .read_until(b'\n', &mut line)
                    .expect("standard error is readable");
                if read == 0 {
                    break;
                }

                if !reported
                    && line
                        .windows(b"waiting for another package command".len())
                        .any(|window| window == b"waiting for another package command")
                {
                    reported = true;
                    let _ = sender.send(());
                }
                output.extend_from_slice(&line);
            }
            output
        });

        Self {
            child,
            waiting,
            stderr,
        }
    }

    fn wait_until_blocked(&mut self) {
        if self.waiting.recv_timeout(Duration::from_secs(10)).is_ok() {
            return;
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
        panic!("the package command did not wait for the dependency lock");
    }

    fn finish(self) -> Output {
        let mut output = self.child.wait_with_output().expect("the Whim CLI exits");
        output.stderr = self.stderr.join().expect("the error reader exits");
        output
    }
}

#[test]
fn init_preserves_gitignore_and_refuses_to_overwrite_the_manifest() {
    let fixture = Fixture::new("init");
    fs::write(fixture.application.join(".gitignore"), "/target/\n")
        .expect("the ignore file is writable");
    assert_success(&fixture.run(&["init"]));

    assert_eq!(
        fs::read_to_string(fixture.application.join(".gitignore"))
            .expect("the ignore file is readable"),
        "/target/\n/vendor/\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.application.join(".gitattributes"))
            .expect("the attributes file is readable"),
        "/tests export-ignore\n/.gitattributes export-ignore\n/.gitignore export-ignore\n/whim.lock export-ignore\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.application.join("src/main.whim"))
            .expect("the source entry point is readable"),
        "write_line!('Hello, Whim!');\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.application.join("tests/main.whim"))
            .expect("the test entry point is readable"),
        "assert!(true);\n"
    );
    assert!(fixture.application.join(".git").is_dir());
    assert_success(&fixture.run(&["fmt", "--check"]));
    let source = fixture.run(&["src/main.whim"]);
    assert_success(&source);
    assert_eq!(source.stdout, b"Hello, Whim!\n");
    assert_success(&fixture.run(&["tests/main.whim"]));

    let manifest =
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable");
    let output = fixture.run(&["init"]);
    assert!(!output.status.success());
    assert_eq!(
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable"),
        manifest
    );
}

#[test]
fn init_ignores_ambient_project_and_runtime_configuration() {
    let ancestor = Fixture::new("init-ambient-manifest");
    fs::write(ancestor.application.join("whim.toml"), "not valid TOML")
        .expect("the invalid ancestor manifest is writable");
    let child = ancestor.application.join("child");
    fs::create_dir(&child).expect("the child directory is creatable");
    let output = ancestor
        .command_in(&child, &["init", "--no-git"])
        .output()
        .expect("the Whim CLI starts");
    assert_success(&output);
    assert!(child.join("whim.toml").is_file());

    let environment = Fixture::new("init-ambient-environment");
    let output = environment
        .command(&["init", "--no-git"])
        .env("WHIM_CALL_DEPTH", "many")
        .output()
        .expect("the Whim CLI starts");
    assert_success(&output);
    assert!(environment.application.join("whim.toml").is_file());

    let explicit = Fixture::new("init-explicit-configuration");
    let output = explicit.run(&["--config", "missing-whim.toml", "init", "--no-git"]);
    assert_success(&output);
    assert!(explicit.application.join("whim.toml").is_file());
}

#[test]
#[cfg(target_os = "linux")]
fn package_cache_supports_non_utf8_project_paths() {
    let fixture = Fixture::new("non-utf8-project");
    fixture.repository("library", "1.0.0", "NonUtf8Fixture");
    let application = fixture
        .directory
        .join(OsString::from_vec(b"application-\xff".to_vec()));
    fs::rename(&fixture.application, &application)
        .expect("the application can move beneath a non-UTF-8 path");

    let init = fixture
        .command_in(&application, &["init", "--no-git"])
        .output()
        .expect("the Whim CLI starts");
    assert_success(&init);
    let add = fixture
        .command_in(
            &application,
            &[
                "add",
                "https://fixtures.invalid/library.git",
                "--version",
                "^1",
            ],
        )
        .output()
        .expect("the Whim CLI starts");
    assert_success(&add);
    assert!(application.join("whim.lock").is_file());
}

#[test]
fn init_does_not_follow_project_links() {
    let linked_directory = Fixture::new("init-linked-directory");
    let outside = linked_directory.directory.join("outside-source");
    fs::create_dir(&outside).expect("the outside directory is creatable");
    symlink(&outside, linked_directory.application.join("src"))
        .expect("the source link is creatable");

    let output = linked_directory.run(&["init", "--no-git"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not a directory"));
    assert!(!outside.join("main.whim").exists());
    assert!(!linked_directory.application.join("whim.toml").exists());

    let linked_file = Fixture::new("init-linked-file");
    let outside = linked_file.directory.join("outside-ignore");
    fs::write(&outside, "/outside/\n").expect("the outside ignore file is writable");
    symlink(&outside, linked_file.application.join(".gitignore"))
        .expect("the ignore link is creatable");

    let output = linked_file.run(&["init"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not a file"));
    assert_eq!(
        fs::read_to_string(&outside).expect("the outside ignore file is readable"),
        "/outside/\n",
    );
    assert!(!linked_file.application.join("whim.toml").exists());
}

#[test]
fn init_preserves_a_dangling_git_link() {
    let fixture = Fixture::new("init-dangling-git");
    let target = fixture.directory.join("missing-git-directory");
    let git = fixture.application.join(".git");
    symlink(&target, &git).expect("the dangling Git link is creatable");

    let output = fixture.run(&["init"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid Git worktree metadata"));
    assert_eq!(
        fs::read_link(&git).expect("the Git link still exists"),
        target
    );
    assert!(!fixture.application.join("whim.toml").exists());
}

#[test]
fn dependency_edits_do_not_replace_a_symbolic_link_manifest() {
    let fixture = Fixture::new("linked-manifest");
    let target = fixture.application.join("project.toml");
    let manifest = "manifest-version = 1\n";
    fs::write(&target, manifest).expect("the manifest target is writable");
    symlink("project.toml", fixture.application.join("whim.toml"))
        .expect("the manifest link is creatable");

    let output = fixture.run(&[
        "add",
        "git+file:///tmp/whim-linked-manifest-fixture",
        "--version",
        "^1.0",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot edit symbolic-link manifest"));
    assert!(
        fs::symlink_metadata(fixture.application.join("whim.toml"))
            .expect("the manifest link still exists")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&target).expect("the manifest target is readable"),
        manifest
    );

    let output = fixture.run(&[
        "--config",
        "whim.toml",
        "add",
        "git+file:///tmp/whim-linked-manifest-fixture",
        "--version",
        "^1.0",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot edit symbolic-link manifest"));
    assert_eq!(
        fs::read_to_string(target).expect("the manifest target is readable"),
        manifest
    );
}

#[test]
fn package_commands_do_not_follow_dependency_manager_links() {
    for relative in [
        "vendor",
        "vendor/.whim",
        "vendor/.whim/git",
        "vendor/.whim/stages",
    ] {
        let fixture = Fixture::new("linked-manager");
        assert_success(&fixture.run(&["init", "--no-git"]));

        let outside = fixture.directory.join("outside-manager");
        fs::create_dir(&outside).expect("the outside directory is creatable");
        let sentinel = outside.join("stage-important");
        fs::create_dir(&sentinel).expect("the outside stage is creatable");
        fs::write(sentinel.join("data"), b"preserve").expect("the outside stage data is writable");
        let link = fixture.application.join(relative);
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent).expect("the link parent is creatable");
        }
        symlink(&outside, &link).expect("the manager link is creatable");

        let output = fixture.run(&["install"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("must be a real directory"),);
        assert_eq!(
            fs::read(sentinel.join("data")).expect("the outside stage data is readable"),
            b"preserve",
        );
    }
}

#[test]
fn init_without_git_preserves_sources_and_git_files() {
    let fixture = Fixture::new("init-no-git");
    fs::create_dir(fixture.application.join("src")).expect("the source directory is creatable");
    fs::create_dir(fixture.application.join("tests")).expect("the test directory is creatable");
    fs::write(
        fixture.application.join("src/main.whim"),
        "write_line!('Existing');\n",
    )
    .expect("the source entry point is writable");
    fs::write(
        fixture.application.join("tests/main.whim"),
        "assert!(42 == 42);\n",
    )
    .expect("the test entry point is writable");
    fs::write(fixture.application.join(".gitignore"), "/existing/\n")
        .expect("the ignore file is writable");
    fs::write(
        fixture.application.join(".gitattributes"),
        "/existing export-ignore\n",
    )
    .expect("the attributes file is writable");

    let mut command = fixture.command(&["init", "--no-git"]);
    let output = command
        .env("PATH", "/definitely/missing")
        .output()
        .expect("the Whim CLI starts");
    assert_success(&output);

    assert!(!fixture.application.join(".git").exists());
    assert_eq!(
        fs::read_to_string(fixture.application.join(".gitignore"))
            .expect("the ignore file is readable"),
        "/existing/\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.application.join(".gitattributes"))
            .expect("the attributes file is readable"),
        "/existing export-ignore\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.application.join("src/main.whim"))
            .expect("the source entry point is readable"),
        "write_line!('Existing');\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.application.join("tests/main.whim"))
            .expect("the test entry point is readable"),
        "assert!(42 == 42);\n"
    );
}

#[test]
fn init_ignores_ambient_git_repository_selection() {
    let fixture = Fixture::new("init-ambient-git");
    let unrelated = fixture.directory.join("unrelated.git");
    git(
        None,
        [
            "init",
            "--bare",
            unrelated.to_str().expect("UTF-8 fixture path"),
        ],
    );

    let output = fixture
        .command(&["init"])
        .env("GIT_DIR", &unrelated)
        .env("GIT_NAMESPACE", "unrelated")
        .output()
        .expect("the Whim CLI starts");

    assert_success(&output);
    assert!(fixture.application.join(".git").is_dir());
}

#[test]
fn init_reuses_an_existing_git_worktree() {
    let fixture = Fixture::new("init-existing-git");
    git(
        None,
        [
            "init",
            fixture.directory.to_str().expect("UTF-8 fixture directory"),
        ],
    );
    fs::write(
        fixture.application.join(".gitattributes"),
        "/README.md export-ignore\n/tests export-ignore\n",
    )
    .expect("the attributes file is writable");
    fs::write(
        fixture.application.join(".gitignore"),
        "/existing/\n/vendor/\n",
    )
    .expect("the ignore file is writable");

    assert_success(&fixture.run(&["init"]));

    assert!(fixture.directory.join(".git").is_dir());
    assert!(!fixture.application.join(".git").exists());
    assert_eq!(
        fs::read_to_string(fixture.application.join(".gitignore"))
            .expect("the ignore file is readable"),
        "/existing/\n/vendor/\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.application.join(".gitattributes"))
            .expect("the attributes file is readable"),
        "/README.md export-ignore\n/tests export-ignore\n/.gitattributes export-ignore\n/.gitignore export-ignore\n/whim.lock export-ignore\n"
    );
}

#[test]
fn init_does_not_write_project_files_when_git_is_unavailable() {
    let fixture = Fixture::new("init-missing-git");
    let mut command = fixture.command(&["init"]);
    let output = command
        .env("PATH", "/definitely/missing")
        .output()
        .expect("the Whim CLI starts");

    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("Git is unavailable"), "{error}");
    assert!(error.contains("whim init --no-git"), "{error}");
    for path in [
        ".git",
        ".gitignore",
        ".gitattributes",
        "whim.toml",
        "src",
        "tests",
    ] {
        assert!(!fixture.application.join(path).exists(), "{path} exists");
    }
}

#[test]
fn invalid_package_arguments_do_not_create_manager_state() {
    let fixture = Fixture::new("invalid-package-arguments");
    assert_success(&fixture.run(&["init", "--no-git"]));

    let add = fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "not-a-requirement",
    ]);
    assert!(!add.status.success());
    assert!(
        String::from_utf8_lossy(&add.stderr).contains("not-a-requirement"),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    assert!(!fixture.application.join("vendor").exists());

    let update = fixture.run(&["update", "not-a-source"]);
    assert!(!update.status.success());
    assert!(
        String::from_utf8_lossy(&update.stderr).contains("only HTTPS, SSH"),
        "{}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert!(!fixture.application.join("vendor").exists());
}

#[test]
fn add_installs_locks_and_autoloads_a_git_release() {
    let fixture = Fixture::new("add");
    fixture.repository("library", "1.2.3", "Fixture");

    assert_success(&fixture.run(&["init"]));
    fs::write(
        fixture.application.join("src/Root.whim"),
        "namespace App;\n\nfinal class Root {\n  public function value(): string {\n    return 'root';\n  }\n}\n",
    )
    .expect("the root source is writable");
    let mut root_manifest = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the manifest is readable");
    root_manifest.push_str("\n[autoload.namespaces]\n\"App\\\\\" = \"src/\"\n");
    fs::write(fixture.application.join("whim.toml"), root_manifest)
        .expect("the manifest is writable");
    assert_success(&fixture.run(&["add", "https://fixtures.invalid/library.git"]));

    let manifest = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the manifest is readable");
    assert!(manifest.contains("git+https://fixtures.invalid/library"));
    assert!(manifest.contains("^1.2"));
    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert!(lock.contains("version = \"1.2.3\""));
    assert!(lock.contains("checksum = \"blake3:"));

    fs::write(
        fixture.application.join("application.whim"),
        "require_once!('vendor/autoload.whim');\n\nwrite_line!(new App\\Root()->value());\nwrite_line!(new Fixture\\Thing()->value());\n",
    )
    .expect("the application source is writable");
    let output = fixture.run(&["application.whim"]);
    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "root\ninstalled\n");

    let package = fs::read_dir(fixture.application.join("vendor/packages"))
        .expect("the package directory is readable")
        .next()
        .expect("one package is installed")
        .expect("the package entry is readable")
        .path();
    fs::write(package.join("src/Thing.whim"), "tampered")
        .expect("the installed source is writable");
    assert_success(&fixture.run(&["install"]));
    assert_ne!(
        fs::read_to_string(package.join("src/Thing.whim"))
            .expect("the restored source is readable"),
        "tampered"
    );

    fs::remove_dir_all(&fixture.remotes).expect("the remote can disappear");
    assert_success(&fixture.run(&["install"]));
}

#[test]
fn install_reports_parallel_work_and_keeps_staging_out_of_the_project_root() {
    let fixture = Fixture::new("progress");
    fixture.repository("first", "1.0.0", "FirstProgress");
    fixture.repository("second", "1.0.0", "SecondProgress");
    assert_success(&fixture.run(&["init"]));

    let mut manifest = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the manifest is readable");
    manifest.push_str(
        "\n[dependencies]\n\"git+https://fixtures.invalid/first\" = \"^1\"\n\"git+https://fixtures.invalid/second\" = \"^1\"\n",
    );
    fs::write(fixture.application.join("whim.toml"), manifest).expect("the manifest is writable");

    let output = fixture.run(&["install"]);
    assert_success(&output);
    let progress = String::from_utf8_lossy(&output.stderr);
    for message in [
        "resolving dependencies",
        "loading package releases",
        "fetching releases",
        "preparing dependencies",
        "preparing package=",
        "applying dependency changes",
        "resolved and installed 2 repositories",
    ] {
        assert!(
            progress.contains(message),
            "missing `{message}` in:\n{progress}"
        );
    }
    let workers = thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(2);
    assert!(
        progress.contains(&format!("workers={workers}")),
        "{progress}"
    );

    let root_entries = fs::read_dir(&fixture.application)
        .expect("the project root is readable")
        .collect::<Result<Vec<_>, _>>()
        .expect("the project root entries are readable");
    assert!(root_entries.iter().all(|entry| {
        !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".whim-stage-")
    }));
}

#[test]
fn release_fetch_errors_name_the_repository() {
    let fixture = Fixture::new("fetch-error");
    assert_success(&fixture.run(&["init"]));

    let mut manifest = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the manifest is readable");
    manifest.push_str("\n[dependencies]\n\"git+https://fixtures.invalid/missing\" = \"^1\"\n");
    fs::write(fixture.application.join("whim.toml"), manifest).expect("the manifest is writable");

    let output = fixture.run(&["install"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("could not load releases from `git+https://fixtures.invalid/missing`"),
        "{error}"
    );
}

#[test]
fn explicit_file_git_sources_use_the_normal_dependency_pipeline() {
    let fixture = Fixture::new("file-source");
    fixture.repository("library", "1.2.3", "FileFixture");
    let remote = fixture.remotes.join("library.git");
    let source = format!(
        "git+{}",
        Url::from_file_path(&remote).expect("the fixture path is absolute")
    );

    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&["add", &source]));

    let manifest = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the manifest is readable");
    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert!(manifest.contains(&source));
    assert!(lock.contains(&format!("source = \"{source}\"")));

    fs::remove_dir_all(remote).expect("the source repository can disappear");
    assert_success(&fixture.run(&["install"]));
}

#[test]
fn install_fetches_a_locked_commit_into_an_empty_cache() {
    let fixture = Fixture::new("cold-locked-install");
    fixture.repository("library", "1.0.0", "ColdLockedFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "^1",
    ]));

    fs::remove_dir_all(fixture.application.join("vendor"))
        .expect("the installed dependency and Git cache are removable");
    assert_success(&fixture.run(&["install"]));

    let loader = fs::read_to_string(fixture.application.join("vendor/autoload.whim"))
        .expect("the rebuilt autoloader is readable");
    assert!(loader.contains("ColdLockedFixture"));
}

#[test]
fn no_dev_excludes_development_packages_from_vendor_and_loader() {
    let fixture = Fixture::new("no-dev");
    fixture.repository("runtime", "1.0.0", "RuntimeFixture");
    fixture.repository("testing", "1.0.0", "TestingFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/runtime.git",
        "--version",
        "^1",
    ]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/testing.git",
        "--version",
        "^1",
        "--dev",
    ]));
    assert_success(&fixture.run(&["install", "--no-dev"]));

    let autoload = fs::read_to_string(fixture.application.join("vendor/autoload.whim"))
        .expect("the autoloader is readable");
    assert!(autoload.contains("RuntimeFixture"));
    assert!(!autoload.contains("TestingFixture"));
    let packages = fs::read_dir(fixture.application.join("vendor/packages"))
        .expect("the package directory is readable")
        .count();
    assert_eq!(packages, 1);

    let runtime = fixture.run(&["why", "https://fixtures.invalid/runtime.git"]);
    assert_success(&runtime);
    let runtime = String::from_utf8_lossy(&runtime.stdout);
    assert!(runtime.contains("is installed because"), "{runtime}");

    let development = fixture.run(&["why", "https://fixtures.invalid/testing.git"]);
    assert_success(&development);
    let development = String::from_utf8_lossy(&development.stdout);
    assert!(
        development.contains("is locked but not installed. It is required because"),
        "{development}"
    );
    assert!(
        development.contains("the root project requires it for development"),
        "{development}"
    );

    let runtime = fixture.run(&["show", "https://fixtures.invalid/runtime.git"]);
    assert_success(&runtime);
    let runtime = String::from_utf8_lossy(&runtime.stdout);
    assert!(runtime.contains("version         : 1.0.0"), "{runtime}");

    let development = fixture.run(&["show", "https://fixtures.invalid/testing.git"]);
    assert!(!development.status.success());
    assert!(
        String::from_utf8_lossy(&development.stderr).contains("is locked but not installed"),
        "{}",
        String::from_utf8_lossy(&development.stderr)
    );
}

#[test]
fn remove_rebuilds_every_dependency_generation() {
    let fixture = Fixture::new("remove");
    fixture.repository("first", "1.0.0", "FirstRemovalFixture");
    fixture.repository("second", "1.0.0", "SecondRemovalFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/first.git",
        "--version",
        "^1",
    ]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/second.git",
        "--version",
        "^1",
    ]));
    assert_success(&fixture.run(&["remove", "https://fixtures.invalid/first.git"]));

    let manifest = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the manifest is readable");
    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    let loader = fs::read_to_string(fixture.application.join("vendor/autoload.whim"))
        .expect("the loader is readable");
    assert!(!manifest.contains("fixtures.invalid/first"));
    assert!(!lock.contains("fixtures.invalid/first"));
    assert!(!loader.contains("FirstRemovalFixture"));
    assert!(manifest.contains("fixtures.invalid/second"));
    assert!(lock.contains("fixtures.invalid/second"));
    assert!(loader.contains("SecondRemovalFixture"));
    assert_eq!(
        fs::read_dir(fixture.application.join("vendor/packages"))
            .expect("the package directory is readable")
            .count(),
        1
    );
}

#[test]
fn a_resolution_failure_leaves_every_generation_unchanged() {
    let fixture = Fixture::new("transaction");
    let common = fixture.repository("common", "1.0.0", "CommonFixture");
    Fixture::tag(&common, "2.0.0");
    fixture.repository_with_dependencies("first", "1.0.0", "FirstFixture", &[("common", "^1")]);
    fixture.repository_with_dependencies("second", "1.0.0", "SecondFixture", &[("common", "^2")]);
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/first.git",
        "--version",
        "^1",
    ]));
    let manifest =
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable");
    let lock = fs::read(fixture.application.join("whim.lock")).expect("the lockfile is readable");
    let loader = fs::read(fixture.application.join("vendor/autoload.whim"))
        .expect("the autoloader is readable");

    let output = fixture.run(&[
        "add",
        "https://fixtures.invalid/second.git",
        "--version",
        "^1",
    ]);
    assert!(!output.status.success());
    assert_eq!(
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable"),
        manifest
    );
    assert_eq!(
        fs::read(fixture.application.join("whim.lock")).expect("the lockfile is readable"),
        lock
    );
    assert_eq!(
        fs::read(fixture.application.join("vendor/autoload.whim"))
            .expect("the autoloader is readable"),
        loader
    );
}

#[test]
fn an_override_keeps_the_logical_source_and_uses_the_replacement() {
    let fixture = Fixture::new("override");
    fixture.repository("replacement", "1.0.0", "ReplacementFixture");
    assert_success(&fixture.run(&["init"]));
    let mut manifest = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the manifest is readable");
    manifest.push_str(
        "\n[overrides]\n\"git+https://fixtures.invalid/original\" = \"git+https://fixtures.invalid/replacement\"\n",
    );
    fs::write(fixture.application.join("whim.toml"), manifest).expect("the manifest is writable");
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/original.git",
        "--version",
        "^1",
    ]));

    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert!(lock.contains("source = \"git+https://fixtures.invalid/original\""));
    assert!(lock.contains("resolved-source = \"git+https://fixtures.invalid/replacement\""));
    let loader = fs::read_to_string(fixture.application.join("vendor/autoload.whim"))
        .expect("the autoloader is readable");
    assert!(loader.contains("ReplacementFixture"));

    let show = fixture.run(&["show", "https://fixtures.invalid/original.git"]);
    assert_success(&show);
    let show = String::from_utf8_lossy(&show.stdout);
    assert!(
        show.contains("resolved source : git+https://fixtures.invalid/replacement"),
        "{show}"
    );
}

#[test]
fn targeted_update_unlocks_the_requested_source() {
    let fixture = Fixture::new("update");
    let work = fixture.repository("library", "1.0.0", "UpdateFixture");
    let other = fixture.repository("other", "1.0.0", "OtherFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "^1",
    ]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/other.git",
        "--version",
        "^1",
    ]));
    Fixture::release(&work, "1.1.0");
    Fixture::release(&other, "1.1.0");
    assert_success(&fixture.run(&["update", "https://fixtures.invalid/library.git"]));

    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert_eq!(
        locked_version(&lock, "git+https://fixtures.invalid/library"),
        "1.1.0"
    );
    assert_eq!(
        locked_version(&lock, "git+https://fixtures.invalid/other"),
        "1.0.0"
    );
}

#[test]
fn update_refreshes_a_lock_after_the_manifest_changes() {
    let fixture = Fixture::new("update-stale-lock");
    let work = fixture.repository("library", "1.0.0", "UpdateStaleFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "=1.0.0",
    ]));
    Fixture::release(&work, "1.1.0");

    let manifest_path = fixture.application.join("whim.toml");
    let manifest = fs::read_to_string(&manifest_path).expect("the manifest is readable");
    fs::write(&manifest_path, manifest.replace("=1.0.0", "^1")).expect("the manifest is writable");

    assert_success(&fixture.run(&["update"]));
    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert_eq!(
        locked_version(&lock, "git+https://fixtures.invalid/library"),
        "1.1.0"
    );
}

#[test]
fn prereleases_require_explicit_permission_and_zero_carets_stay_in_their_minor() {
    let fixture = Fixture::new("versions");
    let work = fixture.repository("library", "0.2.0", "VersionFixture");
    Fixture::release(&work, "0.2.1");
    Fixture::release(&work, "0.3.0");
    Fixture::release(&work, "1.0.0-alpha.1");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "^0.2",
    ]));
    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert_eq!(
        locked_version(&lock, "git+https://fixtures.invalid/library"),
        "0.2.1"
    );

    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "=1.0.0-alpha.1",
    ]));
    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert_eq!(
        locked_version(&lock, "git+https://fixtures.invalid/library"),
        "1.0.0-alpha.1"
    );
}

#[test]
fn ambiguous_version_tags_are_rejected_but_equivalent_spellings_may_share_a_commit() {
    let fixture = Fixture::new("ambiguous-tags");
    let work = fixture.repository("library", "1.0.0", "TagFixture");
    git(Some(&work), ["tag", "1.0.0"]);
    git(Some(&work), ["push", "fixture", "1.0.0"]);
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "^1",
    ]));

    Fixture::release(&work, "1.1.0");
    fs::write(work.join("ambiguous"), "different commit")
        .expect("the ambiguity marker is writable");
    git(Some(&work), ["add", "."]);
    git(
        Some(&work),
        ["commit", "--no-gpg-sign", "-m", "ambiguous tag"],
    );
    git(Some(&work), ["tag", "1.1.0"]);
    git(Some(&work), ["push", "fixture", "1.1.0"]);

    let output = fixture.run(&["update"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("different commits"));
}

#[test]
fn incompatible_engines_and_dependency_cycles_are_rejected() {
    let incompatible = Fixture::new("engine-requirement");
    let work = incompatible.repository("library", "1.0.0", "EngineFixture");
    fs::write(
        work.join("whim.toml"),
        "manifest-version = 1\n\n[requirements]\nwhim = \"^99\"\n\n[autoload.namespaces]\n\"EngineFixture\\\\\" = \"src/\"\n",
    )
    .expect("the manifest is writable");
    Fixture::release(&work, "1.1.0");
    assert_success(&incompatible.run(&["init"]));
    let output = incompatible.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "=1.1.0",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires Whim"));

    let cycle = Fixture::new("cycle");
    let first = cycle.repository("first", "1.0.0", "FirstCycle");
    cycle.repository_with_dependencies("second", "1.0.0", "SecondCycle", &[("first", "^1")]);
    fs::write(
        first.join("whim.toml"),
        "manifest-version = 1\n\n[autoload.namespaces]\n\"FirstCycle\\\\\" = \"src/\"\n\n[dependencies]\n\"git+https://fixtures.invalid/second\" = \"^1\"\n",
    )
    .expect("the cyclic manifest is writable");
    Fixture::release(&first, "1.1.0");
    assert_success(&cycle.run(&["init"]));
    let output = cycle.run(&[
        "add",
        "https://fixtures.invalid/first.git",
        "--version",
        "=1.1.0",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cycle"));
}

#[test]
fn concurrent_installs_serialize_and_generate_identical_output() {
    let fixture = Fixture::new("concurrent");
    fixture.repository("library", "1.0.0", "ConcurrentFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "^1",
    ]));
    let lock = fs::read(fixture.application.join("whim.lock")).expect("the lockfile is readable");
    let loader = fs::read(fixture.application.join("vendor/autoload.whim"))
        .expect("the autoloader is readable");

    let first = fixture
        .command(&["install"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("install starts");
    let second = fixture
        .command(&["install"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("install starts");
    assert_success(&first.wait_with_output().expect("install exits"));
    assert_success(&second.wait_with_output().expect("install exits"));
    assert_eq!(
        fs::read(fixture.application.join("whim.lock")).expect("the lockfile is readable"),
        lock
    );
    assert_eq!(
        fs::read(fixture.application.join("vendor/autoload.whim"))
            .expect("the autoloader is readable"),
        loader
    );
}

#[test]
fn update_rejects_a_moved_tag() {
    let fixture = Fixture::new("moved-tag");
    let work = fixture.repository("library", "1.0.0", "MovedFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "=1.0.0",
    ]));
    fs::write(work.join("mutation"), "changed").expect("the mutation is writable");
    git(Some(&work), ["add", "."]);
    git(Some(&work), ["commit", "--no-gpg-sign", "-m", "mutate tag"]);
    git(Some(&work), ["tag", "--force", "v1.0.0"]);
    git(Some(&work), ["push", "--force", "fixture", "v1.0.0"]);

    let output = fixture.run(&["update"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("security error"), "{error}");
    assert!(error.contains("moved from"), "{error}");
}

#[test]
fn generated_loader_supports_every_grouped_symbol_kind() {
    let fixture = Fixture::new("grouped");
    let work = fixture.repository("grouped", "1.0.0", "Grouped");
    fs::write(
        work.join("src/classes.whim"),
        "namespace Grouped;\n\nfinal class GroupedClass {}\n",
    )
    .expect("the grouped classes are writable");
    fs::write(
        work.join("src/interfaces.whim"),
        "namespace Grouped;\n\ninterface GroupedInterface {}\n",
    )
    .expect("the grouped interfaces are writable");
    fs::write(
        work.join("src/enums.whim"),
        "namespace Grouped;\n\nenum GroupedEnum: string {\n  case Value = 'value';\n}\n",
    )
    .expect("the grouped enums are writable");
    fs::write(
        work.join("src/types.whim"),
        "namespace Grouped;\n\ntype GroupedAlias = string;\nnewtype GroupedNewtype = int;\n",
    )
    .expect("the grouped types are writable");
    fs::write(
        work.join("src/functions.whim"),
        "namespace Grouped;\n\nfunction grouped_function(): string {\n  return 'function';\n}\n",
    )
    .expect("the grouped functions are writable");
    fs::write(
        work.join("src/constants.whim"),
        "namespace Grouped;\n\nconst GROUPED_CONSTANT = 'constant';\n",
    )
    .expect("the grouped constants are writable");
    Fixture::release(&work, "1.1.0");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/grouped.git",
        "--version",
        "^1",
    ]));
    fs::write(
        fixture.application.join("application.whim"),
        "final class Local implements Grouped\\GroupedInterface {}\nfunction accept(Grouped\\GroupedAlias $value): string { return $value; }\n\n$class = new Grouped\\GroupedClass();\n$enum = Grouped\\GroupedEnum::Value;\n$newtype = Grouped\\GroupedNewtype(1);\n$local = new Local();\nwrite_line!(accept(Grouped\\grouped_function()));\nwrite_line!(Grouped\\GROUPED_CONSTANT);\n",
    )
    .expect("the application is writable");
    fs::write(
        fixture.application.join("entry.whim"),
        "require_once!('vendor/autoload.whim');\nrequire_once!('application.whim');\n",
    )
    .expect("the entry source is writable");

    let output = fixture.run(&["entry.whim"]);
    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "function\nconstant\n"
    );
}

#[test]
fn generated_loader_does_not_load_unrelated_symbol_groups() {
    let fixture = Fixture::new("group-isolation");
    let work = fixture.repository("group-isolation", "1.0.0", "Grouped");
    fs::write(
        work.join("src/classes.whim"),
        "namespace Grouped;\n\nfinal class Other {}\n",
    )
    .expect("the grouped classes are writable");
    fs::write(
        work.join("src/interfaces.whim"),
        "panic!('an unrelated symbol group was loaded');\n",
    )
    .expect("the grouped interfaces are writable");
    Fixture::release(&work, "1.1.0");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/group-isolation.git",
        "--version",
        "^1",
    ]));
    fs::write(
        fixture.application.join("entry.whim"),
        "require_once!('vendor/autoload.whim');\n\nuse Whim\\Autoload;\nuse Whim\\Symbol\\SymbolKind;\n\nassert!(!Autoload\\load_symbol(SymbolKind::Class, 'Grouped\\Missing'));\nwrite_line!('ok');\n",
    )
    .expect("the entry source is writable");

    let output = fixture.run(&["entry.whim"]);
    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}

#[test]
fn unsafe_git_trees_do_not_change_dependency_state() {
    let fixture = Fixture::new("unsafe-tree");
    let work = fixture.repository("unsafe", "1.0.0", "UnsafeFixture");
    symlink("Thing.whim", work.join("src/link.whim")).expect("the symbolic link is creatable");
    Fixture::release(&work, "1.1.0");
    assert_success(&fixture.run(&["init"]));
    let manifest =
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable");

    let output = fixture.run(&[
        "add",
        "https://fixtures.invalid/unsafe.git",
        "--version",
        "^1",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a regular file"));
    assert_eq!(
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable"),
        manifest
    );
    assert!(!fixture.application.join("whim.lock").exists());
}

#[test]
fn run_reads_configuration_but_not_dependency_state() {
    let fixture = Fixture::new("run-boundary");
    fs::write(
        fixture.application.join("whim.toml"),
        "manifest-version = 1\n[runtime]\noptimizations = \"off\"\n",
    )
    .expect("the manifest is writable");
    fs::write(fixture.application.join("whim.lock"), "this is not TOML")
        .expect("the malformed lock is writable");
    fs::create_dir(fixture.application.join("vendor")).expect("the vendor directory is creatable");
    fs::write(
        fixture.application.join("vendor/autoload.whim"),
        "this is not Whim",
    )
    .expect("the malformed loader is writable");
    fs::write(
        fixture.application.join("application.whim"),
        "write_line!('ran');\n",
    )
    .expect("the application is writable");

    let output = fixture.run(&["application.whim"]);
    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ran\n");
}

#[test]
fn preexisting_transaction_markers_fail_without_changing_project_files() {
    let fixture = Fixture::new("untrusted-transaction");
    fixture.repository("library", "1.0.0", "RecoveryFixture");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "^1",
    ]));
    let baseline_lock =
        fs::read(fixture.application.join("whim.lock")).expect("the lockfile is readable");
    let baseline_manifest =
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable");
    let baseline_loader =
        fs::read(fixture.application.join("vendor/autoload.whim")).expect("the loader is readable");
    let baseline_state = fs::read(fixture.application.join("vendor/.whim/state.toml"))
        .expect("the installation state is readable");
    let manager = fixture.application.join("vendor/.whim");
    fs::write(manager.join("transaction.pending"), "untrusted")
        .expect("the forged marker is writable");

    let output = fixture.run(&["install"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("interrupted or untrusted dependency transaction")
    );
    assert_eq!(
        fs::read(fixture.application.join("whim.lock")).expect("the lockfile is readable"),
        baseline_lock
    );
    assert_eq!(
        fs::read(fixture.application.join("whim.toml")).expect("the manifest is readable"),
        baseline_manifest
    );
    assert_eq!(
        fs::read(fixture.application.join("vendor/autoload.whim")).expect("the loader is readable"),
        baseline_loader
    );
    assert_eq!(
        fs::read(manager.join("state.toml")).expect("the installation state is readable"),
        baseline_state
    );
    assert!(fixture.application.join("vendor/packages").is_dir());
    assert!(manager.join("transaction.pending").is_file());
}

#[test]
fn install_rejects_a_stale_lock() {
    let fixture = Fixture::new("stale");
    assert_success(&fixture.run(&["init"]));
    assert_success(&fixture.run(&["install"]));
    fs::write(
        fixture.application.join("whim.toml"),
        "manifest-version = 1\n\n[requirements]\nwhim = \"^0.8\"\n",
    )
    .expect("the manifest is writable");

    let output = fixture.run(&["install"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stale"));
}

#[test]
fn package_commands_reload_the_manifest_after_waiting_for_the_lock() {
    const FIRST: &str = "https://fixtures.invalid/first.git";
    const SECOND: &str = "https://fixtures.invalid/second.git";

    let fixture = Fixture::new("locked-manifest-snapshot");
    fixture.repository("first", "1.0.0", "LockedFirst");
    fixture.repository("second", "1.0.0", "LockedSecond");
    assert_success(&fixture.run(&["init", "--no-git"]));
    assert_success(&fixture.run(&["add", FIRST, "--version", "^1"]));
    assert_success(&fixture.run(&["add", SECOND, "--version", "^1"]));

    let manifest_path = fixture.application.join("whim.toml");
    let lock_path = fixture.application.join("whim.lock");
    let two_dependencies_manifest =
        fs::read(&manifest_path).expect("the manifest snapshot is readable");
    let two_dependencies_lock = fs::read(&lock_path).expect("the lock snapshot is readable");
    assert_success(&fixture.run(&["remove", SECOND]));

    let manager_lock_path = fixture.application.join("vendor/.whim/install.lock");
    let manager_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&manager_lock_path)
        .expect("the dependency lock is writable");
    fs2::FileExt::lock_exclusive(&manager_lock).expect("the dependency lock is acquirable");

    let mut update = fixture.command(&["update"]);
    update.env("WHIM_LOG", "info");
    let mut update = WaitingCommand::spawn(update);
    update.wait_until_blocked();
    fs::write(&manifest_path, &two_dependencies_manifest).expect("the manifest is writable");
    fs::write(&lock_path, &two_dependencies_lock).expect("the lockfile is writable");
    fs2::FileExt::unlock(&manager_lock).expect("the dependency lock is releasable");
    assert_success(&update.finish());
    assert_success(&fixture.run(&["install"]));
    assert_success(&fixture.run(&["why", SECOND]));

    assert_success(&fixture.run(&["remove", SECOND]));
    fs2::FileExt::lock_exclusive(&manager_lock).expect("the dependency lock is acquirable");

    let mut why = fixture.command(&["why", SECOND]);
    why.env("WHIM_LOG", "info");
    let mut why = WaitingCommand::spawn(why);
    why.wait_until_blocked();
    fs::write(&manifest_path, two_dependencies_manifest).expect("the manifest is writable");
    fs::write(&lock_path, two_dependencies_lock).expect("the lockfile is writable");
    fs2::FileExt::unlock(&manager_lock).expect("the dependency lock is releasable");

    let why = why.finish();
    assert_success(&why);
    assert!(String::from_utf8_lossy(&why.stdout).contains("second 1.0.0"));
}

#[test]
fn informational_commands_do_not_create_or_modify_manager_state() {
    let clean = Fixture::new("read-only-inspection-clean");
    assert_success(&clean.run(&["init", "--no-git"]));
    let clean_mode = fs::metadata(&clean.application)
        .expect("the project directory is inspectable")
        .permissions()
        .mode();
    fs::set_permissions(&clean.application, fs::Permissions::from_mode(0o555))
        .expect("the project directory can become read-only");
    for arguments in [&["suggestions"][..], &["fund"][..]] {
        let output = clean.run(arguments);
        assert_success(&output);
    }

    let why = clean.run(&["why", "git+https://fixtures.invalid/missing"]);
    assert!(!why.status.success());
    fs::set_permissions(&clean.application, fs::Permissions::from_mode(clean_mode))
        .expect("the project directory can become writable");
    assert!(!clean.application.join("vendor").exists());

    let installed = Fixture::new("read-only-inspection-installed");
    installed.repository("library", "1.0.0", "ReadOnlyInspection");
    assert_success(&installed.run(&["init", "--no-git"]));
    assert_success(&installed.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "^1",
    ]));

    let manager = installed.application.join("vendor/.whim");
    let manager_mode = fs::metadata(&manager)
        .expect("the manager directory is inspectable")
        .permissions()
        .mode();
    let project_mode = fs::metadata(&installed.application)
        .expect("the project directory is inspectable")
        .permissions()
        .mode();
    let lock = manager.join("install.lock");
    let lock_contents = fs::read(&lock).expect("the manager lock is readable");
    fs::set_permissions(&manager, fs::Permissions::from_mode(0o555))
        .expect("the manager directory can become read-only");
    fs::set_permissions(&installed.application, fs::Permissions::from_mode(0o555))
        .expect("the project directory can become read-only");

    let outputs = [
        installed.run(&["why", "https://fixtures.invalid/library.git"]),
        installed.run(&["show", "https://fixtures.invalid/library.git"]),
        installed.run(&["suggestions"]),
        installed.run(&["fund"]),
    ];

    for output in outputs {
        assert_success(&output);
    }

    fs::set_permissions(
        &installed.application,
        fs::Permissions::from_mode(project_mode),
    )
    .expect("the project directory can become writable");
    fs::set_permissions(&manager, fs::Permissions::from_mode(manager_mode))
        .expect("the manager directory can become writable");
    assert_eq!(
        fs::read(lock).expect("the manager lock is readable"),
        lock_contents
    );
}

#[test]
fn why_not_opens_the_project_cache_and_fetches_releases() {
    let fixture = Fixture::new("why-not-project-state");
    fixture.repository("candidate", "1.0.0", "WhyNotCandidate");
    assert_success(&fixture.run(&["init", "--no-git"]));
    assert!(!fixture.application.join("vendor").exists());

    let output = fixture.run(&[
        "why-not",
        "https://fixtures.invalid/candidate.git",
        "--version",
        "^1",
    ]);
    assert_success(&output);
    assert!(
        fixture
            .application
            .join("vendor/.whim/install.lock")
            .is_file()
    );
    assert!(fixture.application.join("vendor/.whim/git").is_dir());
}

#[test]
fn package_commands_preserve_unowned_root_stage_directories() {
    let fixture = Fixture::new("unowned-stage");
    assert_success(&fixture.run(&["init", "--no-git"]));
    let unowned = fixture.application.join(".whim-stage-user-data");
    fs::create_dir(&unowned).expect("the user directory is creatable");
    fs::write(unowned.join("important"), "keep me").expect("the user file is writable");

    assert_success(&fixture.run(&["install"]));
    assert_eq!(
        fs::read_to_string(unowned.join("important")).expect("the user file is readable"),
        "keep me"
    );
}

#[test]
fn conflicts_explanations_suggestions_funding_and_license_warnings_work_together() {
    let fixture = Fixture::new("package-inspection");
    let common = fixture.repository("common", "1.0.0", "CommonInspection");
    Fixture::release(&common, "2.0.0");
    let library = fixture.repository_with_dependencies(
        "library",
        "1.0.0",
        "LibraryInspection",
        &[("common", "*")],
    );
    fs::write(
        library.join("whim.toml"),
        "manifest-version = 1\n\n[package]\nrepository = \"https://fixtures.invalid/library\"\nhomepage = \"https://fixtures.invalid/library/home\"\nauthor = \"Library Author\"\ndescription = \"A package inspection fixture.\"\nlicense = \"GPL-3.0-only\"\nsponsor = \"https://github.com/sponsors/library\"\n\n[requirements]\nwhim = \"*\"\n\n[autoload.namespaces]\n\"LibraryInspection\\\\\" = \"src/\"\n\n[dependencies]\n\"git+https://fixtures.invalid/common\" = \"*\"\n\n[dev-dependencies]\n\"git+https://fixtures.invalid/tool\" = \"^5\"\n\n[conflicts]\n\"git+https://fixtures.invalid/common\" = \"^2\"\n\n[suggests]\n\"git+https://fixtures.invalid/extra\" = \"^3\"\n",
    )
    .expect("the package manifest is writable");
    Fixture::release(&library, "1.1.0");

    assert_success(&fixture.run(&["init"]));
    let mut root = fs::read_to_string(fixture.application.join("whim.toml"))
        .expect("the root manifest is readable");
    root.push_str(
        "\n[package]\nlicense = \"MIT\"\n\n[suggests]\n\"git+https://fixtures.invalid/root-extra\" = \"^4\"\n",
    );
    fs::write(fixture.application.join("whim.toml"), root).expect("the root manifest is writable");
    let add = fixture.run(&[
        "add",
        "https://fixtures.invalid/library.git",
        "--version",
        "=1.1.0",
    ]);
    assert_success(&add);
    assert!(
        String::from_utf8_lossy(&add.stderr).contains("may be incompatible"),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let lock = fs::read_to_string(fixture.application.join("whim.lock"))
        .expect("the lockfile is readable");
    assert_eq!(
        locked_version(&lock, "git+https://fixtures.invalid/common"),
        "1.0.0"
    );

    assert_success(&fixture.run(&[
        "add",
        "https://fixtures.invalid/common.git",
        "--version",
        "=1.0.0",
        "--dev",
    ]));

    let why = fixture.run(&["why", "https://fixtures.invalid/common.git"]);
    assert_success(&why);
    let why = String::from_utf8_lossy(&why.stdout);
    assert!(
        why.contains("the root project requires it at runtime"),
        "{why}"
    );
    assert!(why.contains("library 1.1.0"), "{why}");
    assert!(why.contains("common 1.0.0"), "{why}");

    let show = fixture.run(&["show", "https://fixtures.invalid/library.git"]);
    assert_success(&show);
    let show = String::from_utf8_lossy(&show.stdout);
    for expected in [
        "source          : git+https://fixtures.invalid/library",
        "description     : A package inspection fixture.",
        "version         : 1.1.0",
        "tag             : v1.1.0",
        "license         : GPL-3.0-only",
        "repository      : https://fixtures.invalid/library",
        "homepage        : https://fixtures.invalid/library/home",
        "author          : Library Author",
        "sponsor         : https://github.com/sponsors/library",
        "autoload\n  LibraryInspection\\ => src/",
        "requires\n  whim *\n  git+https://fixtures.invalid/common *",
        "requires (development)\n  git+https://fixtures.invalid/tool ^5",
        "conflicts\n  git+https://fixtures.invalid/common ^2",
        "suggests\n  git+https://fixtures.invalid/extra ^3",
    ] {
        assert!(show.contains(expected), "missing `{expected}` in:\n{show}");
    }
    assert!(show.contains("path            : "), "{show}");
    assert!(show.contains("/vendor/packages/"), "{show}");

    let missing = fixture.run(&["show", "https://fixtures.invalid/missing.git"]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("is not present in the lockfile"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let why_not = fixture.run(&[
        "why-not",
        "https://fixtures.invalid/common.git",
        "--version",
        "^2",
    ]);
    assert_success(&why_not);
    let why_not = String::from_utf8_lossy(&why_not.stdout);
    assert!(why_not.contains("cannot be installed"), "{why_not}");
    assert!(why_not.contains("conflicts with"), "{why_not}");

    let suggestions = fixture.run(&["suggestions"]);
    assert_success(&suggestions);
    let suggestions = String::from_utf8_lossy(&suggestions.stdout);
    assert!(suggestions.contains("extra ^3"), "{suggestions}");
    assert!(suggestions.contains("suggested by"), "{suggestions}");
    assert!(suggestions.contains("root-extra ^4"), "{suggestions}");
    assert!(suggestions.contains("the root project"), "{suggestions}");

    let fund = fixture.run(&["fund"]);
    assert_success(&fund);
    let fund = String::from_utf8_lossy(&fund.stdout);
    assert!(
        fund.starts_with("Whim\n  https://github.com/azjezz"),
        "{fund}"
    );
    assert!(
        fund.contains("https://github.com/sponsors/library"),
        "{fund}"
    );
}

fn git<const N: usize>(directory: Option<&Path>, arguments: [&str; N]) {
    let mut command = Command::new("git");
    if let Some(directory) = directory {
        command.arg("-C").arg(directory);
    }
    let output = command.args(arguments).output().expect("Git starts");
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn locked_version<'a>(lock: &'a str, source: &str) -> &'a str {
    let marker = format!("source = \"{source}\"");
    let package = lock
        .split("[[packages]]")
        .find(|package| package.contains(&marker))
        .expect("the package is locked");
    package
        .lines()
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
        .expect("the package version is locked")
}
