use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::config::Manifest;
use crate::package::install::Error;
use crate::package::lock::LockFile;
use crate::package::lock::LockedPackage;
use crate::package::lock::LockedSuggestion;
use crate::package::resolve::ResolvedGraph;
use crate::package::source::Source;

pub(in crate::package) fn selected_graph_sources(
    graph: &ResolvedGraph,
    no_dev: bool,
) -> Result<BTreeSet<Source>, Error> {
    let roots = if no_dev {
        graph.runtime.as_slice()
    } else {
        return Ok(graph.packages.keys().cloned().collect());
    };

    graph_closure(roots, |source| {
        graph
            .packages
            .get(source)
            .map(|package| package.dependencies.clone())
            .ok_or_else(|| Error::MissingResolvedPackage(source.to_string()))
    })
}

pub(in crate::package) fn selected_locked_sources(
    lock: &LockFile,
    no_dev: bool,
) -> Result<BTreeSet<Source>, Error> {
    let roots = lock
        .root
        .runtime
        .iter()
        .chain(
            (!no_dev)
                .then_some(lock.root.development.iter())
                .into_iter()
                .flatten(),
        )
        .map(|source| Source::parse(source).map_err(Error::from))
        .collect::<Result<Vec<_>, Error>>()?;

    graph_closure(&roots, |source| {
        let package = locked_package(lock, source)?;
        package
            .dependencies
            .iter()
            .map(|dependency| Source::parse(dependency).map_err(Error::from))
            .collect::<Result<Vec<_>, Error>>()
    })
}

fn graph_closure<I, F>(roots: &[Source], mut dependencies: F) -> Result<BTreeSet<Source>, Error>
where
    I: IntoIterator<Item = Source>,
    F: FnMut(&Source) -> Result<I, Error>,
{
    let mut selected = BTreeSet::new();
    let mut pending = roots.to_vec();
    while let Some(source) = pending.pop() {
        if !selected.insert(source.clone()) {
            continue;
        }

        pending.extend(dependencies(&source)?);
    }

    Ok(selected)
}

pub(in crate::package) fn locked_package<'a>(
    lock: &'a LockFile,
    source: &Source,
) -> Result<&'a LockedPackage, Error> {
    lock.packages
        .binary_search_by(|package| package.source.as_str().cmp(source.identity()))
        .ok()
        .and_then(|index| lock.packages.get(index))
        .ok_or_else(|| Error::MissingLockedPackage(source.to_string()))
}

pub(super) fn validate_locked_roots(root: &Manifest, lock: &LockFile) -> Result<(), Error> {
    let runtime = root
        .runtime_requirements()?
        .into_iter()
        .map(|requirement| requirement.source.identity().to_owned())
        .collect::<Vec<_>>();
    let development = root
        .development_requirements()?
        .into_iter()
        .map(|requirement| requirement.source.identity().to_owned())
        .collect::<Vec<_>>();
    if runtime != lock.root.runtime || development != lock.root.development {
        return Err(Error::RootMismatch);
    }

    for conflict in root.conflict_requirements()? {
        let Ok(package) = locked_package(lock, &conflict.source) else {
            continue;
        };

        if conflict.requirement.matches(&package.version) {
            return Err(Error::RootConflict {
                package: package.source.clone(),
                version: Box::new(package.version.clone()),
                requirement: Box::new(conflict.requirement),
            });
        }
    }

    Ok(())
}

pub(super) fn validate_locked_package(
    overrides: &BTreeMap<Source, Source>,
    lock: &LockFile,
    source: &Source,
    package: &LockedPackage,
    manifest: &Manifest,
) -> Result<(), Error> {
    let expected_source = overrides.get(source).cloned();
    let locked_source = package
        .resolved_source
        .as_deref()
        .map(Source::parse)
        .transpose()?;
    if expected_source != locked_source {
        return Err(Error::OverrideMismatch(source.to_string()));
    }

    let requirements = manifest.runtime_requirements()?;
    let dependencies = requirements
        .iter()
        .map(|requirement| requirement.source.identity().to_owned())
        .collect::<Vec<_>>();
    if dependencies != package.dependencies {
        return Err(Error::DependencyEdgesMismatch(source.to_string()));
    }

    for requirement in requirements {
        let dependency = locked_package(lock, &requirement.source)?;
        if !requirement.requirement.matches(&dependency.version) {
            return Err(Error::RequirementMismatch {
                owner: source.to_string(),
                dependency: requirement.source.to_string(),
                version: Box::new(dependency.version.clone()),
                requirement: Box::new(requirement.requirement),
            });
        }
    }

    for conflict in manifest.conflict_requirements()? {
        let Ok(target) = locked_package(lock, &conflict.source) else {
            continue;
        };

        if conflict.requirement.matches(&target.version) {
            return Err(Error::PackageConflict {
                owner: source.to_string(),
                owner_version: Box::new(package.version.clone()),
                target: target.source.clone(),
                target_version: Box::new(target.version.clone()),
                requirement: Box::new(conflict.requirement),
            });
        }
    }

    let suggestions = manifest
        .suggestion_requirements()?
        .into_iter()
        .map(|suggestion| LockedSuggestion {
            source: suggestion.source.identity().to_owned(),
            version: suggestion.requirement.to_string(),
        })
        .collect::<Vec<_>>();

    if package.license != manifest.package.license
        || package.sponsor != manifest.package.sponsor
        || package.suggestions != suggestions
    {
        return Err(Error::MetadataMismatch(source.to_string()));
    }

    Ok(())
}
