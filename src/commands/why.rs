use std::collections::BTreeMap;
use std::collections::VecDeque;

use clap::Args;

use crate::config::Configuration;
use crate::error::Error;
use crate::output;
use crate::package::LockFile;
use crate::package::LockedPackage;
use crate::package::Project;
use crate::package::Source;

#[derive(Args)]
pub(super) struct Arguments {
    /// The locked Git repository to explain.
    source: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(source = %arguments.source))]
pub(super) fn execute(arguments: &Arguments, configuration: &Configuration) -> Result<(), Error> {
    let source = Source::parse(&arguments.source)?;
    let project = Project::inspect(configuration)?;
    let manifest = project.manifest();
    let lock = project
        .current_lock(manifest)?
        .ok_or_else(|| Error::LockNotFound(project.lockfile().to_path_buf()))?;

    let packages = lock
        .packages
        .iter()
        .map(|package| (package.source.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let target = packages
        .get(source.identity())
        .ok_or_else(|| Error::MissingLockedSource(source.to_string()))?;
    let (chain, development) = dependency_chain(&lock, source.identity(), &packages)
        .ok_or_else(|| Error::MissingLockedSource(source.to_string()))?;
    let state = if project.is_installed(&source)? {
        "is installed because:"
    } else {
        "is locked but not installed. It is required because:"
    };

    let mut report = format!("{} {} {state}\n", target.source, target.version);
    report.push_str(if development {
        "  the root project requires it for development\n"
    } else {
        "  the root project requires it at runtime\n"
    });

    for source in chain {
        let package = packages
            .get(source)
            .ok_or_else(|| Error::MissingLockedSource(source.to_owned()))?;
        report.push_str("  -> ");
        report.push_str(&package.source);
        report.push(' ');
        report.push_str(&package.version.to_string());
        report.push('\n');
    }

    output::write(&report)?;
    Ok(())
}

fn dependency_chain<'a>(
    lock: &'a LockFile,
    target: &str,
    packages: &BTreeMap<&'a str, &'a LockedPackage>,
) -> Option<(Vec<&'a str>, bool)> {
    if let Some(chain) = dependency_chain_from(&lock.root.runtime, target, packages) {
        return Some((chain, false));
    }
    dependency_chain_from(&lock.root.development, target, packages).map(|chain| (chain, true))
}

fn dependency_chain_from<'a>(
    roots: &'a [String],
    target: &str,
    packages: &BTreeMap<&'a str, &'a LockedPackage>,
) -> Option<Vec<&'a str>> {
    let mut pending = VecDeque::new();
    let mut previous = BTreeMap::<&str, Option<&str>>::new();
    for source in roots {
        if previous.insert(source, None).is_none() {
            pending.push_back(source.as_str());
        }
    }

    while let Some(source) = pending.pop_front() {
        if source == target {
            let mut chain = Vec::new();
            let mut current = Some(source);
            while let Some(item) = current {
                chain.push(item);
                current = previous.get(item).copied().flatten();
            }
            chain.reverse();
            return Some(chain);
        }

        let package = packages.get(source)?;
        for dependency in &package.dependencies {
            let dependency = dependency.as_str();
            if previous.contains_key(dependency) {
                continue;
            }

            previous.insert(dependency, Some(source));
            pending.push_back(dependency);
        }
    }

    None
}
