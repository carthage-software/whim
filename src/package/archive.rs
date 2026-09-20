use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::path::StripPrefixError;
use std::process::ExitStatus;

use thiserror::Error as ThisError;
use whim_sys::path::{path_bytes, path_from_bytes};

use crate::package::filesystem::Error as FilesystemError;
use crate::package::filesystem::sync_directory;
use crate::package::git::Error as GitError;
use crate::package::git::Repository;

const MAXIMUM_ENTRIES: usize = 100_000;
const MAXIMUM_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAXIMUM_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAXIMUM_PATH_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug)]
pub(crate) enum IoOperation {
    Copy,
    Create,
    Extract,
    Inspect,
    Read,
    SetMode,
    Sync,
}

impl fmt::Display for IoOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Copy => "copy",
            Self::Create => "create",
            Self::Extract => "extract",
            Self::Inspect => "inspect",
            Self::Read => "read",
            Self::SetMode => "set the mode on",
            Self::Sync => "sync",
        })
    }
}

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Filesystem(#[from] FilesystemError),
    #[error("could not {operation} `{}`: {source}", path.display())]
    Io {
        operation: IoOperation,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the Git archive did not provide output")]
    MissingGitOutput,
    #[error("could not wait for Git archive: {0}")]
    WaitForGit(#[source] io::Error),
    #[error("the Git archive exited with {status}: {stderr}")]
    GitFailed { status: ExitStatus, stderr: String },
    #[error("invalid Git archive: {0}")]
    InvalidArchive(#[source] io::Error),
    #[error("invalid Git archive entry: {0}")]
    InvalidEntry(#[source] io::Error),
    #[error("archive entry count overflowed")]
    EntryCountOverflow,
    #[error("archive contains more than {limit} entries")]
    TooManyEntries { limit: usize },
    #[error("archive contains an empty path")]
    EmptyPath,
    #[error("archive path exceeds {limit} bytes")]
    OversizedPath { limit: usize },
    #[error("unsafe archive path `{}`", .0.display())]
    UnsafePath(PathBuf),
    #[error("archive entry `{}` is not a regular file or directory", .0.display())]
    UnsupportedEntry(PathBuf),
    #[error("installed path `{}` has an unsafe file type", .0.display())]
    UnsafeInstalledPath(PathBuf),
    #[error("installed path escaped its package root")]
    EscapedRoot(#[source] StripPrefixError),
    #[error("installed path `{path}` exceeds {limit} bytes", path = path.display())]
    InstalledPathTooLong { path: PathBuf, limit: usize },
    #[error("installed package contains more than {limit} entries")]
    TooManyInstalledEntries { limit: usize },
    #[error("file `{path}` is {size} bytes, exceeding the {limit} byte limit", path = path.display())]
    FileTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    #[error("package size overflowed")]
    SizeOverflow,
    #[error("package exceeds the {limit} byte limit")]
    PackageTooLarge { limit: u64 },
}

impl Error {
    fn io(operation: IoOperation, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(%commit, destination = %destination.display()),
)]
pub(crate) fn export(
    repository: &Repository,
    commit: &str,
    destination: &Path,
) -> Result<String, Error> {
    fs::create_dir(destination)
        .map_err(|source| Error::io(IoOperation::Create, destination, source))?;

    let mut child = repository.archive(commit)?;
    let stdout = child.stdout.take().ok_or(Error::MissingGitOutput)?;
    let extraction = extract(stdout, destination);
    let output = child.wait_with_output();
    if let Err(error) = extraction {
        if let Err(wait_error) = output {
            tracing::debug!(error = ?wait_error, "failed to reap rejected Git archive");
        }

        return Err(error);
    }

    let output = output.map_err(Error::WaitForGit)?;
    if !output.status.success() {
        return Err(Error::GitFailed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    checksum(destination)
}

#[tracing::instrument(level = "trace", skip_all, fields(root = %root.display()))]
pub(crate) fn checksum(root: &Path) -> Result<String, Error> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|source| Error::io(IoOperation::Inspect, root, source))?;

    if !metadata.file_type().is_dir() {
        return Err(Error::UnsafeInstalledPath(root.to_path_buf()));
    }

    let mut entries = Vec::new();
    collect_entries(root, root, &mut entries)?;
    entries.sort_by_cached_key(|path| checksum_path(path));

    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    for relative in entries {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|source| Error::io(IoOperation::Inspect, &path, source))?;
        let path_bytes = checksum_path(&relative);
        hasher.update(&(path_bytes.len() as u64).to_le_bytes());
        hasher.update(&path_bytes);
        let executable = executable(&path, &metadata)?;
        if metadata.is_dir() {
            hasher.update(&[0, executable]);
            hasher.update(&0_u64.to_le_bytes());
        } else if metadata.is_file() {
            validate_file_size(&path, metadata.len(), &mut total)?;
            hasher.update(&[1, executable]);
            hasher.update(&metadata.len().to_le_bytes());
            let mut file =
                File::open(&path).map_err(|source| Error::io(IoOperation::Read, &path, source))?;

            loop {
                let count = file
                    .read(&mut buffer)
                    .map_err(|source| Error::io(IoOperation::Read, &path, source))?;

                if count == 0 {
                    break;
                }

                hasher.update(&buffer[..count]);
            }
        } else {
            return Err(Error::UnsafeInstalledPath(path));
        }
    }

    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(source = %source.display(), destination = %destination.display()),
)]
pub(crate) fn copy(source: &Path, destination: &Path) -> Result<(), Error> {
    fs::create_dir(destination)
        .map_err(|error| Error::io(IoOperation::Create, destination, error))?;
    for entry in
        fs::read_dir(source).map_err(|error| Error::io(IoOperation::Read, source, error))?
    {
        let entry = entry.map_err(|error| Error::io(IoOperation::Read, source, error))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| Error::io(IoOperation::Inspect, &from, error))?;
        if file_type.is_dir() {
            copy(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to).map_err(|error| Error::io(IoOperation::Copy, &from, error))?;
            fs::set_permissions(
                &to,
                fs::metadata(&from)
                    .map_err(|error| Error::io(IoOperation::Inspect, &from, error))?
                    .permissions(),
            )
            .map_err(|error| Error::io(IoOperation::SetMode, &to, error))?;
            File::options()
                .write(true)
                .open(&to)
                .and_then(|file| file.sync_all())
                .map_err(|error| Error::io(IoOperation::Sync, &to, error))?;
        } else {
            return Err(Error::UnsafeInstalledPath(from));
        }
    }

    sync_directory(destination)?;
    Ok(())
}

fn extract(reader: impl Read, destination: &Path) -> Result<(), Error> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(Error::InvalidArchive)?;
    let mut directories = BTreeSet::from([destination.to_path_buf()]);
    let mut count = 0_usize;
    let mut total = 0_u64;
    for entry in entries {
        let mut entry = entry.map_err(Error::InvalidEntry)?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_pax_global_extensions() || entry_type.is_pax_local_extensions() {
            continue;
        }

        count = count.checked_add(1).ok_or(Error::EntryCountOverflow)?;
        if count > MAXIMUM_ENTRIES {
            return Err(Error::TooManyEntries {
                limit: MAXIMUM_ENTRIES,
            });
        }

        let path = entry.path_bytes();
        if path.is_empty() {
            return Err(Error::EmptyPath);
        }

        if path.len() > MAXIMUM_PATH_BYTES {
            return Err(Error::OversizedPath {
                limit: MAXIMUM_PATH_BYTES,
            });
        }

        #[cfg(windows)]
        let has_backslash = path.contains(&b'\\');
        let path = path_from_bytes(&path).map_err(Error::InvalidEntry)?;
        let path = path.as_path();
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(Error::UnsafePath(path.to_path_buf()));
        }
        #[cfg(windows)]
        if has_backslash
            || path.components().any(|component| {
                let name = component.as_os_str().as_encoded_bytes();
                name.iter().any(|byte| {
                    matches!(
                        byte,
                        b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*' | 0..=31
                    )
                }) || name.ends_with(b".")
                    || name.ends_with(b" ")
                    || reserved_windows_name(name)
            })
        {
            return Err(Error::UnsafePath(path.to_path_buf()));
        }

        let size = entry.size();
        validate_file_size(path, size, &mut total)?;
        let target = destination.join(path);
        let mode = entry.header().mode().map_err(Error::InvalidEntry)? & 0o111;
        if entry_type.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| Error::io(IoOperation::Create, &target, error))?;
            set_mode(&target, 0o755)
                .map_err(|error| Error::io(IoOperation::SetMode, &target, error))?;
            record_directories(destination, &target, &mut directories);
        } else if entry_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| Error::io(IoOperation::Create, parent, error))?;
                record_directories(destination, parent, &mut directories);
            }

            let mut file = File::create(&target)
                .map_err(|error| Error::io(IoOperation::Create, &target, error))?;
            io::copy(&mut entry, &mut file)
                .map_err(|error| Error::io(IoOperation::Extract, &target, error))?;
            set_mode(&target, 0o644 | mode)
                .map_err(|error| Error::io(IoOperation::SetMode, &target, error))?;
            file.sync_all()
                .map_err(|error| Error::io(IoOperation::Sync, &target, error))?;
        } else {
            return Err(Error::UnsupportedEntry(path.to_path_buf()));
        }
    }

    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by_key(|path| Reverse(path.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }

    Ok(())
}

