use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_STANDARD_INFO, FileBasicInfo, FileStandardInfo, GetFileInformationByHandle,
    GetFileInformationByHandleEx,
};

use crate::file::{Metadata, Timestamp};
use crate::{Error, Result};

pub fn open(path: &Path, flags: i64, mode: i64) -> Result<File> {
    let mut options = OpenOptions::new();
    let access = flags & 3;
    options
        .read(access != i64::from(libc::O_WRONLY))
        .write(access != i64::from(libc::O_RDONLY))
        .append(flags & i64::from(libc::O_APPEND) != 0)
        .truncate(flags & i64::from(libc::O_TRUNC) != 0);
    let create = flags & i64::from(libc::O_CREAT) != 0;
    options
        .create(create)
        .create_new(create && flags & i64::from(libc::O_EXCL) != 0);
    if mode & 0o222 == 0 {
        options.attributes(FILE_ATTRIBUTE_READONLY);
    }

    options.open(path).map_err(|error| {
        Error::new(
            "CreateFileW",
            if error.kind() == io::ErrorKind::PermissionDenied && path.is_dir() {
                io::Error::from(io::ErrorKind::IsADirectory)
            } else {
                error
            },
        )
    })
}

pub(crate) fn open_metadata(path: &Path, follow: bool) -> Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS
                | if follow {
                    0
                } else {
                    FILE_FLAG_OPEN_REPARSE_POINT
                },
        )
        .open(path)
        .map_err(|error| Error::new(if follow { "stat" } else { "lstat" }, error))
}

pub fn path_metadata(path: &Path, follow: bool) -> Result<Metadata> {
    metadata(&open_metadata(path, follow)?)
}

pub fn metadata(file: &File) -> Result<Metadata> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let mut basic = FILE_BASIC_INFO::default();
    let mut standard = FILE_STANDARD_INFO::default();
    let handle = file.as_raw_handle();
    // SAFETY: the handle is live and all output buffers have the specified sizes.
    if unsafe {
        GetFileInformationByHandle(handle, &raw mut information) == 0
            || GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                (&raw mut basic).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            ) == 0
            || GetFileInformationByHandleEx(
                handle,
                FileStandardInfo,
                (&raw mut standard).cast(),
                size_of::<FILE_STANDARD_INFO>() as u32,
            ) == 0
    } {
        return Err(Error::last("fstat"));
    }

    let attributes = information.dwFileAttributes;
    let kind = if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && file
            .metadata()
            .map_err(|error| Error::new("fstat", error))?
            .file_type()
            .is_symlink()
    {
        0o120000
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        0o040000
    } else {
        0o100000
    };

    let time = |ticks: i64| {
        let ticks = ticks - 116_444_736_000_000_000;
        Timestamp {
            seconds: ticks.div_euclid(10_000_000),
            nanoseconds: ticks.rem_euclid(10_000_000) * 100,
        }
    };

    Ok(Metadata {
        mode: kind
            | 0o444
            | if attributes & FILE_ATTRIBUTE_READONLY == 0 {
                0o222
            } else {
                0
            }
            | if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                0o111
            } else {
                0
            },
        links: i64::from(information.nNumberOfLinks),
        user: -1,
        group: -1,
        size: standard.EndOfFile,
        block_size: 0,
        blocks: standard.AllocationSize / 512,
        device: i64::from(information.dwVolumeSerialNumber),
        inode: ((u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow))
        .cast_signed(),
        accessed: time(basic.LastAccessTime),
        modified: time(basic.LastWriteTime),
        changed: time(basic.ChangeTime),
    })
}

pub fn temporary(directory: &Path) -> Result<File> {
    if directory.as_os_str().is_empty() {
        return Err(Error::invalid("mkstemp"));
    }

    let path = directory.join(format!("whim-spool-{}", uuid::Uuid::new_v4()));
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(path)
        .map_err(|error| Error::new("mkstemp", error))
}

pub fn lock(file: &File, kind: i64, wait: bool) -> Result<bool> {
    let result = match (kind, wait) {
        (1, true) => fs2::FileExt::lock_shared(file),
        (2, true) => fs2::FileExt::lock_exclusive(file),
        (1, false) => fs2::FileExt::try_lock_shared(file),
        (2, false) => fs2::FileExt::try_lock_exclusive(file),
        (8, _) => fs2::FileExt::unlock(file),
        _ => return Err(Error::invalid("flock")),
    };

    match result {
        Ok(()) => Ok(true),
        Err(error)
            if !wait && error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            Ok(false)
        }
        Err(error) => Err(Error::new("flock", error)),
    }
}
