mod fetch;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::env::VarError;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Write;
use std::num::ParseIntError;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

#[cfg(target_os = "linux")]
use rustix::io::Errno;
use semver::Version;
use thiserror::Error as ThisError;

use crate::config::MAXIMUM_MANIFEST_BYTES;
use crate::git::clear_repository_environment;
use crate::package::source::Source;

const MAXIMUM_TAGS: usize = 100_000;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Operation {
    AddOrigin,
    Archive,
    CheckCommit,
    FetchCommit,
    FetchTags,
    Initialize,
    InspectBareRepository,
    InspectManifest,
    ListTags,
    ListRemoteTags,
    PruneTags,
    ReadManifest,
    ReadOrigin,
    ResolveRevision,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AddOrigin => "add the Git origin",
            Self::Archive => "archive a Git commit",
            Self::CheckCommit => "check a Git commit",
            Self::FetchCommit => "fetch a Git commit",
            Self::FetchTags => "fetch Git tags",
            Self::Initialize => "initialize the Git cache",
            Self::InspectBareRepository => "inspect the bare Git cache",
            Self::InspectManifest => "inspect the package manifest",
            Self::ListTags => "list Git tags",
            Self::ListRemoteTags => "list remote Git tags",
            Self::PruneTags => "prune cached Git tags",
            Self::ReadManifest => "read the package manifest",
            Self::ReadOrigin => "read the Git origin",
            Self::ResolveRevision => "resolve a Git revision",
        })
    }
}

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("could not create Git cache `{}`: {source}", path.display())]
    CreateCache {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not inspect Git cache `{}`: {source}", path.display())]
    InspectCache {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("the Git cache path `{}` is not a directory", .0.display())]
    InvalidCachePath(PathBuf),
    #[error("Git cache entry `{}` is not a regular file or directory", .0.display())]
    InvalidCacheEntry(PathBuf),
    #[error("the Git cache size overflowed")]
    CacheSizeOverflow,
    #[error("the Git cache reached or exceeded the {limit} byte limit")]
    CacheTooLarge { limit: u64 },
    #[error("could not remove rejected Git cache `{}`: {source}", path.display())]
    RemoveRejectedCache {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("the Git cache for `{0}` belongs to another repository")]
    WrongRepository(String),
    #[error("the Git cache for `{0}` is not a bare repository")]
    NotBare(String),
    #[error("could not {operation}: {source}")]
    Execute {
        operation: Operation,
        #[source]
        source: IoError,
    },
    #[error("could not provide input while attempting to {operation}: {source}")]
    WriteInput {
        operation: Operation,
        #[source]
        source: IoError,
    },
    #[error("the Git process did not expose its configured input while attempting to {0}")]
    MissingInput(Operation),
    #[error("the Git process did not expose its configured diagnostics while attempting to {0}")]
    MissingDiagnostics(Operation),
    #[error("the Git process did not expose its configured output while attempting to {0}")]
    MissingOutput(Operation),
    #[cfg(target_os = "linux")]
    #[error("could not limit Git cache writes: {0}")]
    LimitChildFileSize(#[source] Errno),
    #[error("could not read Git diagnostics while attempting to {operation}: {source}")]
    ReadDiagnostics {
        operation: Operation,
        #[source]
        source: IoError,
    },
    #[error("could not read Git output while attempting to {operation}: {source}")]
    ReadOutput {
        operation: Operation,
        #[source]
        source: IoError,
    },
    #[error("the Git diagnostic reader failed while attempting to {0}")]
    DiagnosticReaderPanicked(Operation),
    #[error("the Git output reader failed while attempting to {0}")]
    OutputReaderPanicked(Operation),
    #[error("the Git output from an attempt to {operation} is not valid UTF-8: {source}")]
    NonUtf8Output {
        operation: Operation,
        #[source]
        source: FromUtf8Error,
    },
    #[error("could not {operation}; the Git process exited with {status}")]
    Exit {
        operation: Operation,
        status: ExitStatus,
    },
    #[error("could not {operation}; the Git process exited with {status}: {message}")]
    Rejected {
        operation: Operation,
        status: ExitStatus,
        message: String,
    },
    #[error("the Git process did not provide locked commit `{0}`")]
    MissingCommit(String),
    #[error("the Git source exposes more than {limit} tags")]
    TooManyTags { limit: usize },
    #[error("the Git tag metadata exceeds the {limit} byte limit")]
    TagMetadataTooLarge { limit: u64 },
    #[error("the Git tag metadata size overflowed")]
    TagMetadataSizeOverflow,
    #[error("a Git tag metadata line exceeds the {limit} byte limit")]
    TagMetadataLineTooLong { limit: usize },
    #[error("the Git tag metadata is not valid UTF-8: {0}")]
    NonUtf8TagMetadata(#[source] Utf8Error),
    #[error("the Git process returned malformed tag metadata")]
    MalformedTagMetadata,
    #[error("environment variable `WHIM_PACKAGE_NETWORK_TIMEOUT` is not valid Unicode")]
    InvalidNetworkTimeoutEnvironment(#[source] VarError),
    #[error(
        "`WHIM_PACKAGE_NETWORK_TIMEOUT` must be a positive integer number of seconds, not `{value}`"
    )]
    InvalidNetworkTimeout {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("could not {operation} within {seconds} seconds")]
    NetworkTimeout { operation: Operation, seconds: u64 },
    #[error(
        "tags `{first}` and `{second}` both describe version {version} but point to different commits"
    )]
    AmbiguousVersion {
        first: String,
        second: String,
        version: Version,
    },
    #[error("could not inspect `whim.toml` at commit {commit}: {source}")]
    InspectManifest {
        commit: String,
        #[source]
        source: Box<Self>,
    },
    #[error("the Git process returned an invalid manifest size at commit {commit}: {source}")]
    InvalidManifestSize {
        commit: String,
        #[source]
        source: ParseIntError,
    },
    #[error("manifest at commit {commit} exceeds the {limit} byte limit")]
    ManifestTooLarge { commit: String, limit: u64 },
    #[error("could not read `whim.toml` at commit {commit}: {source}")]
    ReadManifest {
        commit: String,
        #[source]
        source: Box<Self>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct GitCandidate {
    pub(crate) version: Version,
    pub(crate) tag: String,
    pub(crate) commit: String,
    pub(crate) tree: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Repository {
    directory: PathBuf,
    fetch: String,
}

impl Repository {
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(source = %source, refresh),
    )]
    pub(crate) fn open(cache: &Path, source: &Source, refresh: bool) -> Result<Self, Error> {
        fs::create_dir_all(cache).map_err(|error| Error::CreateCache {
            path: cache.to_path_buf(),
            source: error,
        })?;

        let directory = cache.join(format!("{}.git", source.digest()));
        let exists = match fs::symlink_metadata(&directory) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir() {
                    return Err(Error::InvalidCachePath(directory));
                }
                true
            }
            Err(error) if error.kind() == ErrorKind::NotFound => false,
            Err(source) => {
                return Err(Error::InspectCache {
                    path: directory,
                    source,
                });
            }
        };

        if exists {
            let origin = run_git(
                Operation::ReadOrigin,
                Some(&directory),
                ["config", "--get", "remote.origin.url"],
            )?;

            if origin.trim() != source.fetch() {
                return Err(Error::WrongRepository(source.to_string()));
            }

            let bare = run_git(
                Operation::InspectBareRepository,
                Some(&directory),
                ["rev-parse", "--is-bare-repository"],
            )?;

            if bare.trim() != "true" {
                return Err(Error::NotBare(source.to_string()));
            }
        } else {
            let initialized = run_git_without_output(
                Operation::Initialize,
                None,
                [
                    OsStr::new("init"),
                    OsStr::new("--bare"),
                    OsStr::new("--template="),
                    directory.as_os_str(),
                ],
            );

            if let Err(error) = initialized {
                remove_partial_cache(&directory);
                return Err(error);
            }

            let origin = run_git_without_output(
                Operation::AddOrigin,
                Some(&directory),
                ["remote", "add", "origin", source.fetch()],
            );

            if let Err(error) = origin {
                remove_partial_cache(&directory);
                return Err(error);
            }
        }

        let repository = Self {
            directory,
            fetch: source.fetch().to_owned(),
        };

        if refresh {
            repository.fetch_tags()?;
        }

        Ok(repository)
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) fn fetch_tags(&self) -> Result<(), Error> {
        let tags = self.remote_version_tags()?;
        if !tags.is_empty() {
            let mut refspecs = String::new();
            for tag in &tags {
                refspecs.push_str("+refs/tags/");
                refspecs.push_str(tag);
                refspecs.push_str(":refs/tags/");
                refspecs.push_str(tag);
                refspecs.push('\n');
            }

            fetch::run(
                Operation::FetchTags,
                &self.directory,
                [
                    "fetch",
                    "--force",
                    "--depth=1",
                    "--no-tags",
                    "--stdin",
                    &self.fetch,
                ],
                Some(refspecs.as_bytes()),
            )?;
        }

        self.prune_tags(&tags)?;

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip_all, fields(%commit))]
    pub(crate) fn fetch_commit(&self, commit: &str) -> Result<(), Error> {
        if self.has_commit(commit)? {
            return Ok(());
        }

        fetch::run(
            Operation::FetchCommit,
            &self.directory,
            ["fetch", "--depth=1", "--no-tags", &self.fetch, commit],
            None,
        )?;

        if !self.has_commit(commit)? {
            return Err(Error::MissingCommit(commit.to_owned()));
        }

        Ok(())
    }

    fn remote_version_tags(&self) -> Result<BTreeSet<String>, Error> {
        fetch::remote_version_tags(&self.fetch, MAXIMUM_TAGS)
    }

    fn prune_tags(&self, retained: &BTreeSet<String>) -> Result<(), Error> {
        let local = run_git(
            Operation::ListTags,
            Some(&self.directory),
            ["for-each-ref", "--format=%(refname:strip=2)", "refs/tags"],
        )?;
        let mut updates = String::new();
        for tag in local.lines() {
            if !retained.contains(tag) {
                updates.push_str("delete refs/tags/");
                updates.push_str(tag);
                updates.push('\n');
            }
        }

        if !updates.is_empty() {
            run_git_with_input(
                Operation::PruneTags,
                Some(&self.directory),
                ["update-ref", "--stdin"],
                updates.as_bytes(),
            )?;
        }

        Ok(())
    }

    pub(crate) fn has_commit(&self, commit: &str) -> Result<bool, Error> {
        let output = execute_git(
            Operation::CheckCommit,
            Some(&self.directory),
            [
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{commit}^{{commit}}"),
            ],
        )?;

        if output.status.success() {
            return Ok(true);
        }

        if output.status.code() == Some(1) {
            return Ok(false);
        }

        Err(failed(Operation::CheckCommit, &output))
    }

    pub(crate) fn candidates(&self) -> Result<Vec<GitCandidate>, Error> {
        let output = run_git(
            Operation::ListTags,
            Some(&self.directory),
            [
                "for-each-ref",
                "--count=100001",
                "--sort=refname",
                "--format=%(refname:strip=2)\t%(objecttype)\t%(objectname)\t%(*objecttype)\t%(*objectname)\t%(tree)\t%(*tree)",
                "refs/tags",
            ],
        )?;

        let mut candidates: BTreeMap<Version, GitCandidate> = BTreeMap::new();
        for (index, line) in output.lines().enumerate() {
            if index >= MAXIMUM_TAGS {
                return Err(Error::TooManyTags {
                    limit: MAXIMUM_TAGS,
                });
            }

            let mut fields = line.split('\t');
            let (
                Some(tag),
                Some(object_type),
                Some(object),
                Some(target_type),
                Some(target),
                Some(object_tree),
                Some(target_tree),
            ) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            )
            else {
                return Err(Error::MalformedTagMetadata);
            };

            if fields.next().is_some() {
                return Err(Error::MalformedTagMetadata);
            }

            let version_text = tag.strip_prefix('v').unwrap_or(tag);
            let Ok(version) = Version::parse(version_text) else {
                continue;
            };

            let (commit, tree) = match (object_type, target_type) {
                ("commit", _) if !object_tree.is_empty() => (object, object_tree),
                ("tag", "commit") if !target_tree.is_empty() => (target, target_tree),
                _ => continue,
            };

            let candidate = GitCandidate {
                version: version.clone(),
                tag: tag.to_owned(),
                commit: commit.to_owned(),
                tree: tree.to_owned(),
            };

            match candidates.get(&version) {
                Some(existing) if existing.commit != candidate.commit => {
                    return Err(Error::AmbiguousVersion {
                        first: existing.tag.clone(),
                        second: candidate.tag,
                        version,
                    });
                }
                Some(_) => {}
                None => {
                    candidates.insert(version, candidate);
                }
            }
        }

        Ok(candidates.into_values().collect())
    }

    #[tracing::instrument(level = "trace", skip_all, fields(%commit))]
    pub(crate) fn manifest(&self, commit: &str) -> Result<String, Error> {
        let object = format!("{commit}:whim.toml");
        let size = run_git(
            Operation::InspectManifest,
            Some(&self.directory),
            ["cat-file", "-s", &object],
        )
        .map_err(|source| Error::InspectManifest {
            commit: commit.to_owned(),
            source: Box::new(source),
        })?
        .trim()
        .parse::<u64>()
        .map_err(|source| Error::InvalidManifestSize {
            commit: commit.to_owned(),
            source,
        })?;

        if size > MAXIMUM_MANIFEST_BYTES {
            return Err(Error::ManifestTooLarge {
                commit: commit.to_owned(),
                limit: MAXIMUM_MANIFEST_BYTES,
            });
        }

        run_git(
            Operation::ReadManifest,
            Some(&self.directory),
            ["show", &object],
        )
        .map_err(|source| Error::ReadManifest {
            commit: commit.to_owned(),
            source: Box::new(source),
        })
    }

    pub(crate) fn rev_parse(&self, revision: &str) -> Result<String, Error> {
        Ok(run_git(
            Operation::ResolveRevision,
            Some(&self.directory),
            ["rev-parse", revision],
        )?
        .trim()
        .to_owned())
    }

    #[tracing::instrument(level = "trace", skip_all, fields(%commit))]
    pub(crate) fn archive(&self, commit: &str) -> Result<Child, Error> {
        let mut command = git_command(Some(&self.directory));
        command
            .args(["archive", "--format=tar", commit])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.spawn().map_err(|source| Error::Execute {
            operation: Operation::Archive,
            source,
        })
    }
}

