use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::{env, fs};

pub use crate::platform::filesystem::*;

pub struct DirectoryEntry {
    pub name: Vec<u8>,
    pub mode: i64,
}

pub struct Space {
    pub block_size: u64,
    pub blocks: u64,
    pub free: u64,
    pub available: u64,
}

pub fn remove_directory(path: &Path) -> Result<()> {
    fs::remove_dir(path).map_err(|error| Error::new("rmdir", error))
}

pub fn remove_file(path: &Path) -> Result<()> {
    fs::remove_file(path).map_err(|error| Error::new("unlink", error))
}

pub fn rename(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|error| Error::new("rename", error))
}

pub fn hard_link(target: &Path, path: &Path) -> Result<()> {
    fs::hard_link(target, path).map_err(|error| Error::new("link", error))
}

pub fn read_link(path: &Path) -> Result<PathBuf> {
    fs::read_link(path).map_err(|error| Error::new("readlink", error))
}

pub fn canonicalize(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|error| Error::new("realpath", error))
}

pub fn temporary_directory() -> PathBuf {
    env::temp_dir()
}
