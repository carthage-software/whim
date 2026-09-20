#[cfg(unix)]
use rustix::io as unix_io;
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::{Error, Result};

pub use crate::platform::file::{lock, metadata, open, path_metadata, temporary};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanoseconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub mode: i64,
    pub links: i64,
    pub user: i64,
    pub group: i64,
    pub size: i64,
    pub block_size: i64,
    pub blocks: i64,
    pub device: i64,
    pub inode: i64,
    pub accessed: Timestamp,
    pub modified: Timestamp,
    pub changed: Timestamp,
}

pub fn read(file: &File, maximum: usize) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        let mut bytes = Vec::<u8>::with_capacity(maximum);
        let (initialized, _) = unix_io::read(file, bytes.spare_capacity_mut())
            .map_err(|error| Error::new("read", error))?;
        let count = initialized.len();
        // SAFETY: read initialized exactly count bytes within capacity.
        unsafe { bytes.set_len(count) };
        Ok(bytes)
    }
    #[cfg(windows)]
    {
        let mut file = file;
        let mut bytes = vec![0; maximum];
        let count = file
            .read(&mut bytes)
            .map_err(|error| Error::new("read", error))?;
        bytes.truncate(count);
        Ok(bytes)
    }
}

pub fn write(mut file: &File, bytes: &[u8]) -> Result<usize> {
    file.write(bytes)
        .map_err(|error| Error::new("write", error))
}

pub fn synchronize(file: &File) -> Result<()> {
    file.sync_all().map_err(|error| Error::new("fsync", error))
}

pub fn truncate(file: &File, length: u64) -> Result<()> {
    file.set_len(length)
        .map_err(|error| Error::new("ftruncate", error))
}

pub fn seek(mut file: &File, offset: i64, origin: i64) -> Result<u64> {
    let origin = match origin {
        0 => SeekFrom::Start(u64::try_from(offset).map_err(|_| Error::invalid("lseek"))?),
        1 => SeekFrom::Current(offset),
        2 => SeekFrom::End(offset),
        _ => return Err(Error::invalid("lseek")),
    };
    file.seek(origin)
        .map_err(|error| Error::new("lseek", error))
}

pub fn read_path(path: &Path, offset: u64, maximum: Option<u64>) -> Result<Vec<u8>> {
    let mut file = open(path, i64::from(libc::O_RDONLY), 0)?;
    let metadata = file
        .metadata()
        .map_err(|error| Error::new("fstat", error))?;
    if !metadata.is_file() {
        return Err(Error::new("fstat", ErrorKind::IsADirectory));
    }
    if offset != 0 {
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| Error::new("lseek", error))?;
    }
    let available = metadata.len().saturating_sub(offset);
    let expected = maximum.map_or(available, |maximum| available.min(maximum));
    let capacity = usize::try_from(expected).unwrap_or(usize::MAX);
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| Error::new("read", ErrorKind::OutOfMemory))?;
    file.take(maximum.unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .map_err(|error| Error::new("read", error))?;
    Ok(bytes)
}
