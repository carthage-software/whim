mod error;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

pub(crate) use error::Error;
use error::IoAction;
use error::MoveAction;

use crate::config::MANIFEST_NAME;
use crate::package::filesystem::sync_directory;
use crate::package::filesystem::write_file;
use crate::package::install::StagedInstallation;
use crate::package::source::Source;
use crate::package::state;

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(project = %project.display(), no_dev, packages = stage.installed.len()),
)]
pub(crate) fn commit(
    project: &Path,
    stage: StagedInstallation,
    lock_text: &str,
    manifest_text: Option<&str>,
    no_dev: bool,
) -> Result<(), Error> {
    let StagedInstallation {
        directory,
        checksums,
        installed,
    } = stage;
    let changes_manifest = manifest_text.is_some();
    let payload = CommitPayload {
        directory,
        checksums,
        installed,
        lock: lock_text,
        changes_manifest,
        no_dev,
    };

    let paths = TransactionPaths::new(project);
    fs::create_dir_all(&paths.manager)
        .map_err(|source| Error::io(IoAction::Create, &paths.manager, source))?;
    let stage_name = payload
        .directory
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| Error::InvalidStageName(payload.directory.clone()))?;
    if payload.directory.parent() != Some(paths.stages.as_path())
        || !stage_name.starts_with("stage-")
    {
        return Err(Error::InvalidStageName(payload.directory.clone()));
    }

    if let Some(manifest) = manifest_text
        && let Err(error) = write_file(&payload.directory.join(MANIFEST_NAME), manifest.as_bytes())
    {
        remove_after_commit(&payload.directory);
        return Err(error.into());
    }

    let transaction = Transaction::new(&paths, changes_manifest)?;

    if let Err(error) = write_file(&paths.marker, b"") {
        remove_after_commit(&payload.directory);
        return Err(error.into());
    }

    sync_directory(&paths.manager)?;
    tracing::debug!(stage = stage_name, "recorded dependency transaction");
    tracing::info!("applying dependency changes");

    if let Err(error) = apply(&paths, &payload, &transaction) {
        if let Err(restore) = restore(&paths, &transaction, &payload.directory) {
            return Err(Error::Rollback {
                failure: Box::new(error),
                restore: Box::new(restore),
            });
        }

        return Err(error);
    }

    if let Err(source) = fs::remove_file(&paths.marker) {
        let error = Error::io(IoAction::Remove, &paths.marker, source);
        if let Err(restore) = restore(&paths, &transaction, &payload.directory) {
            return Err(Error::Rollback {
                failure: Box::new(error),
                restore: Box::new(restore),
            });
        }

        return Err(error);
    }

    sync_directory(&paths.manager)?;
    for (_, _, backup) in paths.artifacts() {
        remove_after_commit(backup);
    }

    remove_after_commit(&payload.directory);
    sync_directory(&paths.project)?;
    sync_directory(&paths.manager)?;
    tracing::debug!(stage = stage_name, "committed dependency transaction");
    Ok(())
}

fn apply(
    paths: &TransactionPaths,
    payload: &CommitPayload<'_>,
    transaction: &Transaction,
) -> Result<(), Error> {
    back_up(paths, transaction)?;
    sync_directory(&paths.vendor)?;
    sync_directory(&paths.project)?;
    sync_directory(&paths.manager)?;
    fs::rename(payload.directory.join("packages"), &paths.packages).map_err(|source| {
        Error::move_path(
            MoveAction::Install,
            payload.directory.join("packages"),
            &paths.packages,
            source,
        )
    })?;

    fs::rename(payload.directory.join("autoload.whim"), &paths.autoload).map_err(|source| {
        Error::move_path(
            MoveAction::Install,
            payload.directory.join("autoload.whim"),
            &paths.autoload,
            source,
        )
    })?;

    sync_directory(&paths.vendor)?;
    sync_directory(&paths.project)?;
    write_file(&paths.lock, payload.lock.as_bytes())?;
    if payload.changes_manifest {
        let staged = payload.directory.join(MANIFEST_NAME);
        fs::rename(&staged, &paths.manifest).map_err(|source| {
            Error::move_path(MoveAction::Install, staged, &paths.manifest, source)
        })?;
    }

    state::write(
        &paths.manager,
        payload.lock,
        payload.no_dev,
        &payload.installed,
        &payload.checksums,
    )?;

    sync_directory(&paths.vendor)?;
    sync_directory(&paths.project)?;
    sync_directory(&paths.manager)?;
    Ok(())
}

