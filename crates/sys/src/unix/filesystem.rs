use std::ffi::{CString, OsString};
use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use rustix::{fs, io::Errno};

use crate::file::{Metadata, Timestamp};
use crate::filesystem::{DirectoryEntry, Space};
use crate::platform::{descriptor::close_on_exec, file::from_stat};
use crate::{Descriptor, Error, Result};

fn mode(value: i64, call: &'static str) -> Result<fs::Mode> {
    libc::mode_t::try_from(value)
        .map(fs::Mode::from_raw_mode)
        .map_err(|_| Error::invalid(call))
}

pub fn open_directory(path: &Path) -> Option<Descriptor> {
    fs::openat(
        fs::CWD,
        path,
        fs::OFlags::RDONLY | fs::OFlags::DIRECTORY | fs::OFlags::NOFOLLOW | fs::OFlags::CLOEXEC,
        fs::Mode::empty(),
    )
    .ok()
    .map(Descriptor::owned)
}

pub fn open_regular_file_beneath(
    root: &Descriptor,
    path: &[u8],
) -> Result<Option<(Descriptor, Metadata)>> {
    if path.is_empty() || path[0] == b'/' {
        return Ok(None);
    }
    let mut components = path.split(|byte| *byte == b'/');
    let Some(name) = components.next_back() else {
        return Ok(None);
    };
    if name.is_empty() || name == b"." || name == b".." {
        return Ok(None);
    }
    let mut directory: Option<OwnedFd> = None;
    let flags =
        fs::OFlags::RDONLY | fs::OFlags::DIRECTORY | fs::OFlags::NOFOLLOW | fs::OFlags::CLOEXEC;
    for component in components {
        if component.is_empty() || component == b"." || component == b".." {
            return Ok(None);
        }
        let Ok(component) = CString::new(component) else {
            return Ok(None);
        };
        let parent = directory.as_ref().map_or(root.borrowed(), OwnedFd::as_fd);
        let Ok(opened) = fs::openat(parent, &component, flags, fs::Mode::empty()) else {
            return Ok(None);
        };
        directory = Some(opened);
    }
    let Ok(name) = CString::new(name) else {
        return Ok(None);
    };
    let parent = directory.as_ref().map_or(root.borrowed(), OwnedFd::as_fd);
    let Ok(file) = fs::openat(
        parent,
        &name,
        fs::OFlags::RDONLY | fs::OFlags::NOFOLLOW | fs::OFlags::CLOEXEC,
        fs::Mode::empty(),
    ) else {
        return Ok(None);
    };
    let Ok(metadata) = fs::fstat(&file) else {
        return Ok(None);
    };
    if fs::FileType::from_raw_mode(metadata.st_mode) != fs::FileType::RegularFile {
        return Ok(None);
    }
    Ok(Some((Descriptor::owned(file), from_stat(metadata))))
}

pub fn exists(path: &Path, follow: bool) -> Result<bool> {
    let flags = if follow {
        fs::AtFlags::empty()
    } else {
        fs::AtFlags::SYMLINK_NOFOLLOW
    };
    match fs::statat(fs::CWD, path, flags) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT | Errno::NOTDIR) => Ok(false),
        Err(error) => Err(Error::new("fstatat", error)),
    }
}

pub fn check_access(
    path: &Path,
    read: bool,
    write: bool,
    execute: bool,
    effective: bool,
) -> Result<bool> {
    let mut access = fs::Access::EXISTS;
    if read {
        access |= fs::Access::READ_OK;
    }
    if write {
        access |= fs::Access::WRITE_OK;
    }
    if execute {
        access |= fs::Access::EXEC_OK;
    }
    let flags = if effective {
        fs::AtFlags::from_bits_retain(libc::AT_EACCESS.cast_unsigned())
    } else {
        fs::AtFlags::empty()
    };
    match fs::accessat(fs::CWD, path, access, flags) {
        Ok(()) => Ok(true),
        Err(Errno::ACCESS | Errno::PERM | Errno::NOENT | Errno::NOTDIR) => Ok(false),
        Err(error) => Err(Error::new("faccessat", error)),
    }
}

pub fn create_directory(path: &Path, permissions: i64) -> Result<()> {
    fs::mkdirat(fs::CWD, path, mode(permissions, "mkdir")?)
        .map_err(|error| Error::new("mkdir", error))
}

pub fn symbolic_link(target: &Path, path: &Path) -> Result<()> {
    fs::symlinkat(target, fs::CWD, path).map_err(|error| Error::new("symlink", error))
}

pub fn create_named_pipe(path: &Path, permissions: i64) -> Result<()> {
    let mode = mode(permissions, "mkfifo")?;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| Error::invalid("mkfifo"))?;
    // SAFETY: path is null terminated and mode fits the platform type.
    if unsafe { libc::mkfifo(path.as_ptr(), mode.as_raw_mode()) } < 0 {
        Err(Error::last("mkfifo"))
    } else {
        Ok(())
    }
}

pub fn set_mode(path: &Path, permissions: i64) -> Result<()> {
    fs::chmodat(
        fs::CWD,
        path,
        mode(permissions, "chmod")?,
        fs::AtFlags::empty(),
    )
    .map_err(|error| Error::new("chmod", error))
}

