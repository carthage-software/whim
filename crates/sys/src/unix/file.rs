use std::ffi::{CStr, CString};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fs;
use rustix::io::Errno;

use crate::file::{Metadata, Timestamp};
use crate::platform::descriptor::close_on_exec;
use crate::{Error, Result};

pub fn open(path: &Path, flags: i64, mode: i64) -> Result<File> {
    let flags = i32::try_from(flags).map_err(|_| Error::invalid("open"))?;
    let mode = libc::mode_t::try_from(mode).map_err(|_| Error::invalid("open"))?;
    fs::openat(
        fs::CWD,
        path,
        fs::OFlags::from_bits_retain(flags.cast_unsigned()) | fs::OFlags::CLOEXEC,
        fs::Mode::from_raw_mode(mode),
    )
    .map(File::from)
    .map_err(|error| Error::new("open", error))
}

pub fn metadata(file: &File) -> Result<Metadata> {
    fs::fstat(file)
        .map(from_stat)
        .map_err(|error| Error::new("fstat", error))
}

pub fn path_metadata(path: &Path, follow: bool) -> Result<Metadata> {
    let flags = if follow {
        fs::AtFlags::empty()
    } else {
        fs::AtFlags::SYMLINK_NOFOLLOW
    };
    fs::statat(fs::CWD, path, flags)
        .map(from_stat)
        .map_err(|error| Error::new(if follow { "stat" } else { "lstat" }, error))
}

pub(crate) fn from_stat(stat: fs::Stat) -> Metadata {
    let integer = |value: i128| value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
    let unsigned =
        |value: i128| u64::try_from(value).unwrap_or_else(|_| integer(value).cast_unsigned());
    #[cfg(target_vendor = "apple")]
    let device = u64::from(stat.st_dev.cast_unsigned());
    #[cfg(not(target_vendor = "apple"))]
    let device = unsigned(stat.st_dev.into());
    Metadata {
        mode: unsigned(stat.st_mode.into()),
        links: unsigned(stat.st_nlink.into()),
        user: integer(stat.st_uid.into()),
        group: integer(stat.st_gid.into()),
        size: unsigned(stat.st_size.into()),
        block_size: unsigned(stat.st_blksize.into()),
        blocks: unsigned(stat.st_blocks.into()),
        device,
        inode: unsigned(stat.st_ino.into()),
        accessed: Timestamp {
            seconds: integer(stat.st_atime.into()),
            nanoseconds: unsigned(stat.st_atime_nsec.into()),
        },
        modified: Timestamp {
            seconds: integer(stat.st_mtime.into()),
            nanoseconds: unsigned(stat.st_mtime_nsec.into()),
        },
        changed: Timestamp {
            seconds: integer(stat.st_ctime.into()),
            nanoseconds: unsigned(stat.st_ctime_nsec.into()),
        },
    }
}

pub fn lock(file: &File, kind: i64, wait: bool) -> Result<bool> {
    let operation = match (kind, wait) {
        (value, true) if value == i64::from(libc::LOCK_SH) => fs::FlockOperation::LockShared,
        (value, true) if value == i64::from(libc::LOCK_EX) => fs::FlockOperation::LockExclusive,
        (value, true) if value == i64::from(libc::LOCK_UN) => fs::FlockOperation::Unlock,
        (value, false) if value == i64::from(libc::LOCK_SH) => {
            fs::FlockOperation::NonBlockingLockShared
        }
        (value, false) if value == i64::from(libc::LOCK_EX) => {
            fs::FlockOperation::NonBlockingLockExclusive
        }
        (value, false) if value == i64::from(libc::LOCK_UN) => {
            fs::FlockOperation::NonBlockingUnlock
        }
        _ => return Err(Error::invalid("flock")),
    };
    match fs::flock(file, operation) {
        Ok(()) => Ok(true),
        Err(Errno::AGAIN) if !wait => Ok(false),
        Err(error) => Err(Error::new("flock", error)),
    }
}

pub fn temporary(directory: &Path) -> Result<File> {
    if directory.as_os_str().is_empty() {
        return Err(Error::invalid("mkstemp"));
    }
    let path = directory.join("whim-spool-XXXXXX");
    let mut template = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::invalid("mkstemp"))?
        .into_bytes_with_nul();
    // SAFETY: template is writable, null terminated, and ends in six X bytes.
    let raw = unsafe { libc::mkstemp(template.as_mut_ptr().cast()) };
    if raw < 0 {
        return Err(Error::last("mkstemp"));
    }
    // SAFETY: mkstemp returned a new descriptor now owned by file.
    let file = unsafe { File::from_raw_fd(raw) };
    let path = CStr::from_bytes_with_nul(&template).map_err(|_| Error::invalid("mkstemp"))?;
    let result = fs::fchmod(&file, fs::Mode::from_raw_mode(0o600))
        .map_err(|error| Error::new("fchmod", error))
        .and_then(|()| close_on_exec(file.as_raw_fd()));
    let removed = fs::unlinkat(fs::CWD, path, fs::AtFlags::empty())
        .map_err(|error| Error::new("unlink", error));
    result?;
    removed?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use rustix::fs;

    use super::from_stat;

    #[test]
    fn metadata_preserves_unsigned_inode_bits() {
        let mut stat = fs::statat(fs::CWD, "/", fs::AtFlags::empty()).unwrap();
        stat.st_ino = !0;
        stat.st_dev = !0;
        stat.st_atime = -1;
        let device_bits = size_of_val(&stat.st_dev) * 8;
        let metadata = from_stat(stat);
        assert_eq!(metadata.inode, u64::MAX);
        assert_eq!(metadata.device, u64::MAX >> (64 - device_bits));
        assert_eq!(metadata.accessed.seconds, -1);
    }
}
