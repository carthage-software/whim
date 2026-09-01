use std::fs::File;
use std::io::Error as IoError;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("could not read `{}`: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not create `{}`: {source}", path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not write `{}`: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not sync `{}`: {source}", path.display())]
    Sync {
        path: PathBuf,
        #[source]
        source: IoError,
    },
}

pub(crate) fn hash_file(path: &Path) -> Result<String, Error> {
    let mut file = File::open(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;

    let mut hasher = blake3::Hasher::new();
    hasher
        .update_reader(&mut file)
        .map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

pub(crate) fn write_file(path: &Path, contents: &[u8]) -> Result<(), Error> {
    let mut file = File::create(path).map_err(|source| Error::Create {
        path: path.to_path_buf(),
        source,
    })?;

    file.write_all(contents).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })?;

    file.sync_all().map_err(|source| Error::Sync {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), Error> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| Error::Sync {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}
