use std::io::Error as IoError;
use std::path::PathBuf;

use rayon::ThreadPoolBuildError;
use semver::Version;
use semver::VersionReq;
use thiserror::Error as ThisError;

use crate::config::Error as ConfigurationError;
use crate::error::boxed_from;
use crate::package::archive::Error as ArchiveError;
use crate::package::filesystem::Error as FilesystemError;
use crate::package::git::Error as GitError;
use crate::package::loader::Error as LoaderError;
use crate::package::lock::Error as LockError;
use crate::package::source::Error as SourceError;
use crate::package::state::Error as StateError;
use crate::package::transaction::Error as TransactionError;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Configuration(Box<ConfigurationError>),
    #[error(transparent)]
    Archive(Box<ArchiveError>),
    #[error(transparent)]
    Filesystem(Box<FilesystemError>),
    #[error(transparent)]
    Git(Box<GitError>),
    #[error(transparent)]
    Loader(Box<LoaderError>),
    #[error(transparent)]
    Lock(Box<LockError>),
    #[error(transparent)]
    Source(Box<SourceError>),
    #[error(transparent)]
    State(Box<StateError>),
    #[error(transparent)]
    Transaction(Box<TransactionError>),
    #[error("could not create staging package directory `{}`: {source}", path.display())]
    CreatePackageDirectory {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not start dependency workers: {0}")]
    CreateWorkerPool(#[source] ThreadPoolBuildError),
    #[error("could not remove uninstalled development package `{package}` at `{}`: {source}", path.display())]
    RemoveDevelopmentPackage {
        package: String,
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not inspect installed path `{}`: {source}", path.display())]
    InspectInstalledPath {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("exported manifest for `{0}` does not match the resolved manifest")]
    ExportedManifestMismatch(String),
    #[error("tree mismatch for `{package}`: expected {expected}, got {actual}")]
    TreeMismatch {
        package: String,
        expected: String,
        actual: String,
    },
    #[error("checksum mismatch for `{package}`: expected {expected}, got {actual}")]
    ChecksumMismatch {
        package: String,
        expected: String,
        actual: String,
    },
    #[error("manifest digest for `{0}` does not match the lockfile")]
    ManifestDigestMismatch(String),
    #[error("resolved package `{0}` is missing")]
    MissingResolvedPackage(String),
    #[error("locked package `{0}` is missing")]
    MissingLockedPackage(String),
    #[error("lockfile roots do not match the root manifest")]
    RootMismatch,
    #[error(
        "the root project conflicts with `{package}` {version} through requirement {requirement}"
    )]
    RootConflict {
        package: String,
        version: Box<Version>,
        requirement: Box<VersionReq>,
    },
    #[error("locked override for `{0}` does not match the root manifest")]
    OverrideMismatch(String),
    #[error("locked dependency edges for `{0}` do not match its manifest")]
    DependencyEdgesMismatch(String),
    #[error("locked {dependency} {version} does not satisfy `{owner}` requirement {requirement}")]
    RequirementMismatch {
        owner: String,
        dependency: String,
        version: Box<Version>,
        requirement: Box<VersionReq>,
    },
    #[error(
        "`{owner}` {owner_version} conflicts with `{target}` {target_version} through requirement {requirement}"
    )]
    PackageConflict {
        owner: String,
        owner_version: Box<Version>,
        target: String,
        target_version: Box<Version>,
        requirement: Box<VersionReq>,
    },
    #[error("locked metadata for `{0}` does not match its manifest")]
    MetadataMismatch(String),
}

boxed_from!(
    ConfigurationError => Configuration,
    ArchiveError => Archive,
    FilesystemError => Filesystem,
    GitError => Git,
    LoaderError => Loader,
    LockError => Lock,
    SourceError => Source,
    StateError => State,
    TransactionError => Transaction,
);