struct CommitPayload<'a> {
    directory: PathBuf,
    checksums: BTreeMap<Source, String>,
    installed: BTreeSet<Source>,
    lock: &'a str,
    changes_manifest: bool,
    no_dev: bool,
}

fn back_up(paths: &TransactionPaths, transaction: &Transaction) -> Result<(), Error> {
    for (artifact, current, backup) in paths.artifacts() {
        if transaction.previous.contains(&artifact) {
            if artifact == Artifact::Manifest {
                fs::hard_link(current, backup).map_err(|source| {
                    Error::move_path(MoveAction::BackUp, current, backup, source)
                })?;
            } else {
                fs::rename(current, backup).map_err(|source| {
                    Error::move_path(MoveAction::BackUp, current, backup, source)
                })?;
            }
        }
    }

    Ok(())
}

#[tracing::instrument(level = "debug", skip_all, fields(project = %project.display()))]
pub(crate) fn prepare(project: &Path) -> Result<(), Error> {
    let paths = TransactionPaths::new(project);
    ensure_consistent(project)?;
    remove_stale_backups(&paths)?;
    remove_abandoned_stages(&paths)?;
    Ok(())
}

pub(crate) fn ensure_consistent(project: &Path) -> Result<(), Error> {
    let paths = TransactionPaths::new(project);
    if path_exists(&paths.marker)? {
        return Err(Error::InterruptedTransaction(paths.marker));
    }

    Ok(())
}

#[tracing::instrument(level = "trace", skip_all, fields(project = %project.display()))]
pub(crate) fn create_stage(project: &Path) -> Result<PathBuf, Error> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    const ATTEMPTS: u32 = 16;

    let paths = TransactionPaths::new(project);
    fs::create_dir_all(&paths.stages)
        .map_err(|source| Error::io(IoAction::Create, &paths.stages, source))?;
    for _ in 0..ATTEMPTS {
        let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
        let stage = paths
            .stages
            .join(format!("stage-{}-{ordinal}", process::id()));
        match fs::create_dir(&stage) {
            Ok(()) => return Ok(stage),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(Error::io(IoAction::Create, stage, source));
            }
        }
    }

    Err(Error::StageNamesExhausted {
        directory: paths.stages,
        attempts: ATTEMPTS,
    })
}

fn remove_stale_backups(paths: &TransactionPaths) -> Result<(), Error> {
    for (_, _, backup) in paths.artifacts() {
        remove_path(backup)?;
    }

    Ok(())
}

fn restore(paths: &TransactionPaths, transaction: &Transaction, stage: &Path) -> Result<(), Error> {
    for (artifact, current, backup) in paths.artifacts() {
        if artifact == Artifact::Manifest && !transaction.changes_manifest {
            continue;
        }

        if path_exists(backup)? {
            remove_path(current)?;
            fs::rename(backup, current)
                .map_err(|source| Error::move_path(MoveAction::Restore, backup, current, source))?;
        } else if !transaction.previous.contains(&artifact) {
            remove_path(current)?;
        }
    }

    sync_directory(&paths.vendor)?;
    sync_directory(&paths.project)?;
    sync_directory(&paths.manager)?;
    remove_path(&paths.marker)?;
    sync_directory(&paths.manager)?;

    if !stage.as_os_str().is_empty() && stage.starts_with(&paths.stages) {
        remove_after_commit(stage);
    }

    tracing::debug!(marker = %paths.marker.display(), "restored dependency transaction");
    Ok(())
}

fn remove_abandoned_stages(paths: &TransactionPaths) -> Result<(), Error> {
    remove_stages_in(&paths.stages, b"stage-")?;
    Ok(())
}

fn remove_stages_in(directory: &Path, prefix: &[u8]) -> Result<(), Error> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(Error::io(IoAction::Inspect, directory, source)),
    };
    for entry in entries {
        let entry = entry.map_err(|source| Error::io(IoAction::Inspect, directory, source))?;
        let name = entry.file_name();
        if name.as_bytes().starts_with(prefix)
            && entry
                .file_type()
                .map_err(|source| Error::io(IoAction::Inspect, entry.path(), source))?
                .is_dir()
        {
            remove_path(&entry.path())?;
        }
    }

    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(Error::io(IoAction::Inspect, path, source)),
    }
}

