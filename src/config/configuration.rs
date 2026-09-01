use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use whim_runtime::engine::EngineConfiguration;

use crate::config::Error;
use crate::config::FormatConfiguration;
use crate::config::MANIFEST_NAME;
use crate::config::Manifest;
use crate::config::runtime::apply_environment;

pub(crate) struct Configuration {
    manifest: Option<LoadedManifest>,
    search_root: PathBuf,
    format: FormatConfiguration,
    runtime: EngineConfiguration,
}

pub(crate) struct LoadedManifest {
    path: PathBuf,
    manifest: Manifest,
}

impl Configuration {
    #[tracing::instrument(level = "debug", skip_all, fields(explicit = path.is_some()))]
    pub(crate) fn load(path: Option<&Path>) -> Result<Self, Error> {
        let current = env::current_dir().map_err(Error::CurrentDirectory)?;
        let search_root = current
            .canonicalize()
            .map_err(|source| Error::ResolveSearchPath {
                path: current,
                source,
            })?;
        let path = match path {
            Some(path) => Some(require_file(path, &search_root)?),
            None => find_manifest(&search_root)?,
        };

        let manifest = path
            .map(|path| {
                let (manifest, _) = Manifest::read(&path, true)?;
                Ok(LoadedManifest { path, manifest })
            })
            .transpose()?;
        let runtime = manifest
            .as_ref()
            .map_or_else(EngineConfiguration::default, |loaded| {
                loaded.manifest.runtime.engine_configuration()
            });
        let format = manifest
            .as_ref()
            .map_or_else(FormatConfiguration::default, |loaded| {
                loaded.manifest.format.clone()
            });

        if let Some(loaded) = &manifest {
            tracing::trace!(path = %loaded.path.display(), "loaded configuration");
        } else {
            tracing::trace!("using default configuration");
        }

        Ok(Self {
            manifest,
            search_root,
            format,
            runtime,
        })
    }

    pub(crate) fn runtime(&self) -> Result<EngineConfiguration, Error> {
        apply_environment(self.runtime)
    }

    pub(crate) const fn format(&self) -> &FormatConfiguration {
        &self.format
    }

    pub(crate) fn root(&self) -> &Path {
        self.manifest
            .as_ref()
            .map_or(self.search_root.as_path(), |loaded| {
                loaded.path.parent().unwrap_or(self.search_root.as_path())
            })
    }

    pub(crate) fn project_root(&self) -> Result<&Path, Error> {
        self.manifest()?;
        Ok(self.root())
    }

    pub(crate) fn manifest(&self) -> Result<&LoadedManifest, Error> {
        self.manifest
            .as_ref()
            .ok_or_else(|| Error::ManifestNotFound(self.search_root.clone()))
    }
}

impl LoadedManifest {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

fn find_manifest(start: &Path) -> Result<Option<PathBuf>, Error> {
    for directory in start.ancestors() {
        let candidate = directory.join(MANIFEST_NAME);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_file() => return Ok(Some(candidate)),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::metadata(&candidate).map_err(|source| Error::InspectManifest {
                    path: candidate.clone(),
                    source,
                })?;

                if target.is_file() {
                    return Ok(Some(candidate));
                }

                return Err(Error::InvalidManifestPath(candidate));
            }
            Ok(_) => return Err(Error::InvalidManifestPath(candidate)),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::InspectManifest {
                    path: candidate,
                    source,
                });
            }
        }
    }

    Ok(None)
}

fn require_file(path: &Path, search_root: &Path) -> Result<PathBuf, Error> {
    let requested = path.to_path_buf();
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        search_root.join(path)
    };
    let Some(name) = path.file_name().map(ToOwned::to_owned) else {
        return Err(Error::InvalidManifestPath(path));
    };
    let parent = path.parent().unwrap_or(search_root);
    let parent = parent
        .canonicalize()
        .map_err(|source| match source.kind() {
            ErrorKind::NotFound => Error::ConfigurationNotFound(requested.clone()),
            _ => Error::ResolveConfigurationPath {
                path: requested.clone(),
                source,
            },
        })?;
    let path = parent.join(name);
    let metadata = fs::metadata(&path).map_err(|source| match source.kind() {
        ErrorKind::NotFound => Error::ConfigurationNotFound(requested),
        _ => Error::InspectManifest {
            path: path.clone(),
            source,
        },
    })?;
    if !metadata.is_file() {
        return Err(Error::InvalidManifestPath(path));
    }

    Ok(path)
}
