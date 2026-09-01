use std::env;
use std::fs;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use clap::Args;
use semver::Version;

use crate::config::MANIFEST_NAME;
use crate::error::Error;
use crate::filesystem;
use crate::git::clear_repository_environment;

const ATTRIBUTES: [&str; 4] = [
    "/tests export-ignore",
    "/.gitattributes export-ignore",
    "/.gitignore export-ignore",
    "/whim.lock export-ignore",
];
const SOURCE: &str = "write_line!('Hello, Whim!');\n";
const TEST: &str = "assert!(true);\n";

#[derive(Args)]
pub(super) struct Arguments {
    /// Do not initialize Git or create Git configuration files.
    #[arg(long)]
    no_git: bool,
}

struct FileChange {
    path: PathBuf,
    previous: Option<String>,
    contents: String,
}

#[derive(Default)]
struct Changes {
    created_files: Vec<PathBuf>,
    replaced_files: Vec<(PathBuf, String)>,
    created_directories: Vec<PathBuf>,
    git_directory: Option<PathBuf>,
}

#[tracing::instrument(level = "debug", skip_all, fields(no_git = arguments.no_git))]
pub(super) fn execute(arguments: &Arguments) -> Result<(), Error> {
    let directory = env::current_dir().map_err(Error::CurrentDirectory)?;
    let manifest = directory.join(MANIFEST_NAME);
    require_missing(&manifest)?;

    let version =
        Version::parse(env!("CARGO_PKG_VERSION")).map_err(Error::InvalidCurrentVersion)?;
    let manifest_contents = format!(
        "manifest-version = 1\n\n[requirements]\nwhim = \"^{}.{}\"\n",
        version.major, version.minor
    );
    let source_directory = directory.join("src");
    let test_directory = directory.join("tests");
    require_directory_or_missing(&source_directory)?;
    require_directory_or_missing(&test_directory)?;

    let mut files = vec![FileChange {
        path: manifest.clone(),
        previous: None,
        contents: manifest_contents,
    }];
    add_if_missing(&mut files, source_directory.join("main.whim"), SOURCE)?;
    add_if_missing(&mut files, test_directory.join("main.whim"), TEST)?;

    if !arguments.no_git {
        add_merged(&mut files, directory.join(".gitignore"), &["/vendor/"])?;
        add_merged(&mut files, directory.join(".gitattributes"), &ATTRIBUTES)?;
    }

    let mut changes = Changes::default();
    if !arguments.no_git
        && let Err(error) = initialize_git(&directory, &mut changes)
    {
        return Err(changes.fail(error));
    }

    for path in [&source_directory, &test_directory] {
        if let Err(error) = changes.create_directory(path) {
            return Err(changes.fail(error));
        }
    }

    for change in files {
        if let Err(error) = changes.apply(change) {
            return Err(changes.fail(error));
        }
    }

    tracing::info!(path = %manifest.display(), "created project");
    Ok(())
}

impl Changes {
    fn create_directory(&mut self, path: &Path) -> Result<(), Error> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_dir() => return Ok(()),
            Ok(_) => return Err(Error::InvalidProjectDirectory(path.to_path_buf())),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::InspectPath {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }

        filesystem::create_directory(path)?;
        self.created_directories.push(path.to_path_buf());
        Ok(())
    }

    fn apply(&mut self, change: FileChange) -> Result<(), Error> {
        if let Some(previous) = change.previous {
            filesystem::replace(&change.path, &change.contents)?;
            self.replaced_files.push((change.path, previous));
        } else if filesystem::create(&change.path, &change.contents)? {
            self.created_files.push(change.path);
        } else {
            return Err(Error::InitializationPathAppeared(change.path));
        }

        Ok(())
    }

    fn fail(mut self, operation: Error) -> Error {
        match self.rollback() {
            Ok(()) => operation,
            Err(rollback) => Error::InitializationRollback {
                operation: Box::new(operation),
                rollback: Box::new(rollback),
            },
        }
    }

    fn rollback(&mut self) -> Result<(), Error> {
        let mut failure = None;

        for (path, contents) in self.replaced_files.drain(..).rev() {
            record_failure(&mut failure, filesystem::replace(&path, &contents));
        }
        for path in self.created_files.drain(..).rev() {
            record_failure(&mut failure, filesystem::remove(&path));
        }
        for path in self.created_directories.drain(..).rev() {
            record_failure(&mut failure, filesystem::remove_directory(&path));
        }
        if let Some(path) = self.git_directory.take() {
            record_failure(&mut failure, filesystem::remove_directory_all(&path));
        }

        failure.map_or(Ok(()), Err)
    }
}

