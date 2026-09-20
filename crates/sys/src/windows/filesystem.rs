use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt, symlink_dir, symlink_file};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, UNIX_EPOCH};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
};
use windows_sys::Win32::Foundation::{OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, UNICODE_STRING};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, GetDiskFreeSpaceExW,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use crate::file::{Metadata, Timestamp, metadata};
use crate::filesystem::{DirectoryEntry, Space};
use crate::path::{path_bytes, path_from_bytes};
use crate::{Descriptor, Error, Result};

pub fn open_directory(path: &Path) -> Option<Descriptor> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return None;
    }

    Some(Descriptor::from_file(file))
}

fn open_relative(parent: &File, name: &[u8], directory: bool) -> io::Result<File> {
    let path = path_from_bytes(name)?;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let length = u16::try_from(wide.len() * 2).map_err(io::Error::other)?;
    let name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &raw const name,
        Attributes: OBJ_CASE_INSENSITIVE,
        ..OBJECT_ATTRIBUTES::default()
    };

    let mut status = IO_STATUS_BLOCK::default();
    let mut handle = ptr::null_mut();
    let options = FILE_OPEN_REPARSE_POINT
        | FILE_SYNCHRONOUS_IO_NONALERT
        | if directory {
            FILE_DIRECTORY_FILE
        } else {
            FILE_NON_DIRECTORY_FILE
        };

    // SAFETY: parent remains open and all name, attribute, and output storage outlives the call.
    let result = unsafe {
        NtCreateFile(
            &raw mut handle,
            FILE_GENERIC_READ,
            &raw const attributes,
            &raw mut status,
            ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            options,
            ptr::null(),
            0,
        )
    };

    if result < 0 {
        // SAFETY: this converts an NT status without dereferencing memory.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) }.cast_signed(),
        ));
    }

    // SAFETY: NtCreateFile returned a new handle now owned by file.
    let file = unsafe { File::from_raw_handle(handle) };
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }

    Ok(file)
}

pub fn open_regular_file_beneath(
    root: &Descriptor,
    path: &[u8],
) -> Result<Option<(Descriptor, Metadata)>> {
    let mut parent = root.try_clone_file()?;
    let components: Vec<&[u8]> = path.split(|byte| matches!(byte, b'/' | b'\\')).collect();
    if components.iter().any(|part| {
        part.is_empty()
            || *part == b"."
            || *part == b".."
            || part.contains(&0)
            || part.contains(&b':')
    }) {
        return Ok(None);
    }

    let Some((name, directories)) = components.split_last() else {
        return Ok(None);
    };

    for directory in directories {
        let Ok(opened) = open_relative(&parent, directory, true) else {
            return Ok(None);
        };
        parent = opened;
    }

    let Ok(file) = open_relative(&parent, name, false) else {
        return Ok(None);
    };

    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Ok(None);
    }

    let Ok(metadata) = metadata(&file) else {
        return Ok(None);
    };

    Ok(Some((Descriptor::from_file(file), metadata)))
}

pub fn exists(path: &Path, follow: bool) -> Result<bool> {
    match if follow {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    } {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(Error::new("metadata", error)),
    }
}

pub fn check_access(
    path: &Path,
    read: bool,
    write: bool,
    execute: bool,
    _effective: bool,
) -> Result<bool> {
    let access = if read { FILE_GENERIC_READ } else { 0 }
        | if write { FILE_GENERIC_WRITE } else { 0 }
        | if execute { FILE_GENERIC_EXECUTE } else { 0 };
    match OpenOptions::new()
        .access_mode(access)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::NotADirectory
                    | io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(Error::new("CreateFileW", error)),
    }
}

pub fn create_directory(path: &Path, _mode: i64) -> Result<()> {
    fs::create_dir(path).map_err(|error| Error::new("CreateDirectoryW", error))
}

pub fn symbolic_link(target: &Path, path: &Path) -> Result<()> {
    let resolved = path.parent().unwrap_or_else(|| Path::new(".")).join(target);
    let result = if resolved.is_dir() {
        symlink_dir(target, path)
    } else {
        symlink_file(target, path)
    };

    result.map_err(|error| Error::new("CreateSymbolicLinkW", error))
}

