mod error;
mod provider;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use pubgrub::DefaultStringReporter;
use pubgrub::PubGrubError;
use pubgrub::Reporter;
use pubgrub::resolve;
use semver::Version;
use semver::VersionReq;

use crate::config::Manifest;
use crate::package::Source;
use crate::package::resolve::provider::Package;
use crate::package::resolve::provider::Provider;
pub(crate) use error::Error;

const MAXIMUM_CONFLICT_ATTEMPTS: usize = 4_096;

#[derive(Clone, Debug)]
pub(crate) struct ResolvedPackage {
    pub(crate) source: Source,
    pub(crate) resolved_source: Option<Source>,
    pub(crate) version: Version,
    pub(crate) tag: String,
    pub(crate) commit: String,
    pub(crate) tree: String,
    pub(crate) manifest: Manifest,
    pub(crate) dependencies: Vec<Source>,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedGraph {
    pub(crate) runtime: Vec<Source>,
    pub(crate) development: Vec<Source>,
    pub(crate) packages: BTreeMap<Source, ResolvedPackage>,
}

pub(crate) fn resolve_graph(
    cache: &Path,
    root: &Manifest,
    preferred: BTreeMap<Source, Version>,
) -> Result<ResolvedGraph, Error> {
    resolve_graph_with_refreshed(cache, root, preferred, BTreeSet::new())
}

#[tracing::instrument(
    level = "debug",
    skip(cache, root, preferred, refreshed),
    fields(preferred = preferred.len(), refreshed = refreshed.len()),
)]
pub(crate) fn resolve_graph_with_refreshed(
    cache: &Path,
    root: &Manifest,
    preferred: BTreeMap<Source, Version>,
    refreshed: BTreeSet<Source>,
) -> Result<ResolvedGraph, Error> {
    let runtime = root.runtime_requirements()?;
    let development = root.development_requirements()?;
    let provider = Provider::new(cache, root, preferred, refreshed)?
        .with_root_dependencies(&runtime, &development)?;
    let runtime = runtime
        .into_iter()
        .map(|requirement| requirement.source)
        .collect::<Vec<_>>();
    let development = development
        .into_iter()
        .map(|requirement| requirement.source)
        .collect::<Vec<_>>();
    let mut pending = vec![BTreeSet::new()];
    let mut visited = BTreeSet::new();
    let mut first_conflict = None;
    let mut last_failure = None;

    while let Some(banned) = pending.pop() {
        if !visited.insert(banned.clone()) {
            continue;
        }

        if visited.len() > MAXIMUM_CONFLICT_ATTEMPTS {
            return Err(Error::ConflictSearchLimit {
                limit: MAXIMUM_CONFLICT_ATTEMPTS,
            });
        }

        provider.set_banned(banned.clone());
        let solution = match resolve(&provider, Package::Root, Version::new(0, 0, 0)) {
            Ok(solution) => solution,
            Err(PubGrubError::NoSolution(mut tree)) => {
                tree.collapse_no_versions();
                last_failure = Some(DefaultStringReporter::report(&tree));
                continue;
            }
            Err(PubGrubError::ErrorRetrievingDependencies {
                package,
                version,
                source,
            }) => {
                return Err(Error::RetrieveDependencies {
                    package: package.to_string(),
                    version,
                    source: Box::new(source),
                });
            }
            Err(PubGrubError::ErrorChoosingVersion { package, source }) => {
                return Err(Error::ChooseVersion {
                    package: package.to_string(),
                    source: Box::new(source),
                });
            }
            Err(PubGrubError::ErrorInShouldCancel(source)) => {
                return Err(Error::Cancellation(Box::new(source)));
            }
        };

        let graph = graph_from_solution(&provider, &runtime, &development, solution)?;
        let Some(conflict) = selected_conflict(root, &graph)? else {
            tracing::debug!(
                packages = graph.packages.len(),
                attempts = visited.len(),
                "resolved dependency graph"
            );
            return Ok(graph);
        };

        first_conflict.get_or_insert_with(|| conflict.describe());

        if let Some((owner, version)) = &conflict.owner {
            let mut without_owner = banned.clone();
            without_owner.insert((owner.clone(), version.clone()));
            pending.push(without_owner);
        }

        let mut without_target = banned;
        without_target.insert((conflict.target.clone(), conflict.target_version));
        pending.push(without_target);
    }

    if let Some(conflict) = first_conflict {
        let suffix = last_failure
            .map(|failure| format!("; no compatible selection exists: {failure}"))
            .unwrap_or_default();
        Err(Error::NoSolution(format!("{conflict}{suffix}")))
    } else {
        Err(Error::NoSolution(last_failure.unwrap_or_else(|| {
            "no compatible dependency selection exists".to_owned()
        })))
    }
}