pub fn set_owner(path: &Path, user: i64, group: i64, follow: bool) -> Result<()> {
    let user = if user == -1 || user == i64::from(libc::uid_t::MAX) {
        None
    } else {
        Some(fs::Uid::from_raw(
            libc::uid_t::try_from(user).map_err(|_| Error::invalid("fchownat"))?,
        ))
    };
    let group = if group == -1 || group == i64::from(libc::gid_t::MAX) {
        None
    } else {
        Some(fs::Gid::from_raw(
            libc::gid_t::try_from(group).map_err(|_| Error::invalid("fchownat"))?,
        ))
    };
    let flags = if follow {
        fs::AtFlags::empty()
    } else {
        fs::AtFlags::SYMLINK_NOFOLLOW
    };
    fs::chownat(fs::CWD, path, user, group, flags).map_err(|error| Error::new("fchownat", error))
}

pub fn set_times(
    path: &Path,
    accessed: Timestamp,
    modified: Timestamp,
    follow: bool,
) -> Result<()> {
    let times = fs::Timestamps {
        last_access: fs::Timespec {
            tv_sec: accessed.seconds,
            tv_nsec: fs::Nsecs::try_from(accessed.nanoseconds)
                .map_err(|_| Error::invalid("utimensat"))?,
        },
        last_modification: fs::Timespec {
            tv_sec: modified.seconds,
            tv_nsec: fs::Nsecs::try_from(modified.nanoseconds)
                .map_err(|_| Error::invalid("utimensat"))?,
        },
    };
    let flags = if follow {
        fs::AtFlags::empty()
    } else {
        fs::AtFlags::SYMLINK_NOFOLLOW
    };
    fs::utimensat(fs::CWD, path, &times, flags).map_err(|error| Error::new("utimensat", error))
}

pub fn read_directory(path: &Path) -> Result<Vec<DirectoryEntry>> {
    let file = fs::openat(
        fs::CWD,
        path,
        fs::OFlags::RDONLY | fs::OFlags::DIRECTORY | fs::OFlags::CLOEXEC,
        fs::Mode::empty(),
    )
    .map_err(|error| Error::new("opendir", error))?;
    let directory = fs::Dir::new(file).map_err(|error| Error::new("opendir", error))?;
    let mut entries = Vec::new();
    for entry in directory {
        let entry = entry.map_err(|error| Error::new("readdir", error))?;
        let name = entry.file_name().to_bytes();
        if name.is_empty() || name == b"." || name == b".." {
            continue;
        }
        let mode = match entry.file_type() {
            fs::FileType::Unknown => 0,
            kind => i64::from(kind.as_raw_mode()),
        };
        entries.push(DirectoryEntry {
            name: name.to_vec(),
            mode,
        });
    }
    Ok(entries)
}

pub fn space(path: &Path) -> Result<Space> {
    let info = fs::statvfs(path).map_err(|error| Error::new("statvfs", error))?;
    Ok(Space {
        block_size: info.f_frsize as u64,
        blocks: info.f_blocks as u64,
        free: info.f_bfree as u64,
        available: info.f_bavail as u64,
    })
}

fn temporary_template(directory: &Path, prefix: &[u8], call: &'static str) -> Result<Vec<u8>> {
    if prefix.contains(&0) || prefix.contains(&b'/') {
        return Err(Error::invalid(call));
    }
    let mut name = prefix.to_vec();
    name.extend_from_slice(b"XXXXXX");
    CString::new(
        directory
            .join(OsString::from_vec(name))
            .as_os_str()
            .as_bytes(),
    )
    .map(CString::into_bytes_with_nul)
    .map_err(|_| Error::invalid(call))
}

pub fn create_temporary_file(
    directory: &Path,
    prefix: &[u8],
    permissions: i64,
) -> Result<(File, PathBuf)> {
    let mode = mode(permissions, "fchmod")?;
    let mut template = temporary_template(directory, prefix, "mkstemp")?;
    // SAFETY: template is writable, null terminated, and ends in six X bytes.
    let raw = unsafe { libc::mkstemp(template.as_mut_ptr().cast()) };
    if raw < 0 {
        return Err(Error::last("mkstemp"));
    }
    // SAFETY: mkstemp returned a new descriptor now owned by file.
    let file = unsafe { File::from_raw_fd(raw) };
    fs::fchmod(&file, mode).map_err(|error| Error::new("fchmod", error))?;
    close_on_exec(file.as_raw_fd())?;
    template.pop();
    Ok((file, PathBuf::from(OsString::from_vec(template))))
}

pub fn create_temporary_directory(directory: &Path, prefix: &[u8]) -> Result<PathBuf> {
    let mut template = temporary_template(directory, prefix, "mkdtemp")?;
    // SAFETY: template is writable, null terminated, and ends in six X bytes.
    if unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) }.is_null() {
        return Err(Error::last("mkdtemp"));
    }
    template.pop();
    Ok(PathBuf::from(OsString::from_vec(template)))
}
