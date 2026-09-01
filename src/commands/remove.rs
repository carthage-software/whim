use std::collections::BTreeSet;

use clap::Args;

use crate::config::Configuration;
use crate::config::DependencyGroup;
use crate::error::Error;
use crate::package::Project;
use crate::package::Source;

#[derive(Args)]
pub(super) struct Arguments {
    /// The HTTPS, SSH, or local file Git repository.
    source: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(source = %arguments.source))]
pub(super) fn execute(arguments: &Arguments, configuration: &Configuration) -> Result<(), Error> {
    let source = Source::parse(&arguments.source)?;
    let project = Project::open(configuration)?;
    let mut document = project.document()?;
    let runtime = document.find(DependencyGroup::Runtime, &source)?;
    let development = document.find(DependencyGroup::Development, &source)?;
    let (group, spelling) = match (runtime, development) {
        (Some(spelling), None) => (DependencyGroup::Runtime, spelling),
        (None, Some(spelling)) => (DependencyGroup::Development, spelling),
        (None, None) => {
            return Err(Error::NotDirectDependency(source.to_string()));
        }
        (Some(_), Some(_)) => {
            return Err(Error::DuplicateDependency(source.to_string()));
        }
    };

    document.remove(group, &spelling)?;
    project.resolve_and_commit(document, BTreeSet::default())?;
    Ok(())
}