fn remove_path(path: &Path) -> Result<(), Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(Error::io(IoAction::Inspect, path, source)),
    };

    let result = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };

    result.map_err(|source| Error::io(IoAction::Remove, path, source))
}

fn remove_after_commit(path: &Path) {
    if let Err(error) = remove_path(path) {
        tracing::warn!(path = %path.display(), %error, "could not clean dependency transaction artifact");
    }
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum Artifact {
    Packages,
    Autoload,
    Lock,
    Manifest,
    State,
}

struct Transaction {
    previous: BTreeSet<Artifact>,
    changes_manifest: bool,
}

impl Transaction {
    fn new(paths: &TransactionPaths, changes_manifest: bool) -> Result<Self, Error> {
        let mut previous = BTreeSet::new();
        for (artifact, current, _) in paths.artifacts() {
            if (artifact != Artifact::Manifest || changes_manifest) && path_exists(current)? {
                previous.insert(artifact);
            }
        }

        Ok(Self {
            previous,
            changes_manifest,
        })
    }
}

struct TransactionPaths {
    project: PathBuf,
    vendor: PathBuf,
    manager: PathBuf,
    stages: PathBuf,
    packages: PathBuf,
    packages_backup: PathBuf,
    autoload: PathBuf,
    autoload_backup: PathBuf,
    lock: PathBuf,
    lock_backup: PathBuf,
    manifest: PathBuf,
    manifest_backup: PathBuf,
    state: PathBuf,
    state_backup: PathBuf,
    marker: PathBuf,
}

impl TransactionPaths {
    fn new(project: &Path) -> Self {
        let vendor = project.join("vendor");
        let manager = vendor.join(".whim");
        Self {
            project: project.to_path_buf(),
            stages: manager.join("stages"),
            packages: vendor.join("packages"),
            packages_backup: manager.join("packages.backup"),
            autoload: vendor.join("autoload.whim"),
            autoload_backup: manager.join("autoload.backup"),
            lock: project.join("whim.lock"),
            lock_backup: manager.join("lock.backup"),
            manifest: project.join("whim.toml"),
            manifest_backup: manager.join("manifest.backup"),
            state: manager.join("state.toml"),
            state_backup: manager.join("state.backup"),
            marker: manager.join("transaction.pending"),
            vendor,
            manager,
        }
    }

    fn artifacts(&self) -> [(Artifact, &Path, &Path); 5] {
        [
            (
                Artifact::Packages,
                self.packages.as_path(),
                self.packages_backup.as_path(),
            ),
            (
                Artifact::Autoload,
                self.autoload.as_path(),
                self.autoload_backup.as_path(),
            ),
            (
                Artifact::Lock,
                self.lock.as_path(),
                self.lock_backup.as_path(),
            ),
            (
                Artifact::Manifest,
                self.manifest.as_path(),
                self.manifest_backup.as_path(),
            ),
            (
                Artifact::State,
                self.state.as_path(),
                self.state_backup.as_path(),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::AtomicU32;
    use std::sync::atomic::Ordering;

    use super::TransactionPaths;
    use super::create_stage;
    use super::prepare;

    struct TemporaryProject(PathBuf);

    impl TemporaryProject {
        fn create() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);

            let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("whim-transaction-test-{}-{ordinal}", process::id()));
            fs::create_dir(&path).expect("temporary project should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TemporaryProject {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("temporary project should be removed");
        }
    }

    #[test]
    fn stages_and_backups_stay_inside_the_manager_directory() {
        let project = TemporaryProject::create();
        let paths = TransactionPaths::new(project.path());
        let stage = create_stage(project.path()).expect("stage should be created");

        assert_eq!(Some(paths.stages.as_path()), stage.parent());
        assert!(paths.lock_backup.starts_with(&paths.manager));
        assert!(paths.manifest_backup.starts_with(&paths.manager));
        assert!(!project.path().join(".whim.lock.backup").exists());
        assert!(!project.path().join(".whim-manifest.backup").exists());
    }

    #[test]
    fn preparation_removes_owned_stages_but_preserves_project_directories() {
        let project = TemporaryProject::create();
        let current = create_stage(project.path()).expect("stage should be created");
        let legacy = project.path().join(".whim-stage-interrupted");
        fs::create_dir(&legacy).expect("legacy stage should be created");

        prepare(project.path()).expect("preparation should succeed");
        assert!(!current.exists());
        assert!(legacy.exists());
    }
}
