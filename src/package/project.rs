mod error;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use crate::config::Configuration;
use crate::config::EditableManifest;
use crate::config::LOCK_NAME;
use crate::config::Manifest;
use crate::package::Error;
use crate::package::install;
use crate::package::lock::LockFile;
use crate::package::resolve::resolve_graph_with_refreshed;
use crate::package::source::Source;
use crate::package::transaction;
use crate::package::warn_graph_licenses;

pub(crate) use error::Error as ProjectError;

pub(crate) struct Project {
    root: PathBuf,
    manifest_path: PathBuf,
    manifest: Manifest,
    manifest_text: String,
    lockfile: PathBuf,
    cache: PathBuf,
    lock: Option<File>,
}

impl Project {
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn open(configuration: &Configuration) -> Result<Self, Error> {
        let location = ProjectLocation::new(configuration)?;
        let root = &location.root;

        let vendor = root.join("vendor");
        require_manager_directory(&vendor)?;
        let manager = vendor.join(".whim");
        require_manager_directory(&manager)?;
        let cache = manager.join("git");
        require_manager_directory(&cache)?;
        require_manager_directory(&manager.join("stages"))?;

        let lock_path = manager.join("install.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| ProjectError::OpenLock {
                path: lock_path.clone(),
                source,
            })?;

        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                tracing::info!("waiting for another package command");
                fs2::FileExt::lock_exclusive(&lock).map_err(|source| {
                    ProjectError::AcquireLock {
                        path: lock_path.clone(),
                        source,
                    }
                })?;
            }
            Err(source) => {
                return Err(ProjectError::AcquireLock {
                    path: lock_path,
                    source,
                }
                .into());
            }
        }

        transaction::prepare(root).map_err(ProjectError::from)?;
        let loaded = location.load()?;
        tracing::debug!(root = %loaded.root.display(), "opened dependency project");

        Ok(Self {
            lockfile: loaded.root.join(LOCK_NAME),
            cache,
            root: loaded.root,
            manifest_path: loaded.manifest_path,
            manifest: loaded.manifest,
            manifest_text: loaded.manifest_text,
            lock: Some(lock),
        })
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn inspect(configuration: &Configuration) -> Result<Self, Error> {
        let location = ProjectLocation::new(configuration)?;
        let lock = acquire_inspection_lock(&location.root)?;
        transaction::ensure_consistent(&location.root).map_err(ProjectError::from)?;
        let loaded = location.load()?;
        tracing::debug!(root = %loaded.root.display(), "inspected dependency project");

        Ok(Self {
            lockfile: loaded.root.join(LOCK_NAME),
            cache: loaded.root.join("vendor/.whim/git"),
            root: loaded.root,
            manifest_path: loaded.manifest_path,
            manifest: loaded.manifest,
            manifest_text: loaded.manifest_text,
            lock,
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub(crate) fn lockfile(&self) -> &Path {
        &self.lockfile
    }

    pub(crate) fn cache(&self) -> &Path {
        &self.cache
    }

    pub(crate) fn document(&self) -> Result<EditableManifest, Error> {
        let metadata = fs::symlink_metadata(&self.manifest_path).map_err(|source| {
            ProjectError::InspectManifest {
                path: self.manifest_path.clone(),
                source,
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ProjectError::SymbolicLinkManifest(self.manifest_path.clone()).into());
        }

        Ok(EditableManifest::parse(
            &self.manifest_path,
            &self.manifest_text,
        )?)
    }

    pub(crate) fn current_lock(&self, manifest: &Manifest) -> Result<Option<LockFile>, Error> {
        let Some(lock) = self.read_lock()? else {
            return Ok(None);
        };

        if lock.manifest != manifest.resolution_hash()? {
            return Err(ProjectError::StaleLock.into());
        }

        Ok(Some(lock))
    }

    pub(crate) fn read_lock(&self) -> Result<Option<LockFile>, Error> {
        let metadata = match fs::symlink_metadata(&self.lockfile) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ProjectError::InspectLock {
                    path: self.lockfile.clone(),
                    source,
                }
                .into());
            }
        };

        if !metadata.file_type().is_file() {
            return Err(ProjectError::InvalidLockPath(self.lockfile.clone()).into());
        }

        let lock = LockFile::read(&self.lockfile)?;
        Ok(Some(lock))
    }

    pub(crate) fn is_installed(&self, source: &Source) -> Result<bool, Error> {
        let path = self.root.join("vendor/packages").join(source.digest());
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
            Ok(_) => Err(ProjectError::InvalidInstalledPackagePath(path).into()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(source) => Err(ProjectError::InspectInstalledPackage { path, source }.into()),
        }
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, document, refreshed),
        fields(refreshed = refreshed.len()),
    )]
    pub(crate) fn resolve_and_commit(
        &self,
        document: EditableManifest,
        refreshed: BTreeSet<Source>,
    ) -> Result<(), Error> {
        let manifest_text = document.render();
        let manifest = Manifest::parse(&manifest_text, true)?;
        let preferred = if let Some(lock) = self.current_lock(&self.manifest)? {
            lock.preferred_versions()?
        } else {
            BTreeMap::new()
        };

        tracing::info!("resolving dependencies");
        let graph = resolve_graph_with_refreshed(&self.cache, &manifest, preferred, refreshed)?;
        warn_graph_licenses(&manifest, &graph, false)?;
        let stage = install::stage_graph(&self.root, &manifest, &graph, false)?;
        let lock = LockFile::from_graph(manifest.resolution_hash()?, &graph, &stage.checksums)?;
        let lock_text = lock.render()?;
        transaction::commit(&self.root, stage, &lock_text, Some(&manifest_text), false)?;
        tracing::info!("resolved {} repositories", graph.packages.len());
        Ok(())
    }
}

