use std::io::Error as IoError;
use std::path::PathBuf;

use thiserror::Error as ThisError;

use crate::config::Error as ConfigurationError;
use crate::package::transaction::Error as TransactionError;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Configuration(Box<ConfigurationError>),
    #[error(transparent)]
    Transaction(Box<TransactionError>),
    #[error("manifest `{}` has no parent directory", .0.display())]
    MissingManifestParent(PathBuf),
    #[error("could not inspect manifest `{}`: {source}", path.display())]
    InspectManifest {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("cannot edit symbolic-link manifest `{}`", .0.display())]
    SymbolicLinkManifest(PathBuf),
    #[error("could not create dependency manager directory `{}`: {source}", path.display())]
    CreateManager {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not inspect dependency manager path `{}`: {source}", path.display())]
    InspectManager {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("dependency manager path `{}` must be a real directory", .0.display())]
    InvalidManagerPath(PathBuf),
    #[error("could not open dependency lock `{}`: {source}", path.display())]
    OpenLock {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not acquire dependency lock `{}`: {source}", path.display())]
    AcquireLock {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not inspect dependency lock `{}`: {source}", path.display())]
    InspectLock {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("dependency lock path `{}` is not a regular file", .0.display())]
    InvalidLockPath(PathBuf),
    #[error("could not inspect installed package `{}`: {source}", path.display())]
    InspectInstalledPackage {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("installed package path `{}` is not a directory", .0.display())]
    InvalidInstalledPackagePath(PathBuf),
    #[error("`whim.lock` is stale; run `whim update` first")]
    StaleLock,
}

impl From<ConfigurationError> for Error {
    fn from(error: ConfigurationError) -> Self {
        Self::Configuration(Box::new(error))
    }
}

impl From<TransactionError> for Error {
    fn from(error: TransactionError) -> Self {
        Self::Transaction(Box::new(error))
    }
}