pub fn create_named_pipe(_path: &Path, _mode: i64) -> Result<()> {
    Err(Error::Unsupported("filesystem FIFO nodes"))
}
pub fn set_mode(_path: &Path, _mode: i64) -> Result<()> {
    Err(Error::Unsupported("POSIX file permission modes"))
}
pub fn set_owner(_path: &Path, _user: i64, _group: i64, _follow: bool) -> Result<()> {
    Err(Error::Unsupported("POSIX file ownership"))
}

pub fn set_times(
    path: &Path,
    accessed: Timestamp,
    modified: Timestamp,
    follow: bool,
) -> Result<()> {
    let time = |stamp: Timestamp| {
        let nanos = u32::try_from(stamp.nanoseconds).map_err(|_| Error::invalid("SetFileTime"))?;
        if nanos >= 1_000_000_000 {
            return Err(Error::invalid("SetFileTime"));
        }

        let duration = Duration::from_secs(stamp.seconds.unsigned_abs());
        let time = if stamp.seconds < 0 {
            UNIX_EPOCH.checked_sub(duration)
        } else {
            UNIX_EPOCH.checked_add(duration)
        };

        time.and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanos))))
            .ok_or_else(|| Error::invalid("SetFileTime"))
    };

    let accessed = time(accessed)?;
    let modified = time(modified)?;
    let file = OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS
                | if follow {
                    0
                } else {
                    FILE_FLAG_OPEN_REPARSE_POINT
                },
        )
        .open(path)
        .map_err(|error| Error::new("CreateFileW", error))?;

    file.set_times(
        fs::FileTimes::new()
            .set_accessed(accessed)
            .set_modified(modified),
    )
    .map_err(|error| Error::new("SetFileTime", error))
}

pub fn read_directory(path: &Path) -> Result<Vec<DirectoryEntry>> {
    let entries = fs::read_dir(path).map_err(|error| Error::new("FindFirstFileW", error))?;
    entries
        .map(|entry| {
            let entry = entry.map_err(|error| Error::new("FindNextFileW", error))?;
            let kind = entry
                .file_type()
                .map_err(|error| Error::new("FindNextFileW", error))?;
            Ok(DirectoryEntry {
                name: path_bytes(Path::new(&entry.file_name())),
                mode: if kind.is_symlink() {
                    0o120000
                } else if kind.is_dir() {
                    0o040000
                } else {
                    0o100000
                },
            })
        })
        .collect()
}

pub fn space(path: &Path) -> Result<Space> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let (mut available, mut total, mut free) = (0, 0, 0);
    // SAFETY: path is null terminated and all output buffers remain writable.
    if unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &raw mut available,
            &raw mut total,
            &raw mut free,
        )
    } == 0
    {
        return Err(Error::last("GetDiskFreeSpaceExW"));
    }

    Ok(Space {
        block_size: 1,
        blocks: total,
        free,
        available,
    })
}

fn temporary_path(directory: &Path, prefix: &[u8]) -> Result<PathBuf> {
    if prefix
        .iter()
        .any(|byte| matches!(byte, 0 | b'/' | b'\\' | b':'))
    {
        return Err(Error::invalid("temporary_path"));
    }

    let mut name = prefix.to_vec();
    name.extend_from_slice(uuid::Uuid::new_v4().to_string().as_bytes());
    Ok(
        directory
            .join(path_from_bytes(&name).map_err(|error| Error::new("temporary_path", error))?),
    )
}

pub fn create_temporary_file(
    directory: &Path,
    prefix: &[u8],
    _mode: i64,
) -> Result<(File, PathBuf)> {
    let path = temporary_path(directory, prefix)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| Error::new("CreateFileW", error))?;
    Ok((file, path))
}

pub fn create_temporary_directory(directory: &Path, prefix: &[u8]) -> Result<PathBuf> {
    let path = temporary_path(directory, prefix)?;
    fs::create_dir(&path).map_err(|error| Error::new("CreateDirectoryW", error))?;
    Ok(path)
}
