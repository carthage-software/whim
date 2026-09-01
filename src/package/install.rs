mod error;
pub(super) mod selection;
pub(super) mod verification;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

pub(crate) use error::Error;

use crate::config::Manifest;
use crate::package::archive;
use crate::package::filesystem::write_file;
use crate::package::git::Repository;
use crate::package::install::selection::locked_package;
use crate::package::install::selection::selected_graph_sources;
use crate::package::install::selection::selected_locked_sources;
use crate::package::install::selection::validate_locked_package;
use crate::package::install::selection::validate_locked_roots;
use crate::package::install::verification::reuse_package;
use crate::package::loader;
use crate::package::lock::LockFile;
use crate::package::lock::LockedPackage;
use crate::package::parallel;
use crate::package::resolve::ResolvedGraph;
use crate::package::resolve::ResolvedPackage;
use crate::package::source::Source;
use crate::package::transaction;

pub(crate) struct StagedInstallation {
    pub(crate) directory: PathBuf,
    pub(crate) checksums: BTreeMap<Source, String>,
    pub(crate) installed: BTreeSet<Source>,
}

struct GraphRepository<'graph> {
    source: Source,
    packages: Vec<(&'graph Source, &'graph ResolvedPackage)>,
}

struct LockedRepository<'lock> {
    source: Source,
    packages: Vec<(&'lock Source, &'lock LockedPackage)>,
}

struct PreparedPackage {
    source: Source,
    checksum: String,
    manifest: Option<Manifest>,
}

#[tracing::instrument(
    level = "debug",
    skip(root, graph),
    fields(packages = graph.packages.len(), no_dev)
)]
pub(crate) fn stage_graph(
    project: &Path,
    root: &Manifest,
    graph: &ResolvedGraph,
    no_dev: bool,
) -> Result<StagedInstallation, Error> {
    let selected = selected_graph_sources(graph, no_dev)?;
    let repositories = graph_repositories(graph);
    let stage = transaction::create_stage(project)?;
    let packages = stage.join("packages");
    if let Err(source) = fs::create_dir(&packages) {
        remove_stage(&stage);
        return Err(Error::CreatePackageDirectory {
            path: packages,
            source,
        });
    }
    let cache = project.join("vendor/.whim/git");
    if !repositories.is_empty() {
        tracing::info!(
            packages = graph.packages.len(),
            workers = parallel::workers(repositories.len()),
            "preparing dependencies"
        );
    }
    let result = (|| {
        let prepared = parallel::try_map(
            &repositories,
            |repository| prepare_graph_repository(repository, &cache, &packages, &selected),
            Error::CreateWorkerPool,
        )?;
        let (checksums, manifests) = collect_prepared(prepared);
        let autoload = loader::generate(root, &manifests)?;
        write_file(&stage.join("autoload.whim"), autoload.as_bytes())?;
        Ok::<_, Error>(checksums)
    })();

    let checksums = result.inspect_err(|_| remove_stage(&stage))?;

    Ok(StagedInstallation {
        directory: stage,
        checksums,
        installed: selected,
    })
}

#[tracing::instrument(
    level = "debug",
    skip(root, lock),
    fields(packages = lock.packages.len(), no_dev)
)]
pub(crate) fn stage_lock(
    project: &Path,
    root: &Manifest,
    lock: &LockFile,
    no_dev: bool,
) -> Result<StagedInstallation, Error> {
    validate_locked_roots(root, lock)?;
    let overrides = root.normalized_overrides()?;
    let selected = selected_locked_sources(lock, no_dev)?;
    let repositories = locked_repositories(lock, &selected)?;
    let stage = transaction::create_stage(project)?;
    let packages = stage.join("packages");
    if let Err(source) = fs::create_dir(&packages) {
        remove_stage(&stage);
        return Err(Error::CreatePackageDirectory {
            path: packages,
            source,
        });
    }

    let cache = project.join("vendor/.whim/git");
    let installed = project.join("vendor/packages");
    if !repositories.is_empty() {
        tracing::info!(
            packages = selected.len(),
            workers = parallel::workers(repositories.len()),
            "installing locked dependencies"
        );
    }
    let result = (|| {
        let prepared = parallel::try_map(
            &repositories,
            |repository| {
                prepare_locked_repository(
                    repository, &cache, &installed, &packages, &overrides, lock,
                )
            },
            Error::CreateWorkerPool,
        )?;
        let (checksums, manifests) = collect_prepared(prepared);
        let autoload = loader::generate(root, &manifests)?;
        write_file(&stage.join("autoload.whim"), autoload.as_bytes())?;
        Ok::<_, Error>(checksums)
    })();

    let checksums = result.inspect_err(|_| remove_stage(&stage))?;

    Ok(StagedInstallation {
        directory: stage,
        checksums,
        installed: selected,
    })
}

fn graph_repositories(graph: &ResolvedGraph) -> Vec<GraphRepository<'_>> {
    let mut repositories = BTreeMap::<Source, Vec<_>>::new();
    for (source, package) in &graph.packages {
        let actual = package.resolved_source.as_ref().unwrap_or(&package.source);
        repositories
            .entry(actual.clone())
            .or_default()
            .push((source, package));
    }

    repositories
        .into_iter()
        .map(|(source, packages)| GraphRepository { source, packages })
        .collect()
}

