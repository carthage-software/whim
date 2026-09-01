use std::io::Error as IoError;
use std::path::PathBuf;
use std::path::StripPrefixError;
use std::process::ExitCode;
use std::process::ExitStatus;

use semver::Error as SemVerError;
use thiserror::Error as ThisError;
use whim_formatter::settings::SettingsError;
use whim_runtime::engine::EngineError;

use crate::config::Error as ConfigurationError;
use crate::filesystem::Error as FilesystemError;
use crate::package::Error as PackageError;
use crate::package::GitError;
use crate::package::LockError;
use crate::package::SourceError;
use crate::server::Error as ServerError;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Package(Box<PackageError>),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Git(Box<GitError>),
    #[error(transparent)]
    Filesystem(Box<FilesystemError>),
    #[error(transparent)]
    Lock(Box<LockError>),
    #[error(transparent)]
    Configuration(Box<ConfigurationError>),
    #[error(transparent)]
    Server(Box<ServerError>),
    #[error("the Whim build version is invalid")]
    InvalidCurrentVersion(#[source] SemVerError),
    #[error("invalid dependency requirement `{requirement}`: {source}")]
    InvalidDependencyRequirement {
        requirement: String,
        #[source]
        source: SemVerError,
    },
    #[error("invalid requested requirement `{requirement}`: {source}")]
    InvalidRequestedRequirement {
        requirement: String,
        #[source]
        source: SemVerError,
    },
    #[error("invalid suggestion requirement `{requirement}` for `{dependency}`: {source}")]
    InvalidSuggestionRequirement {
        dependency: String,
        requirement: String,
        #[source]
        source: SemVerError,
    },
    #[error("could not read current directory: {0}")]
    CurrentDirectory(#[source] IoError),
    #[error("could not read `{path}`: {source}", path = path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not read Whim source from standard input: {0}")]
    ReadStdin(#[source] IoError),
    #[error("could not inspect `{path}`: {source}", path = path.display())]
    InspectPath {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not resolve `{path}`: {source}", path = path.display())]
    ResolvePath {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not read directory `{path}`: {source}", path = path.display())]
    ReadDirectory {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("invalid format settings: {0}")]
    InvalidFormatSettings(#[source] SettingsError),
    #[error("`{}` already exists", .0.display())]
    ManifestExists(PathBuf),
    #[error("project directory `{}` exists but is not a directory", .0.display())]
    InvalidProjectDirectory(PathBuf),
    #[error("project file `{}` exists but is not a file", .0.display())]
    InvalidProjectFile(PathBuf),
    #[error("project path `{}` appeared during initialization", .0.display())]
    InitializationPathAppeared(PathBuf),
    #[error("could not inspect the Git worktree at `{}`: {source}", path.display())]
    InspectGitWorktree {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error(
        "could not initialize `{}` because Git is unavailable; install Git or use `whim init --no-git`",
        path.display()
    )]
    GitUnavailable {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("`{}` contains invalid Git worktree metadata", .0.display())]
    InvalidGitWorktree(PathBuf),
    #[error("could not initialize Git at `{}`: {source}", path.display())]
    InitializeGit {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not initialize Git at `{}`; Git exited with {status}: {message}", path.display())]
    GitInitializationRejected {
        path: PathBuf,
        status: ExitStatus,
        message: String,
    },
    #[error("project initialization failed: {operation}; rollback also failed: {rollback}")]
    InitializationRollback {
        #[source]
        operation: Box<Self>,
        rollback: Box<Self>,
    },
    #[error("could not find `{}`; run `whim install`", .0.display())]
    LockNotFound(PathBuf),
    #[error("`{dependency}` is already a {group} dependency")]
    DependencyGroupConflict {
        dependency: String,
        group: &'static str,
    },
    #[error("`{0}` has no stable SemVer release tag")]
    NoStableRelease(String),
    #[error("`{0}` is not a direct dependency")]
    NotDirectDependency(String),
    #[error("`{0}` appears in both dependency groups")]
    DuplicateDependency(String),
    #[error("targeted update requires an existing lockfile")]
    TargetedUpdateNeedsLock,
    #[error("`{0}` is not present in the lockfile")]
    MissingLockedSource(String),
    #[error("`{0}` is locked but not installed; run `whim install`")]
    PackageNotInstalled(String),
    #[error("security error: tag {tag} for `{repository}` moved from {previous} to {current}")]
    MovedTag {
        tag: String,
        repository: String,
        previous: String,
        current: String,
    },
    #[error("the entry file is missing")]
    MissingEntryFile,
    #[error("{}: is not a file or directory", .0.display())]
    InvalidFormatTarget(PathBuf),
    #[error("format target `{}` is outside discovery root `{}`", path.display(), root.display())]
    FormatTargetEscapesRoot {
        path: PathBuf,
        root: PathBuf,
        #[source]
        source: StripPrefixError,
    },
    #[error("could not write the disassembly: {0}")]
    WriteDisassembly(#[source] IoError),
    #[error("could not write command output: {0}")]
    WriteOutput(#[source] IoError),
    #[error("could not write command error output: {0}")]
    WriteErrorOutput(#[source] IoError),
    #[error("the standard-library artifact failed to load:\n{0}")]
    LoadStandardLibrary(#[source] EngineError),
}

macro_rules! boxed_from {
    ($($source:ty => $variant:ident),+ $(,)?) => {
        $(
            impl From<$source> for Error {
                fn from(error: $source) -> Self {
                    Self::$variant(Box::new(error))
                }
            }
        )+
    };
}

pub(crate) use boxed_from;

boxed_from!(
    PackageError => Package,
    FilesystemError => Filesystem,
    GitError => Git,
    LockError => Lock,
    ConfigurationError => Configuration,
    ServerError => Server,
);

impl Error {
    pub(crate) fn exit_code(&self) -> ExitCode {
        match self {
            Self::LoadStandardLibrary(_) => ExitCode::from(255),
            _ => ExitCode::FAILURE,
        }
    }
}
