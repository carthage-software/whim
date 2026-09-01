use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use clap::Args;

use crate::config::Configuration;
use crate::error::Error;
use crate::package;
use crate::package::LockFile;
use crate::package::Project;
use crate::package::Repository;
use crate::package::ResolvedGraph;
use crate::package::Source;

#[derive(Args)]
pub(super) struct Arguments {
    /// Logical Git sources to update. Omit to update everything.
    #[arg(value_name = "SOURCE")]
    sources: Vec<String>,
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(sources = arguments.sources.len()),
)]
pub(super) fn execute(arguments: &Arguments, configuration: &Configuration) -> Result<(), Error> {
    let selected = arguments
        .sources
        .iter()
        .map(|source| Source::parse(source))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let project = Project::open(configuration)?;
    let manifest = project.manifest();
    let old_lock = project.read_lock()?;

    if !selected.is_empty() {
        let Some(lock) = &old_lock else {
            return Err(Error::TargetedUpdateNeedsLock);
        };

        for source in &selected {
            if !lock
                .packages
                .iter()
                .any(|package| package.source == source.identity())
            {
                return Err(Error::MissingLockedSource(source.to_string()));
            }
        }
    }

    let mut preferred = if selected.is_empty() {
        BTreeMap::new()
    } else {
        old_lock
            .as_ref()
            .ok_or(Error::TargetedUpdateNeedsLock)?
            .preferred_versions()?
    };

    for source in &selected {
        preferred.remove(source);
    }

    tracing::info!("resolving dependency updates");
    let graph = package::resolve_graph(project.cache(), manifest, preferred)?;
    if let Some(lock) = &old_lock {
        tracing::info!("checking locked release tags");
        reject_moved_tags(project.cache(), lock, &graph)?;
    }

    package::warn_graph_licenses(manifest, &graph, false)?;
    let stage = package::stage_graph(project.root(), manifest, &graph, false)?;
    let lock = LockFile::from_graph(manifest.resolution_hash()?, &graph, &stage.checksums)?;
    let lock_text = lock.render()?;
    package::commit(project.root(), stage, &lock_text, None, false)?;
    tracing::info!("updated {} repositories", graph.packages.len());
    Ok(())
}

fn reject_moved_tags(cache: &Path, lock: &LockFile, graph: &ResolvedGraph) -> Result<(), Error> {
    for locked in &lock.packages {
        let source = Source::parse(&locked.source)?;
        if !graph.packages.contains_key(&source) {
            continue;
        }

        let actual = match &locked.resolved_source {
            Some(source) => Source::parse(source)?,
            None => source,
        };

        let repository = Repository::open(cache, &actual, false)?;
        if let Some(candidate) = repository
            .candidates()?
            .into_iter()
            .find(|candidate| candidate.version == locked.version)
            && candidate.commit != locked.commit
        {
            return Err(Error::MovedTag {
                tag: locked.tag.clone(),
                repository: locked.source.clone(),
                previous: locked.commit.clone(),
                current: candidate.commit,
            });
        }
    }

    Ok(())
}