fn run_git_with_input<I, A>(
    operation: Operation,
    directory: Option<&Path>,
    arguments: I,
    input: &[u8],
) -> Result<(), Error>
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let mut command = git_command(directory);
    tracing::trace!(%operation, ?directory, "running Git");
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|source| Error::Execute { operation, source })?;
    let Some(mut stdin) = child.stdin.take() else {
        fetch::stop(&mut child);
        return Err(Error::MissingInput(operation));
    };

    if let Err(source) = stdin.write_all(input) {
        fetch::stop(&mut child);
        return Err(Error::WriteInput { operation, source });
    }

    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|source| Error::Execute { operation, source })?;
    if !output.status.success() {
        return Err(failed(operation, &output));
    }

    Ok(())
}

fn remove_partial_cache(directory: &Path) {
    if let Err(error) = fs::remove_dir_all(directory)
        && error.kind() != ErrorKind::NotFound
    {
        tracing::warn!(path = %directory.display(), %error, "could not remove partial Git cache");
    }
}

fn run_git<I, A>(
    operation: Operation,
    directory: Option<&Path>,
    arguments: I,
) -> Result<String, Error>
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let mut command = git_command(directory);
    tracing::trace!(%operation, ?directory, "running Git");
    command.args(arguments);
    let output = command
        .output()
        .map_err(|source| Error::Execute { operation, source })?;

    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|source| Error::NonUtf8Output { operation, source });
    }

    Err(failed(operation, &output))
}

fn run_git_without_output<I, A>(
    operation: Operation,
    directory: Option<&Path>,
    arguments: I,
) -> Result<(), Error>
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let output = execute_git(operation, directory, arguments)?;
    if output.status.success() {
        return Ok(());
    }

    Err(failed(operation, &output))
}

fn execute_git<I, A>(
    operation: Operation,
    directory: Option<&Path>,
    arguments: I,
) -> Result<Output, Error>
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let mut command = git_command(directory);
    tracing::trace!(%operation, ?directory, "running Git");
    command
        .args(arguments)
        .output()
        .map_err(|source| Error::Execute { operation, source })
}

fn failed(operation: Operation, output: &Output) -> Error {
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if message.is_empty() {
        Error::Exit {
            operation,
            status: output.status,
        }
    } else {
        Error::Rejected {
            operation,
            status: output.status,
            message,
        }
    }
}

fn git_command(directory: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    command.args([
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "protocol.allow=never",
        "-c",
        "protocol.https.allow=always",
        "-c",
        "protocol.ssh.allow=always",
        "-c",
        "protocol.file.allow=always",
    ]);

    clear_repository_environment(&mut command);
    if let Some(directory) = directory {
        command.arg("-C").arg(directory);
    }

    command
}
