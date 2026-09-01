use thiserror::Error as ThisError;

use crate::config::Error as ConfigurationError;
use crate::error::boxed_from;
use crate::package::archive::Error as ArchiveError;
use crate::package::filesystem::Error as FilesystemError;
use crate::package::git::Error as GitError;
use crate::package::install::Error as InstallationError;
use crate::package::loader::Error as LoaderError;
use crate::package::lock::Error as LockError;
use crate::package::project::ProjectError;
use crate::package::resolve::Error as ResolutionError;
use crate::package::source::Error as SourceError;
use crate::package::state::Error as StateError;
use crate::package::transaction::Error as TransactionError;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Configuration(Box<ConfigurationError>),
    #[error(transparent)]
    Source(Box<SourceError>),
    #[error(transparent)]
    Git(Box<GitError>),
    #[error(transparent)]
    Archive(Box<ArchiveError>),
    #[error(transparent)]
    Filesystem(Box<FilesystemError>),
    #[error(transparent)]
    Lock(Box<LockError>),
    #[error(transparent)]
    Loader(Box<LoaderError>),
    #[error(transparent)]
    State(Box<StateError>),
    #[error(transparent)]
    Transaction(Box<TransactionError>),
    #[error(transparent)]
    Project(Box<ProjectError>),
    #[error(transparent)]
    Installation(Box<InstallationError>),
    #[error("dependency resolution failed: {0}")]
    Resolution(#[source] Box<ResolutionError>),
}

boxed_from!(
    ConfigurationError => Configuration,
    SourceError => Source,
    GitError => Git,
    ArchiveError => Archive,
    FilesystemError => Filesystem,
    LockError => Lock,
    LoaderError => Loader,
    StateError => State,
    TransactionError => Transaction,
    ProjectError => Project,
    InstallationError => Installation,
);

impl From<ResolutionError> for Error {
    fn from(error: ResolutionError) -> Self {
        Self::Resolution(Box::new(error))
    }
}

impl Error {
    pub(crate) fn resolution_reason(&self) -> Option<&str> {
        match self {
            Self::Resolution(error) => error.explanation(),
            _ => None,
        }
    }
}