fn record_directories(root: &Path, path: &Path, directories: &mut BTreeSet<PathBuf>) {
    for directory in path.ancestors() {
        if !directory.starts_with(root) {
            break;
        }

        directories.insert(directory.to_path_buf());
        if directory == root {
            break;
        }
    }
}

fn collect_entries(root: &Path, directory: &Path, entries: &mut Vec<PathBuf>) -> Result<(), Error> {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut children = fs::read_dir(&directory)
            .map_err(|error| Error::io(IoOperation::Read, &directory, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| Error::io(IoOperation::Read, &directory, error))?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let path = child.path();
            let relative = path
                .strip_prefix(root)
                .map_err(Error::EscapedRoot)?
                .to_path_buf();
            if path_bytes(&relative).len() > MAXIMUM_PATH_BYTES {
                return Err(Error::InstalledPathTooLong {
                    path: relative,
                    limit: MAXIMUM_PATH_BYTES,
                });
            }

            if entries.len() >= MAXIMUM_ENTRIES {
                return Err(Error::TooManyInstalledEntries {
                    limit: MAXIMUM_ENTRIES,
                });
            }

            entries.push(relative);
            if child
                .file_type()
                .map_err(|error| Error::io(IoOperation::Inspect, &path, error))?
                .is_dir()
            {
                pending.push(path);
            }
        }
    }

    Ok(())
}

