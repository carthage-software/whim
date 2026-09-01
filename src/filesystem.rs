use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use thiserror::Error as ThisError;

const MAXIMUM_TEMPORARY_ATTEMPTS: u32 = 16;

pub(crate) enum LimitedString {
    Contents(String),
    TooLarge,
}

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("could not inspect `{}`: {source}", path.display())]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not create `{}`: {source}", path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not create directory `{}`: {source}", path.display())]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not write `{}`: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not preserve permissions for `{}`: {source}", path.display())]
    PreservePermissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not replace `{}`: {source}", path.display())]
    Replace {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not remove `{}`: {source}", path.display())]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not remove directory `{}`: {source}", path.display())]
    RemoveDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not sync `{}`: {source}", path.display())]
    Sync {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not create a temporary file in `{}`: {source}", directory.display())]
    CreateTemporary {
        directory: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub(crate) fn read_limited_string(path: &Path, limit: u64) -> io::Result<LimitedString> {
    let file = fs::File::open(path)?;
    read_limited_string_from(file, limit)
}

fn read_limited_string_from(reader: impl Read, limit: u64) -> io::Result<LimitedString> {
    let maximum = limit
        .checked_add(1)
        .ok_or_else(|| io::Error::other("file size limit overflowed"))?;
    let capacity = usize::try_from(limit.min(8 * 1024))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let mut contents = Vec::with_capacity(capacity);
    reader.take(maximum).read_to_end(&mut contents)?;
    if contents.len() as u64 > limit {
        return Ok(LimitedString::TooLarge);
    }
    String::from_utf8(contents)
        .map(LimitedString::Contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn replace(path: &Path, contents: &str) -> Result<(), Error> {
    let directory = parent(path);
    let permissions = fs::metadata(path)
        .map_err(|source| Error::Inspect {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();

    let mut temporary = TemporaryFile::create(directory)?;
    temporary.write(contents, path)?;
    fs::set_permissions(&temporary.path, permissions).map_err(|source| {
        Error::PreservePermissions {
            path: path.to_path_buf(),
            source,
        }
    })?;

    temporary.sync(path)?;
    fs::rename(&temporary.path, path).map_err(|source| Error::Replace {
        path: path.to_path_buf(),
        source,
    })?;

    temporary.persisted = true;
    sync_directory(directory)
}

pub(crate) fn create(path: &Path, contents: &str) -> Result<bool, Error> {
    let directory = parent(path);
    let mut temporary = TemporaryFile::create(directory)?;
    temporary.write(contents, path)?;
    temporary.sync(path)?;
    match fs::hard_link(&temporary.path, path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(source) => {
            return Err(Error::Create {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    temporary.remove();
    sync_directory(directory)?;
    Ok(true)
}

pub(crate) fn remove(path: &Path) -> Result<(), Error> {
    fs::remove_file(path).map_err(|source| Error::Remove {
        path: path.to_path_buf(),
        source,
    })?;

    sync_directory(parent(path))
}

pub(crate) fn create_directory(path: &Path) -> Result<(), Error> {
    fs::create_dir(path).map_err(|source| Error::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })?;

    sync_directory(parent(path))
}

pub(crate) fn remove_directory(path: &Path) -> Result<(), Error> {
    fs::remove_dir(path).map_err(|source| Error::RemoveDirectory {
        path: path.to_path_buf(),
        source,
    })?;

    sync_directory(parent(path))
}

pub(crate) fn remove_directory_all(path: &Path) -> Result<(), Error> {
    match fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(Error::RemoveDirectory {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    sync_directory(parent(path))
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn sync_directory(directory: &Path) -> Result<(), Error> {
    fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| Error::Sync {
            path: directory.to_path_buf(),
            source,
        })
}

struct TemporaryFile {
    path: PathBuf,
    file: fs::File,
    persisted: bool,
}

impl TemporaryFile {
    fn create(directory: &Path) -> Result<Self, Error> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        for _ in 0..MAXIMUM_TEMPORARY_ATTEMPTS {
            let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(".whim-{}-{ordinal}.tmp", process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file,
                        persisted: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(Error::CreateTemporary {
                        directory: directory.to_path_buf(),
                        source,
                    });
                }
            }
        }

        Err(Error::CreateTemporary {
            directory: directory.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "no free temporary file name beside the target",
            ),
        })
    }

    fn write(&mut self, contents: &str, target: &Path) -> Result<(), Error> {
        self.file
            .write_all(contents.as_bytes())
            .map_err(|source| Error::Write {
                path: target.to_path_buf(),
                source,
            })
    }

    fn sync(&self, target: &Path) -> Result<(), Error> {
        self.file.sync_all().map_err(|source| Error::Sync {
            path: target.to_path_buf(),
            source,
        })
    }

    fn remove(&mut self) {
        self.persisted = true;
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path = %self.path.display(), %error, "could not remove temporary file");
            }
        }
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if !self.persisted {
            self.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;
    use std::process;

    use crate::filesystem::LimitedString;
    use crate::filesystem::TemporaryFile;
    use crate::filesystem::read_limited_string_from;
    use crate::filesystem::remove_directory_all;

    #[test]
    fn oversized_utf8_is_detected_before_decoding() {
        let result =
            read_limited_string_from(Cursor::new("aaaé"), 3).expect("the bounded read succeeds");

        assert!(matches!(result, LimitedString::TooLarge));
    }

    #[test]
    fn recursive_removal_accepts_an_absent_directory() {
        let path = env::temp_dir().join(format!("whim-absent-removal-test-{}", process::id()));

        remove_directory_all(&path).expect("removing an absent directory is complete");
    }

    #[test]
    fn temporary_files_are_private_before_they_are_written() {
        let directory = env::temp_dir().join(format!(
            "whim-private-temporary-file-test-{}",
            process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).expect("the temporary directory is creatable");

        let temporary = TemporaryFile::create(&directory).expect("the temporary file is created");
        let mode = fs::metadata(&temporary.path)
            .expect("the temporary file is inspectable")
            .permissions()
            .mode();

        assert_eq!(mode & 0o077, 0);
        drop(temporary);
        fs::remove_dir(&directory).expect("the temporary directory is removable");
    }
}