struct LoadedProject {
    root: PathBuf,
    manifest_path: PathBuf,
    manifest: Manifest,
    manifest_text: String,
}

struct ProjectLocation {
    root: PathBuf,
    manifest_path: PathBuf,
}

impl ProjectLocation {
    fn new(configuration: &Configuration) -> Result<Self, Error> {
        let loaded = configuration.manifest().map_err(ProjectError::from)?;
        let manifest_path = loaded.path().to_path_buf();
        let root = manifest_path
            .parent()
            .ok_or_else(|| ProjectError::MissingManifestParent(manifest_path.clone()))?
            .to_path_buf();

        Ok(Self {
            root,
            manifest_path,
        })
    }

    fn load(self) -> Result<LoadedProject, Error> {
        let (manifest, manifest_text) = Manifest::read(&self.manifest_path, true)?;

        Ok(LoadedProject {
            root: self.root,
            manifest_path: self.manifest_path,
            manifest,
            manifest_text,
        })
    }
}

fn require_manager_directory(path: &Path) -> Result<(), ProjectError> {
    loop {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_dir() => return Ok(()),
            Ok(_) => return Err(ProjectError::InvalidManagerPath(path.to_path_buf())),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ProjectError::InspectManager {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }

        match fs::create_dir(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(ProjectError::CreateManager {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }
}

fn acquire_inspection_lock(root: &Path) -> Result<Option<File>, ProjectError> {
    let path = root.join("vendor/.whim/install.lock");
    let lock = match File::open(&path) {
        Ok(lock) => lock,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ProjectError::OpenLock { path, source }),
    };

    match fs2::FileExt::try_lock_shared(&lock) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::WouldBlock => {
            tracing::info!("waiting for another package command");
            fs2::FileExt::lock_shared(&lock).map_err(|source| ProjectError::AcquireLock {
                path: path.clone(),
                source,
            })?;
        }
        Err(source) => return Err(ProjectError::AcquireLock { path, source }),
    }

    Ok(Some(lock))
}

impl Drop for Project {
    fn drop(&mut self) {
        if let Some(lock) = &self.lock
            && let Err(error) = fs2::FileExt::unlock(lock)
        {
            tracing::warn!(%error, "could not release dependency lock");
        }
    }
}