fn initialize_git(directory: &Path, changes: &mut Changes) -> Result<(), Error> {
    let inspection = git(directory, ["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|source| git_inspection_error(directory, source))?;
    if inspection.status.success() && inspection.stdout.trim_ascii() == b"true" {
        tracing::debug!(path = %directory.display(), "using existing Git worktree");
        return Ok(());
    }

    let git_directory = directory.join(".git");
    match fs::symlink_metadata(&git_directory) {
        Ok(_) => return Err(Error::InvalidGitWorktree(git_directory)),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::InspectPath {
                path: git_directory,
                source,
            });
        }
    }
    changes.git_directory = Some(git_directory);

    let output = git(directory, ["init", "--quiet"])
        .output()
        .map_err(|source| Error::InitializeGit {
            path: directory.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        tracing::debug!(path = %directory.display(), "initialized Git worktree");
        return Ok(());
    }

    Err(Error::GitInitializationRejected {
        path: directory.to_path_buf(),
        status: output.status,
        message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

fn git_inspection_error(directory: &Path, source: IoError) -> Error {
    if source.kind() == ErrorKind::NotFound {
        Error::GitUnavailable {
            path: directory.to_path_buf(),
            source,
        }
    } else {
        Error::InspectGitWorktree {
            path: directory.to_path_buf(),
            source,
        }
    }
}

fn git<const N: usize>(directory: &Path, arguments: [&str; N]) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(directory).args(arguments);
    clear_repository_environment(&mut command);
    command
}

fn require_missing(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(Error::ManifestExists(path.to_path_buf())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::InspectPath {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn require_directory_or_missing(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(Error::InvalidProjectDirectory(path.to_path_buf())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::InspectPath {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn add_if_missing(files: &mut Vec<FileChange>, path: PathBuf, contents: &str) -> Result<(), Error> {
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(Error::InvalidProjectFile(path)),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            files.push(FileChange {
                path,
                previous: None,
                contents: contents.to_owned(),
            });
            Ok(())
        }
        Err(source) => Err(Error::InspectPath { path, source }),
    }
}

fn add_merged(files: &mut Vec<FileChange>, path: PathBuf, lines: &[&str]) -> Result<(), Error> {
    let previous = read_optional(&path)?;
    let Some(contents) = append_missing(previous.as_deref().unwrap_or(""), lines) else {
        return Ok(());
    };

    files.push(FileChange {
        path,
        previous,
        contents,
    });
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<String>, Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(Error::InvalidProjectFile(path.to_path_buf()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::InspectPath {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    fs::read_to_string(path)
        .map(Some)
        .map_err(|source| Error::ReadFile {
            path: path.to_path_buf(),
            source,
        })
}

fn append_missing(contents: &str, lines: &[&str]) -> Option<String> {
    let missing: Vec<_> = lines
        .iter()
        .copied()
        .filter(|expected| !contents.lines().any(|line| line.trim() == *expected))
        .collect();
    if missing.is_empty() {
        return None;
    }

    let mut updated = contents.to_owned();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    for line in missing {
        updated.push_str(line);
        updated.push('\n');
    }
    Some(updated)
}

fn record_failure(result: &mut Option<Error>, operation: Result<(), filesystem::Error>) {
    if let Err(error) = operation
        && result.is_none()
    {
        *result = Some(error.into());
    }
}
