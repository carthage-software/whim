use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error as ThisError;
use toml::ser::Error as TomlEncodingError;

use crate::filesystem;
use crate::filesystem::LimitedString;
use crate::package::filesystem::Error as FilesystemError;
use crate::package::filesystem::hash_file;
use crate::package::filesystem::write_file;
use crate::package::source::Source;

const MAXIMUM_STATE_BYTES: u64 = 1_048_576;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("could not read installation state `{}`: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("missing checksum for installed package `{0}`")]
    MissingChecksum(String),
    #[error("dependency manager directory `{}` has no parent", .0.display())]
    MissingVendorParent(PathBuf),
    #[error("could not encode installation state: {0}")]
    Encode(#[source] TomlEncodingError),
    #[error(transparent)]
    Filesystem(#[from] FilesystemError),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    pub(super) version: u32,
    pub(super) lock: String,
    mode: InstallationMode,
    pub(super) autoload: String,
    pub(super) packages: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum InstallationMode {
    All,
    NoDev,
}

impl State {
    pub(super) fn matches_mode(&self, no_dev: bool) -> bool {
        self.mode
            == if no_dev {
                InstallationMode::NoDev
            } else {
                InstallationMode::All
            }
    }
}

#[tracing::instrument(level = "trace", skip_all, fields(project = %project.display()))]
pub(super) fn read(project: &Path) -> Result<Option<State>, Error> {
    let path = project.join("vendor/.whim/state.toml");
    let text = match filesystem::read_limited_string(&path, MAXIMUM_STATE_BYTES) {
        Ok(LimitedString::Contents(text)) => text,
        Ok(LimitedString::TooLarge) => {
            tracing::debug!(path = %path.display(), "ignoring oversized installation state");
            return Ok(None);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Read { path, source });
        }
    };

    match toml::from_str(&text) {
        Ok(state) => Ok(Some(state)),
        Err(error) => {
            tracing::debug!(path = %path.display(), ?error, "ignoring invalid installation state");
            Ok(None)
        }
    }
}

#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(manager = %manager.display(), no_dev, packages = installed.len()),
)]
pub(crate) fn write(
    manager: &Path,
    lock_text: &str,
    no_dev: bool,
    installed: &BTreeSet<Source>,
    checksums: &BTreeMap<Source, String>,
) -> Result<(), Error> {
    let packages = installed
        .iter()
        .map(|source| {
            checksums
                .get(source)
                .cloned()
                .map(|checksum| (source.identity().to_owned(), checksum))
                .ok_or_else(|| Error::MissingChecksum(source.to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let vendor = manager
        .parent()
        .ok_or_else(|| Error::MissingVendorParent(manager.to_path_buf()))?;
    let autoload_path = vendor.join("autoload.whim");
    let state = State {
        version: 1,
        lock: format!("blake3:{}", blake3::hash(lock_text.as_bytes()).to_hex()),
        mode: if no_dev {
            InstallationMode::NoDev
        } else {
            InstallationMode::All
        },
        autoload: hash_file(&autoload_path)?,
        packages,
    };

    write_file(
        &manager.join("state.toml"),
        toml::to_string(&state).map_err(Error::Encode)?.as_bytes(),
    )?;

    Ok(())
}
