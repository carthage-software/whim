use spdx::Expression;
use spdx::LicenseItem;

use crate::config::Manifest;
use crate::package::Error;
use crate::package::LockFile;
use crate::package::ResolvedGraph;
use crate::package::Source;
use crate::package::install::selection::locked_package;
use crate::package::install::selection::selected_graph_sources;
use crate::package::install::selection::selected_locked_sources;
use crate::package::resolve::Error as ResolutionError;

pub(crate) fn warn_graph(
    root: &Manifest,
    graph: &ResolvedGraph,
    no_dev: bool,
) -> Result<(), Error> {
    let selected = selected_graph_sources(graph, no_dev)?;
    for source in selected {
        let package = graph
            .packages
            .get(&source)
            .ok_or_else(|| ResolutionError::MissingResolvedPackage(source.to_string()))?;
        warn_incompatible(
            root.package.license.as_deref(),
            &source,
            package.manifest.package.license.as_deref(),
        );
    }

    Ok(())
}

pub(crate) fn warn_lock(root: &Manifest, lock: &LockFile, no_dev: bool) -> Result<(), Error> {
    for source in selected_locked_sources(lock, no_dev)? {
        let package = locked_package(lock, &source)?;
        warn_incompatible(
            root.package.license.as_deref(),
            &source,
            package.license.as_deref(),
        );
    }

    Ok(())
}

fn warn_incompatible(root: Option<&str>, source: &Source, dependency: Option<&str>) {
    if licenses_may_conflict(root, dependency) {
        tracing::warn!(
            "`{source}` uses {}, which may be incompatible with the root project's {} license",
            dependency.unwrap_or("a proprietary license"),
            root.unwrap_or("proprietary")
        );
    }
}

fn licenses_may_conflict(root: Option<&str>, dependency: Option<&str>) -> bool {
    match (root, dependency) {
        (_, None) => true,
        (None, Some(dependency)) => requires_copyleft(dependency),
        (Some(root), Some(dependency)) => {
            requires_copyleft(dependency) && !offers_dependency_terms(root, dependency)
        }
    }
}

fn requires_copyleft(license: &str) -> bool {
    Expression::parse(license).is_ok_and(|expression| {
        !expression.evaluate(|requirement| match requirement.license {
            LicenseItem::Spdx { id, .. } => !id.is_copyleft(),
            LicenseItem::Other { .. } => false,
        })
    })
}

fn offers_dependency_terms(root: &str, dependency: &str) -> bool {
    let (Ok(root), Ok(dependency)) = (Expression::parse(root), Expression::parse(dependency))
    else {
        return false;
    };

    root.evaluate(|root| {
        dependency
            .requirements()
            .any(|dependency| dependency.req == *root)
    })
}

#[cfg(test)]
mod tests {
    use crate::package::license::licenses_may_conflict;

    #[test]
    fn proprietary_dependencies_are_never_assumed_compatible() {
        assert!(licenses_may_conflict(Some("MIT"), None));
        assert!(licenses_may_conflict(None, None));
    }

    #[test]
    fn copyleft_dependencies_require_a_compatible_root_license() {
        assert!(licenses_may_conflict(Some("MIT"), Some("GPL-3.0-only")));
        assert!(licenses_may_conflict(Some("MPL-2.0"), Some("GPL-3.0-only")));
        assert!(!licenses_may_conflict(
            Some("GPL-3.0-only"),
            Some("GPL-3.0-only")
        ));
        assert!(!licenses_may_conflict(
            Some("MIT OR GPL-3.0-only"),
            Some("GPL-3.0-only")
        ));
        assert!(!licenses_may_conflict(Some("MIT"), Some("MIT")));
    }
}
