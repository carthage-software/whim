use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::Args;
use clap::ColorChoice;

use crate::color::should_use_colors;
use crate::config::Configuration;
use crate::error::Error;
use crate::pipeline::files::Selection;
use crate::service::lint::LintService;

#[derive(Args)]
pub(super) struct Arguments {
    /// Files and directories to lint. Omit to lint the project.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

#[tracing::instrument(name = "lint", level = "info", skip_all, err(level = "debug"))]
pub(super) fn execute(
    arguments: &Arguments,
    configuration: &Configuration,
    colors: ColorChoice,
) -> Result<ExitCode, Error> {
    let start = Instant::now();
    let lint = configuration.lint();
    let registry = lint.registry()?;
    tracing::debug!(root = %configuration.root().display(), targets = arguments.paths.len(), rules = registry.len(), minimum_fail_level = ?lint.minimum_fail_level, "resolved lint settings");
    for rule in registry.rules() {
        tracing::trace!(rule = rule.code(), "enabled lint rule");
    }

    if registry.is_empty() {
        tracing::warn!("no lint rules are enabled; only checking syntax");
    }

    let patterns = lint.patterns()?;
    let selection = Selection::new(&arguments.paths, configuration, &patterns)?;
    tracing::info!(
        files = selection.targets.len(),
        rules = registry.len(),
        "linting source files"
    );

    let linter = LintService::new(
        registry,
        lint.minimum_fail_level.clone(),
        selection.root,
        should_use_colors(colors),
    );

    Ok(linter.run(&selection.targets)?.report(start.elapsed()))
}
