use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::package::archive;
use crate::package::filesystem::hash_file;
use crate::package::install::Error;
use crate::package::install::selection::locked_package;
use crate::package::install::selection::selected_locked_sources;
use crate::package::lock::LockFile;
use crate::package::lock::LockedPackage;
use crate::package::parallel;
use crate::package::source::Source;
use crate::package::state;

#[tracing::instrument(level = "debug", skip(lock), fields(no_dev))]
pub(in crate::package) fn installation_is_current(
    project: &Path,
    lock: &LockFile,
    no_dev: bool,
) -> Result<bool, Error> {
    let Some(state) = state::read(project)? else {
        return Ok(false);
    };

    if state.version != 1
        || state.lock
            != format!(
                "blake3:{}",
                blake3::hash(lock.render()?.as_bytes()).to_hex()
            )
        || !state.matches_mode(no_dev)
    {
        return Ok(false);
    }

    let autoload_path = project.join("vendor/autoload.whim");
    if !matches!(path_type(&autoload_path)?, Some(file_type) if file_type.is_file()) {
        return Ok(false);
    }

    let selected = selected_locked_sources(lock, no_dev)?;
    if state.packages.len() != selected.len() {
        return Ok(false);
    }

    if state.autoload != hash_file(&autoload_path)? {
        return Ok(false);
    }

    let selected = selected.into_iter().collect::<Vec<_>>();
    if !selected.is_empty() {
        tracing::info!(
            packages = selected.len(),
            workers = parallel::workers(selected.len()),
            "verifying installed dependencies"
        );
    }
    let valid = parallel::try_map(
        &selected,
        |source| installed_package_is_current(project, lock, &state.packages, source),
        Error::CreateWorkerPool,
    )?;

    Ok(valid.into_iter().all(|valid| valid))
}

fn installed_package_is_current(
    project: &Path,
    lock: &LockFile,
    checksums: &BTreeMap<String, String>,
    source: &Source,
) -> Result<bool, Error> {
    let Some(expected) = checksums.get(source.identity()) else {
        return Ok(false);
    };
    if locked_package(lock, source)?.checksum != *expected {
        return Ok(false);
    }

    let directory = project.join("vendor/packages").join(source.digest());
    if !matches!(path_type(&directory)?, Some(file_type) if file_type.is_dir()) {
        return Ok(false);
    }

    Ok(archive::checksum(&directory)? == *expected)
}

pub(super) fn reuse_package(
    source: &Source,
    package: &LockedPackage,
    existing: &Path,
    destination: &Path,
) -> Result<Option<String>, Error> {
    if !matches!(path_type(existing)?, Some(file_type) if file_type.is_dir()) {
        return Ok(None);
    }

    let checksum = match archive::checksum(existing) {
        Ok(checksum) if checksum == package.checksum => checksum,
        Ok(checksum) => {
            tracing::debug!(
                package = %source,
                actual = checksum,
                expected = package.checksum,
                "installed package checksum changed"
            );
            return Ok(None);
        }
        Err(error) => {
            tracing::debug!(
                package = %source,
                ?error,
                "installed package could not be verified"
            );
            return Ok(None);
        }
    };

    archive::copy(existing, destination)?;
    let copied = archive::checksum(destination)?;
    if copied != checksum {
        return Err(Error::ChecksumMismatch {
            package: source.to_string(),
            expected: checksum,
            actual: copied,
        });
    }

    tracing::trace!(package = %source, "reused installed package");
    Ok(Some(copied))
}

fn path_type(path: &Path) -> Result<Option<fs::FileType>, Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata.file_type())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::InspectInstalledPath {
            path: path.to_path_buf(),
            source,
        }),
    }
}
