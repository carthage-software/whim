mod archive;
mod error;
mod filesystem;
mod git;
mod install;
mod license;
mod loader;
mod lock;
mod parallel;
mod project;
mod resolve;
mod source;
mod state;
mod transaction;

use std::collections::BTreeMap;
use std::path::Path;

use semver::Version;

use crate::config::Manifest;

pub(crate) use error::Error;
pub(crate) use git::Error as GitError;
pub(crate) use git::Repository;
pub(crate) use install::StagedInstallation;
pub(crate) use license::warn_graph as warn_graph_licenses;
pub(crate) use license::warn_lock as warn_lock_licenses;
pub(crate) use lock::Error as LockError;
pub(crate) use lock::LockFile;
pub(crate) use lock::LockedPackage;
pub(crate) use project::Project;
pub(crate) use resolve::ResolvedGraph;
pub(crate) use source::Error as SourceError;
pub(crate) use source::Source;

pub(crate) fn resolve_graph(
    cache: &Path,
    root: &Manifest,
    preferred: BTreeMap<Source, Version>,
) -> Result<ResolvedGraph, Error> {
    resolve::resolve_graph(cache, root, preferred).map_err(Error::from)
}

pub(crate) fn installation_is_current(
    project: &Path,
    lock: &LockFile,
    no_dev: bool,
) -> Result<bool, Error> {
    install::verification::installation_is_current(project, lock, no_dev).map_err(Error::from)
}

pub(crate) fn stage_graph(
    project: &Path,
    root: &Manifest,
    graph: &ResolvedGraph,
    no_dev: bool,
) -> Result<StagedInstallation, Error> {
    install::stage_graph(project, root, graph, no_dev).map_err(Error::from)
}

pub(crate) fn stage_lock(
    project: &Path,
    root: &Manifest,
    lock: &LockFile,
    no_dev: bool,
) -> Result<StagedInstallation, Error> {
    install::stage_lock(project, root, lock, no_dev).map_err(Error::from)
}

pub(crate) fn commit(
    project: &Path,
    stage: StagedInstallation,
    lock_text: &str,
    manifest_text: Option<&str>,
    no_dev: bool,
) -> Result<(), Error> {
    transaction::commit(project, stage, lock_text, manifest_text, no_dev).map_err(Error::from)
}
