use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use pubgrub::Dependencies;
use pubgrub::DependencyConstraints;
use pubgrub::DependencyProvider;
use pubgrub::PackageResolutionStatistics;
use pubgrub::Ranges;
use semver::Version;
use semver::VersionReq;

use crate::config::DependencyRequirement;
use crate::config::Error as ConfigurationError;
use crate::config::Manifest;
use crate::package::git::GitCandidate;
use crate::package::git::Repository;
use crate::package::parallel;
use crate::package::resolve::Error;
use crate::package::source::Source;

const MAXIMUM_SOURCES: usize = 8_192;

type VersionSet = Ranges<Version>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum Package {
    Root,
    Dependency(Source),
}

impl fmt::Display for Package {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => formatter.write_str("the root project"),
            Self::Dependency(source) => source.fmt(formatter),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub(super) git: GitCandidate,
    pub(super) manifest: Option<Manifest>,
    dependencies: Option<Vec<(Package, VersionSet)>>,
}

struct CatalogRequest {
    source: Source,
    packages: Vec<Source>,
    refresh: bool,
}

struct LoadedCatalog {
    packages: Vec<Source>,
    repository: Repository,
    candidates: BTreeMap<Version, Candidate>,
}

pub(super) struct Provider<'a> {
    cache: &'a Path,
    root_dependencies: Vec<(Package, VersionSet)>,
    overrides: BTreeMap<Source, Source>,
    preferred: BTreeMap<Source, Version>,
    repositories: RefCell<BTreeMap<Source, Repository>>,
    candidates: RefCell<BTreeMap<Source, BTreeMap<Version, Candidate>>>,
    banned: RefCell<BTreeSet<(Source, Version)>>,
    refreshed: RefCell<BTreeSet<Source>>,
    current_whim: Version,
}

impl<'a> Provider<'a> {
    pub(super) fn new(
        cache: &'a Path,
        root: &Manifest,
        preferred: BTreeMap<Source, Version>,
        refreshed: BTreeSet<Source>,
    ) -> Result<Self, Error> {
        let overrides = root.normalized_overrides()?;
        let current_whim =
            Version::parse(env!("CARGO_PKG_VERSION")).map_err(Error::InvalidCurrentVersion)?;
        if let Some(requirement) = &root.requirements.whim {
            let requirement = VersionReq::parse(requirement).map_err(|source| {
                ConfigurationError::InvalidWhimRequirement {
                    requirement: requirement.clone(),
                    source,
                }
            })?;

            if !requirement.matches(&current_whim) {
                return Err(ConfigurationError::IncompatibleWhim {
                    owner: "the root project".to_owned(),
                    requirement,
                    current: current_whim,
                }
                .into());
            }
        }

        let provider = Self {
            cache,
            root_dependencies: Vec::new(),
            overrides,
            preferred,
            repositories: RefCell::new(BTreeMap::new()),
            candidates: RefCell::new(BTreeMap::new()),
            banned: RefCell::new(BTreeSet::new()),
            refreshed: RefCell::new(refreshed),
            current_whim,
        };

        Ok(provider)
    }

    pub(super) fn with_root_dependencies(
        mut self,
        runtime: &[DependencyRequirement],
        development: &[DependencyRequirement],
    ) -> Result<Self, Error> {
        self.preload_sources(
            runtime
                .iter()
                .chain(development)
                .map(|requirement| &requirement.source),
        )?;

        let mut dependencies = Vec::with_capacity(runtime.len() + development.len());
        for requirement in runtime.iter().chain(development) {
            let range = self.range_for(&requirement.source, &requirement.requirement)?;
            dependencies.push((Package::Dependency(requirement.source.clone()), range));
        }

        self.root_dependencies = dependencies;
        Ok(self)
    }

