use std::collections::BTreeMap;

use clap::Args;
use semver::VersionReq;

use crate::config::Configuration;
use crate::config::DependencyGroup;
use crate::config::Manifest;
use crate::error::Error;
use crate::output;
use crate::package;
use crate::package::Project;
use crate::package::Source;

#[derive(Args)]
pub(super) struct Arguments {
    /// The Git repository to test against the dependency graph.
    source: String,

    /// The release range to test.
    #[arg(long, default_value = "*", value_name = "REQUIREMENT")]
    version: String,
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(source = %arguments.source, requirement = %arguments.version),
)]
pub(super) fn execute(arguments: &Arguments, configuration: &Configuration) -> Result<(), Error> {
    let source = Source::parse(&arguments.source)?;
    VersionReq::parse(&arguments.version).map_err(|source| Error::InvalidRequestedRequirement {
        requirement: arguments.version.clone(),
        source,
    })?;
    let project = Project::open(configuration)?;
    let mut document = project.document()?;
    let runtime = document.find(DependencyGroup::Runtime, &source)?;
    let development = document.find(DependencyGroup::Development, &source)?;
    let (group, spelling) = match (runtime, development) {
        (Some(spelling), None) => (DependencyGroup::Runtime, spelling),
        (None, Some(spelling)) => (DependencyGroup::Development, spelling),
        (None, None) => (DependencyGroup::Runtime, source.identity().to_owned()),
        (Some(_), Some(_)) => return Err(Error::DuplicateDependency(source.to_string())),
    };

    document.insert(group, &spelling, arguments.version.clone())?;
    let manifest = Manifest::parse(&document.render(), true)?;
    let current = project.manifest();
    let preferred = if let Some(lock) = project.current_lock(current)? {
        lock.preferred_versions()?
    } else {
        BTreeMap::new()
    };

    match package::resolve_graph(project.cache(), &manifest, preferred) {
        Ok(graph) => {
            let selected = graph
                .packages
                .get(&source)
                .ok_or_else(|| Error::MissingLockedSource(source.to_string()))?;
            output::write(&format!(
                "{} {} can be installed; nothing blocks requirement {}\n",
                source, selected.version, arguments.version
            ))?;
        }
        Err(error) => {
            let Some(reason) = error.resolution_reason() else {
                return Err(error.into());
            };

            output::write(&format!(
                "{} cannot be installed with requirement {}:\n  {}\n",
                source, arguments.version, reason
            ))?;
        }
    }

    Ok(())
}