fn graph_from_solution(
    provider: &Provider<'_>,
    runtime: &[Source],
    development: &[Source],
    solution: impl IntoIterator<Item = (Package, Version)>,
) -> Result<ResolvedGraph, Error> {
    let mut packages = BTreeMap::new();
    for (package, version) in solution {
        let Package::Dependency(source) = package else {
            continue;
        };

        let candidate = provider.load_candidate(&source, &version)?;
        let manifest = candidate.manifest.ok_or_else(|| Error::MissingManifest {
            package: source.to_string(),
            version: version.clone(),
        })?;

        let dependencies = manifest
            .runtime_requirements()?
            .into_iter()
            .map(|requirement| requirement.source)
            .collect();
        let actual = provider.actual_source(&source);
        packages.insert(
            source.clone(),
            ResolvedPackage {
                source: source.clone(),
                resolved_source: (actual != source).then_some(actual),
                version,
                tag: candidate.git.tag,
                commit: candidate.git.commit,
                tree: candidate.git.tree,
                manifest,
                dependencies,
            },
        );
    }

    reject_cycles(&packages)?;
    Ok(ResolvedGraph {
        runtime: runtime.to_vec(),
        development: development.to_vec(),
        packages,
    })
}

struct SelectedConflict {
    owner: Option<(Source, Version)>,
    target: Source,
    target_version: Version,
    requirement: VersionReq,
}

impl SelectedConflict {
    fn describe(&self) -> String {
        match &self.owner {
            Some((owner, version)) => format!(
                "`{owner}` {version} conflicts with `{}` {} through requirement {}",
                self.target, self.target_version, self.requirement
            ),
            None => format!(
                "the root project conflicts with `{}` {} through requirement {}",
                self.target, self.target_version, self.requirement
            ),
        }
    }
}

fn selected_conflict(
    root: &Manifest,
    graph: &ResolvedGraph,
) -> Result<Option<SelectedConflict>, Error> {
    if let Some(conflict) = matching_conflict(None, root, graph)? {
        return Ok(Some(conflict));
    }

    for package in graph.packages.values() {
        if let Some(conflict) = matching_conflict(
            Some((&package.source, &package.version)),
            &package.manifest,
            graph,
        )? {
            return Ok(Some(conflict));
        }
    }

    Ok(None)
}

fn matching_conflict(
    owner: Option<(&Source, &Version)>,
    manifest: &Manifest,
    graph: &ResolvedGraph,
) -> Result<Option<SelectedConflict>, Error> {
    for conflict in manifest.conflict_requirements()? {
        let Some(target) = graph.packages.get(&conflict.source) else {
            continue;
        };

        if conflict.requirement.matches(&target.version) {
            return Ok(Some(SelectedConflict {
                owner: owner.map(|(source, version)| (source.clone(), version.clone())),
                target: target.source.clone(),
                target_version: target.version.clone(),
                requirement: conflict.requirement,
            }));
        }
    }

    Ok(None)
}

fn reject_cycles(packages: &BTreeMap<Source, ResolvedPackage>) -> Result<(), Error> {
    fn visit(
        source: &Source,
        packages: &BTreeMap<Source, ResolvedPackage>,
        visiting: &mut BTreeSet<Source>,
        visited: &mut BTreeSet<Source>,
    ) -> Result<(), Error> {
        if visited.contains(source) {
            return Ok(());
        }

        if !visiting.insert(source.clone()) {
            return Err(Error::Cycle(source.to_string()));
        }

        if let Some(package) = packages.get(source) {
            for dependency in &package.dependencies {
                visit(dependency, packages, visiting, visited)?;
            }
        }

        visiting.remove(source);
        visited.insert(source.clone());
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for source in packages.keys() {
        visit(source, packages, &mut visiting, &mut visited)?;
    }

    Ok(())
}