    pub(super) fn actual_source(&self, source: &Source) -> Source {
        self.overrides
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.clone())
    }

    pub(super) fn set_banned(&self, banned: BTreeSet<(Source, Version)>) {
        *self.banned.borrow_mut() = banned;
    }

    fn is_banned(&self, source: &Source, version: &Version) -> bool {
        self.banned
            .borrow()
            .contains(&(source.clone(), version.clone()))
    }

    fn ensure_candidates(&self, source: &Source) -> Result<(), Error> {
        if self.candidates.borrow().contains_key(source) {
            return Ok(());
        }

        self.preload_sources([source])?;
        if self.candidates.borrow().contains_key(source) {
            Ok(())
        } else {
            Err(Error::MissingCandidateCatalog(source.to_string()))
        }
    }

    fn preload_sources<'source>(
        &self,
        sources: impl IntoIterator<Item = &'source Source>,
    ) -> Result<(), Error> {
        let candidates = self.candidates.borrow();
        let mut grouped = BTreeMap::<Source, BTreeSet<Source>>::new();
        for source in sources {
            if candidates.contains_key(source) {
                continue;
            }

            grouped
                .entry(self.actual_source(source))
                .or_default()
                .insert(source.clone());
        }

        let new_packages = grouped.values().map(BTreeSet::len).sum::<usize>();
        if candidates.len().saturating_add(new_packages) > MAXIMUM_SOURCES {
            return Err(Error::GraphTooLarge {
                limit: MAXIMUM_SOURCES,
            });
        }
        drop(candidates);

        if grouped.is_empty() {
            return Ok(());
        }

        let mut refreshed = self.refreshed.borrow_mut();
        let requests = grouped
            .into_iter()
            .map(|(source, packages)| {
                let refresh = refreshed.insert(source.clone());
                CatalogRequest {
                    source,
                    packages: packages.into_iter().collect(),
                    refresh,
                }
            })
            .collect::<Vec<_>>();
        drop(refreshed);

        tracing::info!(
            repositories = requests.len(),
            workers = parallel::workers(requests.len()),
            "loading package releases"
        );
        let loaded = parallel::try_map(
            &requests,
            |request| Self::load_catalog(self.cache, request),
            Error::CreateWorkerPool,
        )?;

        let mut repositories = self.repositories.borrow_mut();
        let mut candidates = self.candidates.borrow_mut();
        for catalog in loaded {
            for package in catalog.packages {
                repositories.insert(package.clone(), catalog.repository.clone());
                candidates.insert(package, catalog.candidates.clone());
            }
        }

        Ok(())
    }

    fn load_catalog(cache: &Path, request: &CatalogRequest) -> Result<LoadedCatalog, Error> {
        if request.refresh {
            tracing::info!(repository = %request.source, "fetching releases");
        } else {
            tracing::info!(repository = %request.source, "reading cached releases");
        }

        let repository =
            Repository::open(cache, &request.source, request.refresh).map_err(|source| {
                Error::LoadReleases {
                    repository: request.source.clone(),
                    source: Box::new(source),
                }
            })?;
        let candidates = repository
            .candidates()
            .map_err(|source| Error::LoadReleases {
                repository: request.source.clone(),
                source: Box::new(source),
            })?
            .into_iter()
            .map(|git| {
                (
                    git.version.clone(),
                    Candidate {
                        git,
                        manifest: None,
                        dependencies: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        tracing::info!(
            repository = %request.source,
            versions = candidates.len(),
            "loaded releases"
        );

        Ok(LoadedCatalog {
            packages: request.packages.clone(),
            repository,
            candidates,
        })
    }

    fn range_for(&self, source: &Source, requirement: &VersionReq) -> Result<VersionSet, Error> {
        self.ensure_candidates(source)?;
        let candidates = self.candidates.borrow();
        let versions = candidates
            .get(source)
            .ok_or_else(|| Error::MissingCandidateCatalog(source.to_string()))?;
        let mut range = Ranges::empty();
        for version in versions
            .keys()
            .filter(|version| requirement.matches(version))
        {
            range = range.union(&Ranges::singleton(version.clone()));
        }

        Ok(range)
    }

    pub(super) fn load_candidate(
        &self,
        source: &Source,
        version: &Version,
    ) -> Result<Candidate, Error> {
        self.ensure_candidates(source)?;
        if let Some(candidate) = self
            .candidates
            .borrow()
            .get(source)
            .and_then(|versions| versions.get(version))
            .filter(|candidate| candidate.manifest.is_some())
            .cloned()
        {
            return Ok(candidate);
        }

        let commit = self
            .candidates
            .borrow()
            .get(source)
            .and_then(|versions| versions.get(version))
            .map(|candidate| candidate.git.commit.clone())
            .ok_or_else(|| Error::VersionUnavailable {
                package: source.to_string(),
                version: version.clone(),
            })?;

        let repository = self
            .repositories
            .borrow()
            .get(source)
            .cloned()
            .ok_or_else(|| Error::MissingRepository(source.to_string()))?;

        let text = repository.manifest(&commit)?;
        let manifest = Manifest::parse(&text, false).map_err(|error| Error::InvalidManifest {
            package: source.to_string(),
            version: version.clone(),
            error: Box::new(error),
        })?;

        tracing::trace!(package = %source, %version, "loaded package manifest");

        let requirements = manifest.runtime_requirements()?;
        for requirement in &requirements {
            if &requirement.source == source {
                return Err(Error::SelfDependency {
                    package: source.to_string(),
                    version: version.clone(),
                });
            }
        }

        self.preload_sources(requirements.iter().map(|requirement| &requirement.source))?;
        let mut dependencies = Vec::with_capacity(requirements.len());
        for requirement in requirements {
            let range = self.range_for(&requirement.source, &requirement.requirement)?;
            dependencies.push((Package::Dependency(requirement.source), range));
        }

        let mut catalogs = self.candidates.borrow_mut();
        let candidate = catalogs
            .get_mut(source)
            .and_then(|versions| versions.get_mut(version))
            .ok_or_else(|| Error::VersionDisappeared {
                package: source.to_string(),
                version: version.clone(),
            })?;

        candidate.manifest = Some(manifest);
        candidate.dependencies = Some(dependencies);
        Ok(candidate.clone())
    }

    fn candidate_count(&self, source: &Source, range: &VersionSet) -> usize {
        if self.ensure_candidates(source).is_err() {
            return 0;
        }

        self.candidates.borrow().get(source).map_or(0, |versions| {
            versions
                .keys()
                .filter(|version| range.contains(version) && !self.is_banned(source, version))
                .count()
        })
    }
}

impl DependencyProvider for Provider<'_> {
    type P = Package;
    type V = Version;
    type VS = VersionSet;
    type Priority = (u32, Reverse<usize>);
    type M = String;
    type Err = Error;

    fn choose_version(
        &self,
        package: &Self::P,
        range: &Self::VS,
    ) -> Result<Option<Self::V>, Self::Err> {
        match package {
            Package::Root => Ok(Some(Version::new(0, 0, 0))),
            Package::Dependency(source) => {
                self.ensure_candidates(source)?;
                if let Some(preferred) = self.preferred.get(source)
                    && range.contains(preferred)
                    && !self.is_banned(source, preferred)
                    && self
                        .candidates
                        .borrow()
                        .get(source)
                        .is_some_and(|versions| versions.contains_key(preferred))
                {
                    return Ok(Some(preferred.clone()));
                }

                Ok(self.candidates.borrow().get(source).and_then(|versions| {
                    versions
                        .keys()
                        .rev()
                        .find(|version| range.contains(version) && !self.is_banned(source, version))
                        .cloned()
                }))
            }
        }
    }

    fn prioritize(
        &self,
        package: &Self::P,
        range: &Self::VS,
        statistics: &PackageResolutionStatistics,
    ) -> Self::Priority {
        let count = match package {
            Package::Root => 1,
            Package::Dependency(source) => self.candidate_count(source, range),
        };

        (statistics.conflict_count(), Reverse(count))
    }

    fn get_dependencies(
        &self,
        package: &Self::P,
        version: &Self::V,
    ) -> Result<Dependencies<Self::P, Self::VS, Self::M>, Self::Err> {
        let source = match package {
            Package::Root => {
                return Ok(Dependencies::Available(
                    self.root_dependencies.clone().into_iter().collect(),
                ));
            }
            Package::Dependency(source) => source,
        };

        let candidate = self.load_candidate(source, version)?;
        let manifest = candidate
            .manifest
            .as_ref()
            .ok_or(Error::MissingParsedManifest)?;

        if let Some(requirement) = &manifest.requirements.whim {
            let requirement = VersionReq::parse(requirement).map_err(|source| {
                ConfigurationError::InvalidWhimRequirement {
                    requirement: requirement.clone(),
                    source,
                }
            })?;

            if !requirement.matches(&self.current_whim) {
                return Ok(Dependencies::Unavailable(format!(
                    "it requires Whim {requirement}, but this is Whim {}",
                    self.current_whim
                )));
            }
        }

        let dependencies = candidate.dependencies.ok_or(Error::MissingDependencySet)?;
        Ok(Dependencies::Available(
            dependencies
                .into_iter()
                .collect::<DependencyConstraints<_, _>>(),
        ))
    }
}