fn checksum_path(path: &Path) -> Vec<u8> {
    let bytes = path_bytes(path);
    #[cfg(windows)]
    let bytes = bytes
        .into_iter()
        .map(|byte| if byte == b'\\' { b'/' } else { byte })
        .collect();
    bytes
}

#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "reading the Windows executable-bit stream can fail"
)]
fn executable(_path: &Path, metadata: &fs::Metadata) -> Result<u8, Error> {
    Ok(u8::from(metadata.permissions().mode() & 0o111 != 0))
}

#[cfg(windows)]
fn executable(path: &Path, metadata: &fs::Metadata) -> Result<u8, Error> {
    if metadata.is_dir() {
        return Ok(1);
    }
    match fs::read(mode_stream(path)) {
        Ok(mode) if mode == b"1" => Ok(1),
        Ok(_) => Err(Error::UnsafeInstalledPath(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(Error::io(IoOperation::Read, path, error)),
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(windows)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    if mode & 0o111 != 0 && path.is_file() {
        use std::io::Write;
        let mut stream = File::create(mode_stream(path))?;
        stream.write_all(b"1")?;
        stream.sync_all()?;
    } else if path.is_file() {
        match fs::remove_file(mode_stream(path)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn mode_stream(path: &Path) -> PathBuf {
    let mut path = path.as_os_str().to_os_string();
    path.push(":whim-executable");
    path.into()
}

#[cfg(windows)]
fn reserved_windows_name(name: &[u8]) -> bool {
    let name = name.split(|byte| *byte == b'.').next().unwrap_or_default();
    [
        b"CON".as_slice(),
        b"PRN",
        b"AUX",
        b"NUL",
        b"CONIN$",
        b"CONOUT$",
    ]
    .iter()
    .any(|reserved| name.eq_ignore_ascii_case(reserved))
        || (name.len() == 4
            && (name[..3].eq_ignore_ascii_case(b"COM") || name[..3].eq_ignore_ascii_case(b"LPT"))
            && matches!(name[3], b'1'..=b'9'))
        || (name.len() == 5
            && (name[..3].eq_ignore_ascii_case(b"COM") || name[..3].eq_ignore_ascii_case(b"LPT"))
            && name[3] == 0xc2
            && matches!(name[4], 0xb9 | 0xb2 | 0xb3))
}

fn validate_file_size(path: &Path, size: u64, total: &mut u64) -> Result<(), Error> {
    if size > MAXIMUM_FILE_BYTES {
        return Err(Error::FileTooLarge {
            path: path.to_path_buf(),
            size,
            limit: MAXIMUM_FILE_BYTES,
        });
    }

    *total = total.checked_add(size).ok_or(Error::SizeOverflow)?;
    if *total > MAXIMUM_TOTAL_BYTES {
        return Err(Error::PackageTooLarge {
            limit: MAXIMUM_TOTAL_BYTES,
        });
    }

    Ok(())
}