fn locked_repositories<'lock>(
    lock: &'lock LockFile,
    selected: &'lock BTreeSet<Source>,
) -> Result<Vec<LockedRepository<'lock>>, Error> {
    let mut repositories = BTreeMap::<Source, Vec<_>>::new();
    for source in selected {
        let package = locked_package(lock, source)?;
        let actual = match &package.resolved_source {
            Some(source) => Source::parse(source)?,
            None => source.clone(),
        };
        repositories
            .entry(actual)
            .or_default()
            .push((source, package));
    }

    Ok(repositories
        .into_iter()
        .map(|(source, packages)| LockedRepository { source, packages })
        .collect())
}

fn prepare_graph_repository(
    job: &GraphRepository<'_>,
    cache: &Path,
    packages: &Path,
    selected: &BTreeSet<Source>,
) -> Result<Vec<PreparedPackage>, Error> {
    let repository = Repository::open(cache, &job.source, false)?;
    let mut prepared = Vec::with_capacity(job.packages.len());
    for (source, package) in &job.packages {
        tracing::info!(package = %source, version = %package.version, "preparing");
        let destination = packages.join(source.digest());
        let checksum = archive::export(&repository, &package.commit, &destination)?;
        let (manifest, _) = Manifest::read(&destination.join("whim.toml"), false)?;
        if manifest.consumed_resolution_hash()? != package.manifest.consumed_resolution_hash()? {
            return Err(Error::ExportedManifestMismatch(source.to_string()));
        }

        let manifest = if selected.contains(*source) {
            Some(manifest)
        } else {
            fs::remove_dir_all(&destination).map_err(|error| Error::RemoveDevelopmentPackage {
                package: source.to_string(),
                path: destination,
                source: error,
            })?;
            None
        };
        prepared.push(PreparedPackage {
            source: (*source).clone(),
            checksum,
            manifest,
        });
    }

    Ok(prepared)
}

fn prepare_locked_repository(
    job: &LockedRepository<'_>,
    cache: &Path,
    installed: &Path,
    packages: &Path,
    overrides: &BTreeMap<Source, Source>,
    lock: &LockFile,
) -> Result<Vec<PreparedPackage>, Error> {
    let mut repository = None;
    let mut prepared = Vec::with_capacity(job.packages.len());
    for (source, package) in &job.packages {
        tracing::info!(package = %source, version = %package.version, "installing");
        let destination = packages.join(source.digest());
        let existing = installed.join(source.digest());
        let checksum =
            if let Some(checksum) = reuse_package(source, package, &existing, &destination)? {
                checksum
            } else {
                let repository = open_repository(&mut repository, cache, &job.source)?;
                repository.fetch_commit(&package.commit)?;
                let tree = repository.rev_parse(&format!("{}^{{tree}}", package.commit))?;
                if tree != package.tree {
                    return Err(Error::TreeMismatch {
                        package: source.to_string(),
                        expected: package.tree.clone(),
                        actual: tree,
                    });
                }

                let checksum = archive::export(repository, &package.commit, &destination)?;
                if checksum != package.checksum {
                    return Err(Error::ChecksumMismatch {
                        package: source.to_string(),
                        expected: package.checksum.clone(),
                        actual: checksum,
                    });
                }
                checksum
            };

        let manifest_path = destination.join("whim.toml");
        let (manifest, _) = Manifest::read(&manifest_path, false)?;
        manifest.check_current_whim(&format!("`{source}` {}", package.version))?;
        if manifest.consumed_resolution_hash()? != package.manifest {
            return Err(Error::ManifestDigestMismatch(source.to_string()));
        }

        validate_locked_package(overrides, lock, source, package, &manifest)?;
        prepared.push(PreparedPackage {
            source: (*source).clone(),
            checksum,
            manifest: Some(manifest),
        });
    }

    Ok(prepared)
}

fn open_repository<'repository>(
    slot: &'repository mut Option<Repository>,
    cache: &Path,
    source: &Source,
) -> Result<&'repository Repository, Error> {
    match slot {
        Some(repository) => Ok(repository),
        missing @ None => Ok(missing.insert(Repository::open(cache, source, false)?)),
    }
}

fn collect_prepared(
    groups: Vec<Vec<PreparedPackage>>,
) -> (BTreeMap<Source, String>, BTreeMap<Source, Manifest>) {
    let mut checksums = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for package in groups.into_iter().flatten() {
        checksums.insert(package.source.clone(), package.checksum);
        if let Some(manifest) = package.manifest {
            manifests.insert(package.source, manifest);
        }
    }

    (checksums, manifests)
}

fn remove_stage(stage: &Path) {
    if let Err(error) = fs::remove_dir_all(stage)
        && error.kind() != ErrorKind::NotFound
    {
        tracing::warn!(path = %stage.display(), %error, "could not remove failed installation stage");
    }
}
