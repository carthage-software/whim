use std::collections::BTreeMap;

use semver::VersionReq;

use crate::config::Configuration;
use crate::error::Error;
use crate::output;
use crate::package::Project;
use crate::package::Source;

struct Suggestion {
    source: String,
    requirement: String,
    owner: String,
}

#[tracing::instrument(level = "debug", skip_all)]
pub(super) fn execute(configuration: &Configuration) -> Result<(), Error> {
    let project = Project::inspect(configuration)?;
    let manifest = project.manifest();
    let mut suggestions = manifest
        .suggestion_requirements()?
        .into_iter()
        .map(|suggestion| Suggestion {
            source: suggestion.source.identity().to_owned(),
            requirement: suggestion.requirement.to_string(),
            owner: "the root project".to_owned(),
        })
        .collect::<Vec<_>>();

    let lock = project.current_lock(manifest)?;

    if let Some(lock) = &lock {
        for package in &lock.packages {
            let source = Source::parse(&package.source)?;
            if !project.is_installed(&source)? {
                continue;
            }

            suggestions.extend(package.suggestions.iter().map(|suggestion| Suggestion {
                source: suggestion.source.clone(),
                requirement: suggestion.version.clone(),
                owner: format!("{} {}", package.source, package.version),
            }));
        }
    }

    suggestions.sort_by(|left, right| {
        (&left.source, &left.requirement, &left.owner).cmp(&(
            &right.source,
            &right.requirement,
            &right.owner,
        ))
    });

    if suggestions.is_empty() {
        output::write("No package suggestions are available.\n")?;
        return Ok(());
    }

    let mut installed = BTreeMap::new();
    if let Some(lock) = &lock {
        for package in &lock.packages {
            let source = Source::parse(&package.source)?;
            if project.is_installed(&source)? {
                installed.insert(package.source.as_str(), &package.version);
            }
        }
    }

    let mut report = String::new();
    for suggestion in suggestions {
        let status = match installed.get(suggestion.source.as_str()) {
            Some(version)
                if VersionReq::parse(&suggestion.requirement)
                    .map_err(|source| Error::InvalidSuggestionRequirement {
                        dependency: suggestion.source.clone(),
                        requirement: suggestion.requirement.clone(),
                        source,
                    })?
                    .matches(version) =>
            {
                format!("installed at {version}")
            }
            Some(version) => format!("installed at {version}, outside the suggested range"),
            None => "not installed".to_owned(),
        };

        report.push_str(&suggestion.source);
        report.push(' ');
        report.push_str(&suggestion.requirement);
        report.push_str(" (");
        report.push_str(&status);
        report.push_str(")\n  suggested by ");
        report.push_str(&suggestion.owner);
        report.push('\n');
    }

    output::write(&report)?;
    Ok(())
}
