use std::collections::BTreeMap;

use clap::Args;

use crate::config::Configuration;
use crate::error::Error;
use crate::package;
use crate::package::LockFile;
use crate::package::Project;

#[derive(Args)]
pub(super) struct Arguments {
    /// Exclude development dependencies and namespaces.
    #[arg(long)]
    no_dev: bool,
}

#[tracing::instrument(level = "debug", skip_all, fields(no_dev = arguments.no_dev))]
pub(super) fn execute(arguments: &Arguments, configuration: &Configuration) -> Result<(), Error> {
    let project = Project::open(configuration)?;
    let manifest = project.manifest();
    let manifest_hash = manifest.resolution_hash()?;
    if let Some(lock) = project.current_lock(manifest)? {
        manifest.check_current_whim("the root project")?;
        package::warn_lock_licenses(manifest, &lock, arguments.no_dev)?;
        if package::installation_is_current(project.root(), &lock, arguments.no_dev)? {
            tracing::info!("dependencies are already installed");
            return Ok(());
        }

        let stage = package::stage_lock(project.root(), manifest, &lock, arguments.no_dev)?;
        let lock_text = lock.render()?;
        package::commit(project.root(), stage, &lock_text, None, arguments.no_dev)?;
        tracing::info!("installed {} locked repositories", lock.packages.len());
        return Ok(());
    }

    manifest.check_current_whim("the root project")?;
    tracing::info!("resolving dependencies");
    let graph = package::resolve_graph(project.cache(), manifest, BTreeMap::new())?;
    package::warn_graph_licenses(manifest, &graph, arguments.no_dev)?;
    let stage = package::stage_graph(project.root(), manifest, &graph, arguments.no_dev)?;
    let lock = LockFile::from_graph(manifest_hash, &graph, &stage.checksums)?;
    let lock_text = lock.render()?;
    package::commit(project.root(), stage, &lock_text, None, arguments.no_dev)?;
    tracing::info!(
        "resolved and installed {} repositories",
        graph.packages.len()
    );

    Ok(())
}
