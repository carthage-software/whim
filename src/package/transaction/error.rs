use std::fmt;
use std::io::Error as IoError;
use std::path::PathBuf;

use thiserror::Error as ThisError;

use crate::package::filesystem::Error as FilesystemError;
use crate::package::state::Error as StateError;

#[derive(Clone, Copy, Debug)]
pub(crate) enum IoAction {
    Create,
    Inspect,
    Remove,
}

impl fmt::Display for IoAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Create => "create",
            Self::Inspect => "inspect",
            Self::Remove => "remove",
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MoveAction {
    BackUp,
    Install,
    Restore,
}

impl fmt::Display for MoveAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BackUp => "back up",
            Self::Install => "install",
            Self::Restore => "restore",
        })
    }
}

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Filesystem(#[from] FilesystemError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("an interrupted or untrusted dependency transaction marker exists at `{}`; inspect the package state before removing it", .0.display())]
    InterruptedTransaction(PathBuf),
    #[error("could not {action} `{}`: {source}", path.display())]
    Io {
        action: IoAction,
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not {action} `{}` to `{}`: {source}", from.display(), to.display())]
    Move {
        action: MoveAction,
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("staging directory `{}` has no valid file name", .0.display())]
    InvalidStageName(PathBuf),
    #[error("could not allocate a staging directory in `{}` after {attempts} attempts", directory.display())]
    StageNamesExhausted { directory: PathBuf, attempts: u32 },
    #[error("{failure}; restoring the previous dependency state also failed: {restore}")]
    Rollback {
        #[source]
        failure: Box<Self>,
        restore: Box<Self>,
    },
}

impl Error {
    pub(super) fn io(action: IoAction, path: impl Into<PathBuf>, source: IoError) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }

    pub(super) fn move_path(
        action: MoveAction,
        from: impl Into<PathBuf>,
        to: impl Into<PathBuf>,
        source: IoError,
    ) -> Self {
        Self::Move {
            action,
            from: from.into(),
            to: to.into(),
            source,
        }
    }
}
