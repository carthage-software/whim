use std::collections::BTreeSet;

use clap::Args;

use crate::config::Configuration;
use crate::config::DependencyGroup;
use crate::error::Error;
use crate::package;
use crate::package::Project;
use crate::package::Source;

#[derive(Args)]
pub(super) struct Arguments {
    /// The HTTPS, SSH, or local file Git repository.
    source: String,

    /// The required release range.
    #[arg(long, value_name = "REQUIREMENT")]
    version: Option<String>,

    /// Add the repository as a development dependency.
    #[arg(long)]
    dev: bool,
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(source = %arguments.source, development = arguments.dev),
)]
pub(super) fn execute(arguments: Arguments, configuration: &Configuration) -> Result<(), Error> {
    let source = Source::parse(&arguments.source)?;
    let requirement = arguments
        .version
        .map(|requirement| -> Result<_, Error> {
            semver::VersionReq::parse(&requirement).map_err(|source| {
                Error::InvalidDependencyRequirement {
                    requirement: requirement.clone(),
                    source,
                }
            })?;
            Ok(requirement)
        })
        .transpose()?;
    let project = Project::open(configuration)?;
    let mut document = project.document()?;
    let runtime = document.find(DependencyGroup::Runtime, &source)?;
    let development = document.find(DependencyGroup::Development, &source)?;
    let group = if arguments.dev {
        if runtime.is_some() {
            return Err(Error::DependencyGroupConflict {
                dependency: source.to_string(),
                group: "runtime",
            });
        }
        DependencyGroup::Development
    } else {
        if development.is_some() {
            return Err(Error::DependencyGroupConflict {
                dependency: source.to_string(),
                group: "development",
            });
        }
        DependencyGroup::Runtime
    };

    let (requirement, refreshed) = match requirement {
        Some(requirement) => (requirement, BTreeSet::new()),
        None => latest_caret(&project, &source)?,
    };
    let spelling = document
        .find(group, &source)?
        .unwrap_or_else(|| source.identity().to_owned());
    document.insert(group, &spelling, requirement)?;
    project.resolve_and_commit(document, refreshed)?;
    Ok(())
}

fn latest_caret(project: &Project, source: &Source) -> Result<(String, BTreeSet<Source>), Error> {
    tracing::info!(repository = %source, "finding latest stable release");
    let actual = project
        .manifest()
        .normalized_overrides()?
        .get(source)
        .cloned()
        .unwrap_or_else(|| source.clone());
    let repository = package::Repository::open(project.cache(), &actual, true)?;
    let version = repository
        .candidates()?
        .into_iter()
        .filter(|candidate| candidate.version.pre.is_empty())
        .map(|candidate| candidate.version)
        .max()
        .ok_or_else(|| Error::NoStableRelease(source.to_string()))?;
    let requirement = if version.major != 0 || version.minor != 0 {
        format!("^{}.{}", version.major, version.minor)
    } else {
        format!("^0.0.{}", version.patch)
    };
    Ok((requirement, BTreeSet::from([actual])))
}
