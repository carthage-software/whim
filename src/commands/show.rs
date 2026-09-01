use std::iter::repeat_n;

use clap::Args;

use crate::config::Configuration;
use crate::config::DependencyRequirement;
use crate::config::MANIFEST_NAME;
use crate::config::Manifest;
use crate::error::Error;
use crate::output;
use crate::package::Project;
use crate::package::Source;

const FIELD_WIDTH: usize = 16;

#[derive(Args)]
pub(super) struct Arguments {
    /// The installed Git repository to inspect.
    source: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(source = %arguments.source))]
pub(super) fn execute(arguments: &Arguments, configuration: &Configuration) -> Result<(), Error> {
    let source = Source::parse(&arguments.source)?;
    let project = Project::inspect(configuration)?;
    let lock = project
        .current_lock(project.manifest())?
        .ok_or_else(|| Error::LockNotFound(project.lockfile().to_path_buf()))?;
    let package = lock
        .packages
        .iter()
        .find(|package| package.source == source.identity())
        .ok_or_else(|| Error::MissingLockedSource(source.to_string()))?;

    if !project.is_installed(&source)? {
        return Err(Error::PackageNotInstalled(source.to_string()));
    }

    let path = project.root().join("vendor/packages").join(source.digest());
    let (manifest, _) = Manifest::read(&path.join(MANIFEST_NAME), false)?;
    let mut report = String::new();

    push_field(&mut report, "source", &package.source);
    if let Some(resolved_source) = &package.resolved_source {
        push_field(&mut report, "resolved source", resolved_source);
    }
    if let Some(description) = &manifest.package.description {
        push_field(&mut report, "description", description);
    }
    push_field(&mut report, "version", &package.version.to_string());
    push_field(&mut report, "tag", &package.tag);
    push_field(&mut report, "commit", &package.commit);
    push_field(&mut report, "tree", &package.tree);
    push_field(&mut report, "path", &path.to_string_lossy());
    push_field(
        &mut report,
        "license",
        manifest.package.license.as_deref().unwrap_or("proprietary"),
    );
    push_optional_field(
        &mut report,
        "repository",
        manifest.package.repository.as_deref(),
    );
    push_optional_field(
        &mut report,
        "homepage",
        manifest.package.homepage.as_deref(),
    );
    push_optional_field(&mut report, "author", manifest.package.author.as_deref());
    push_optional_field(&mut report, "sponsor", manifest.package.sponsor.as_deref());

    push_autoload(&mut report, &manifest);
    push_requirements(
        &mut report,
        "requires",
        manifest.requirements.whim.as_deref(),
        &manifest.runtime_requirements()?,
    );
    push_requirements(
        &mut report,
        "requires (development)",
        None,
        &manifest.development_requirements()?,
    );
    push_requirements(
        &mut report,
        "conflicts",
        None,
        &manifest.conflict_requirements()?,
    );
    push_requirements(
        &mut report,
        "suggests",
        None,
        &manifest.suggestion_requirements()?,
    );

    output::write(&report)?;
    Ok(())
}

fn push_field(report: &mut String, name: &str, value: &str) {
    report.push_str(name);
    report.extend(repeat_n(' ', FIELD_WIDTH.saturating_sub(name.len())));
    report.push_str(": ");
    push_escaped(report, value);
    report.push('\n');
}

fn push_optional_field(report: &mut String, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        push_field(report, name, value);
    }
}

fn push_autoload(report: &mut String, manifest: &Manifest) {
    if manifest.autoload.namespaces.is_empty() {
        return;
    }

    report.push_str("\nautoload\n");
    for (prefix, directory) in &manifest.autoload.namespaces {
        report.push_str("  ");
        push_escaped(report, prefix);
        report.push_str(" => ");
        push_escaped(report, directory);
        report.push('\n');
    }
}

fn push_requirements(
    report: &mut String,
    heading: &str,
    whim: Option<&str>,
    requirements: &[DependencyRequirement],
) {
    if whim.is_none() && requirements.is_empty() {
        return;
    }

    report.push('\n');
    report.push_str(heading);
    report.push('\n');
    if let Some(requirement) = whim {
        report.push_str("  whim ");
        push_escaped(report, requirement);
        report.push('\n');
    }
    for requirement in requirements {
        report.push_str("  ");
        report.push_str(requirement.source.identity());
        report.push(' ');
        report.push_str(&requirement.requirement.to_string());
        report.push('\n');
    }
}

fn push_escaped(report: &mut String, value: &str) {
    for character in value.chars() {
        if character.is_control() {
            report.extend(character.escape_default());
        } else {
            report.push(character);
        }
    }
}
