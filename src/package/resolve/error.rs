use rayon::ThreadPoolBuildError;
use semver::Error as SemVerError;
use semver::Version;
use thiserror::Error as ThisError;

use crate::config::Error as ConfigurationError;
use crate::package::git::Error as GitError;
use crate::package::source::Source;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Configuration(Box<ConfigurationError>),
    #[error(transparent)]
    Git(Box<GitError>),
    #[error("could not load releases from `{repository}`: {source}")]
    LoadReleases {
        repository: Source,
        #[source]
        source: Box<GitError>,
    },
    #[error("the Whim build version is invalid")]
    InvalidCurrentVersion(#[source] SemVerError),
    #[error("conflict search exceeded {limit} attempts")]
    ConflictSearchLimit { limit: usize },
    #[error("could not retrieve dependencies for `{package}` {version}: {source}")]
    RetrieveDependencies {
        package: String,
        version: Version,
        #[source]
        source: Box<Self>,
    },
    #[error("could not choose a version for `{package}`: {source}")]
    ChooseVersion {
        package: String,
        #[source]
        source: Box<Self>,
    },
    #[error("dependency resolution cancellation check failed: {0}")]
    Cancellation(#[source] Box<Self>),
    #[error("{0}")]
    NoSolution(String),
    #[error("resolved package `{package}` {version} has no manifest")]
    MissingManifest { package: String, version: Version },
    #[error("resolved package `{0}` is missing")]
    MissingResolvedPackage(String),
    #[error("dependency cycle includes `{0}`")]
    Cycle(String),
    #[error("dependency graph exceeds {limit} sources")]
    GraphTooLarge { limit: usize },
    #[error("could not start dependency workers: {0}")]
    CreateWorkerPool(#[source] ThreadPoolBuildError),
    #[error("candidate catalog for `{0}` is missing")]
    MissingCandidateCatalog(String),
    #[error("version {version} is unavailable for `{package}`")]
    VersionUnavailable { package: String, version: Version },
    #[error("the Git repository for `{0}` is missing")]
    MissingRepository(String),
    #[error("invalid manifest for `{package}` {version}: {error}")]
    InvalidManifest {
        package: String,
        version: Version,
        #[source]
        error: Box<ConfigurationError>,
    },
    #[error("`{package}` {version} depends on itself")]
    SelfDependency { package: String, version: Version },
    #[error("version {version} disappeared for `{package}`")]
    VersionDisappeared { package: String, version: Version },
    #[error("resolved candidate has no parsed manifest")]
    MissingParsedManifest,
    #[error("resolved candidate has no dependency set")]
    MissingDependencySet,
}

impl From<ConfigurationError> for Error {
    fn from(error: ConfigurationError) -> Self {
        Self::Configuration(Box::new(error))
    }
}

impl From<GitError> for Error {
    fn from(error: GitError) -> Self {
        Self::Git(Box::new(error))
    }
}

impl Error {
    pub(crate) fn explanation(&self) -> Option<&str> {
        match self {
            Self::NoSolution(explanation) => Some(explanation),
            _ => None,
        }
    }
}
