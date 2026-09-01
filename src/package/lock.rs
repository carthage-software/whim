use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::io::Error as IoError;
use std::path::Path;
use std::path::PathBuf;

use semver::Version;
use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;
use spdx::ParseError as SpdxParseError;
use thiserror::Error as ThisError;
use toml::de::Error as TomlDecodingError;
use toml::ser::Error as TomlEncodingError;
use url::ParseError as UrlParseError;

use crate::config::Error as ConfigurationError;
use crate::filesystem;
use crate::filesystem::LimitedString;
use crate::package::resolve::ResolvedGraph;
use crate::package::source::Error as SourceError;
use crate::package::source::Source;

const MAXIMUM_LOCK_BYTES: u64 = 1_048_576;

#[derive(Clone, Copy, Debug)]
pub(crate) enum SourceKind {
    Dependency,
    Resolved,
    RootDevelopment,
    RootRuntime,
    Source,
    Suggestion,
}

impl fmt::Display for SourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Dependency => "dependency",
            Self::Resolved => "resolved",
            Self::RootDevelopment => "root development dependency",
            Self::RootRuntime => "root runtime dependency",
            Self::Source => "package",
            Self::Suggestion => "suggestion",
        })
    }
}

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error(transparent)]
    Configuration(#[from] ConfigurationError),
    #[error("missing installed checksum for `{0}`")]
    MissingChecksum(String),
    #[error("lockfile exceeds the {limit} byte limit")]
    TooLarge { limit: u64 },
    #[error("could not decode lockfile: {0}")]
    Decode(#[source] TomlDecodingError),
    #[error("unsupported lockfile version {actual}; expected {expected}")]
    UnsupportedVersion { actual: u32, expected: u32 },
    #[error("could not read lockfile `{}`: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("invalid lockfile `{}`: {source}", path.display())]
    At {
        path: PathBuf,
        #[source]
        source: Box<Self>,
    },
    #[error("could not encode lockfile: {0}")]
    Encode(#[source] TomlEncodingError),
    #[error("a root source appears in both dependency groups")]
    DuplicateRootDependency,
    #[error("lockfile package entries must be uniquely sorted")]
    UnsortedPackages,
    #[error("root runtime dependencies must be uniquely sorted")]
    UnsortedRootRuntime,
    #[error("root development dependencies must be uniquely sorted")]
    UnsortedRootDevelopment,
    #[error("dependencies for `{0}` must be uniquely sorted")]
    UnsortedDependencies(String),
    #[error("suggestions for `{0}` must be uniquely sorted")]
    UnsortedSuggestions(String),
    #[error("invalid locked tag `{tag}`: {source}")]
    InvalidTag {
        tag: String,
        #[source]
        source: semver::Error,
    },
    #[error("locked tag `{tag}` does not describe version {version}")]
    TagVersionMismatch { tag: String, version: Version },
    #[error("invalid suggestion requirement `{requirement}` for `{owner}`: {source}")]
    InvalidSuggestionRequirement {
        requirement: String,
        owner: String,
        #[source]
        source: semver::Error,
    },
    #[error("invalid SPDX license expression `{expression}`: {source}")]
    InvalidLicense {
        expression: String,
        #[source]
        source: SpdxParseError,
    },
    #[error("invalid sponsor URL: {0}")]
    InvalidSponsor(#[source] UrlParseError),
    #[error("sponsor URLs may not contain credentials")]
    SponsorCredentials,
    #[error("sponsor URL `{0}` must use HTTPS or HTTP")]
    UnsupportedSponsorScheme(String),
    #[error("lockfile refers to missing package `{0}`")]
    MissingReference(String),
    #[error("lockfile contains unreachable package `{0}`")]
    UnreachablePackage(String),
    #[error("lockfile dependency graph contains a cycle through `{0}`")]
    DependencyCycle(String),
    #[error("invalid {kind} Git source: {source}")]
    InvalidSource {
        kind: SourceKind,
        #[source]
        source: SourceError,
    },
    #[error("{kind} source `{value}` is not normalized")]
    NonNormalizedSource { kind: SourceKind, value: String },
    #[error("{field} digest must use BLAKE3")]
    InvalidDigestAlgorithm { field: &'static str },
    #[error("{field} is not a valid {length}-digit hexadecimal value")]
    InvalidHex { field: &'static str, length: usize },
    #[error("{field} is not a full Git object identifier")]
    InvalidGitId { field: &'static str },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LockFile {
    #[serde(rename = "lock-version")]
    lock_version: u32,
    pub(crate) manifest: String,
    pub(crate) root: LockRoot,
    #[serde(rename = "packages", default)]
    pub(crate) packages: Vec<LockedPackage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LockRoot {
    pub(crate) runtime: Vec<String>,
    pub(crate) development: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LockedPackage {
    pub(crate) source: String,
    #[serde(rename = "resolved-source", skip_serializing_if = "Option::is_none")]
    pub(crate) resolved_source: Option<String>,
    pub(crate) version: Version,
    pub(crate) tag: String,
    pub(crate) commit: String,
    pub(crate) tree: String,
    pub(crate) manifest: String,
    pub(crate) checksum: String,
    pub(crate) dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sponsor: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) suggestions: Vec<LockedSuggestion>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LockedSuggestion {
    pub(crate) source: String,
    pub(crate) version: String,
}

impl LockFile {
    pub(crate) fn from_graph(
        manifest: String,
        graph: &ResolvedGraph,
        checksums: &BTreeMap<Source, String>,
    ) -> Result<Self, Error> {
        let mut runtime = graph
            .runtime
            .iter()
            .map(|source| source.identity().to_owned())
            .collect::<Vec<_>>();
        runtime.sort();
        let mut development = graph
            .development
            .iter()
            .map(|source| source.identity().to_owned())
            .collect::<Vec<_>>();
        development.sort();

        let mut packages = Vec::with_capacity(graph.packages.len());
        for package in graph.packages.values() {
            let mut dependencies = package
                .dependencies
                .iter()
                .map(|source| source.identity().to_owned())
                .collect::<Vec<_>>();
            dependencies.sort();
            let checksum = checksums
                .get(&package.source)
                .cloned()
                .ok_or_else(|| Error::MissingChecksum(package.source.to_string()))?;
            let suggestions = package
                .manifest
                .suggestion_requirements()?
                .into_iter()
                .map(|suggestion| LockedSuggestion {
                    source: suggestion.source.identity().to_owned(),
                    version: suggestion.requirement.to_string(),
                })
                .collect();
            packages.push(LockedPackage {
                source: package.source.identity().to_owned(),
                resolved_source: package
                    .resolved_source
                    .as_ref()
                    .map(|source| source.identity().to_owned()),
                version: package.version.clone(),
                tag: package.tag.clone(),
                commit: package.commit.clone(),
                tree: package.tree.clone(),
                manifest: package.manifest.consumed_resolution_hash()?,
                checksum,
                dependencies,
                license: package.manifest.package.license.clone(),
                sponsor: package.manifest.package.sponsor.clone(),
                suggestions,
            });
        }
        packages.sort_by(|left, right| left.source.cmp(&right.source));

        Ok(Self {
            lock_version: 1,
            manifest,
            root: LockRoot {
                runtime,
                development,
            },
            packages,
        })
    }

    pub(crate) fn parse(text: &str) -> Result<Self, Error> {
        if text.len() as u64 > MAXIMUM_LOCK_BYTES {
            return Err(Error::TooLarge {
                limit: MAXIMUM_LOCK_BYTES,
            });
        }
        let lock: Self = toml::from_str(text).map_err(Error::Decode)?;
        if lock.lock_version != 1 {
            return Err(Error::UnsupportedVersion {
                actual: lock.lock_version,
                expected: 1,
            });
        }
        lock.validate()?;
        Ok(lock)
    }

    pub(crate) fn read(path: &Path) -> Result<Self, Error> {
        let text =
            match filesystem::read_limited_string(path, MAXIMUM_LOCK_BYTES).map_err(|source| {
                Error::Read {
                    path: path.to_path_buf(),
                    source,
                }
            })? {
                LimitedString::Contents(text) => text,
                LimitedString::TooLarge => {
                    return Err(Error::At {
                        path: path.to_path_buf(),
                        source: Box::new(Error::TooLarge {
                            limit: MAXIMUM_LOCK_BYTES,
                        }),
                    });
                }
            };
        Self::parse(&text).map_err(|source| Error::At {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    }

    pub(crate) fn render(&self) -> Result<String, Error> {
        toml::to_string_pretty(self).map_err(Error::Encode)
    }

    pub(crate) fn preferred_versions(&self) -> Result<BTreeMap<Source, Version>, Error> {
        self.packages
            .iter()
            .map(|package| {
                let source =
                    Source::parse(&package.source).map_err(|source| Error::InvalidSource {
                        kind: SourceKind::Source,
                        source,
                    })?;
                Ok((source, package.version.clone()))
            })
            .collect()
    }

    fn validate(&self) -> Result<(), Error> {
        validate_root(&self.root)?;
        let mut previous = None;
        for package in &self.packages {
            validate_package(package, previous)?;
            previous = Some(package.source.as_str());
        }
        validate_references(self)?;
        validate_digest("manifest", &self.manifest)?;
        Ok(())
    }
}

fn validate_root(root: &LockRoot) -> Result<(), Error> {
    if !is_uniquely_sorted(&root.runtime) {
        return Err(Error::UnsortedRootRuntime);
    }
    validate_sources(SourceKind::RootRuntime, &root.runtime)?;
    if !is_uniquely_sorted(&root.development) {
        return Err(Error::UnsortedRootDevelopment);
    }
    validate_sources(SourceKind::RootDevelopment, &root.development)?;
    if root
        .runtime
        .iter()
        .any(|source| root.development.binary_search(source).is_ok())
    {
        return Err(Error::DuplicateRootDependency);
    }
    Ok(())
}

fn validate_package(package: &LockedPackage, previous: Option<&str>) -> Result<(), Error> {
    validate_normalized_source(SourceKind::Source, &package.source)?;
    if let Some(resolved) = &package.resolved_source {
        validate_normalized_source(SourceKind::Resolved, resolved)?;
    }
    if previous.is_some_and(|previous| previous >= package.source.as_str()) {
        return Err(Error::UnsortedPackages);
    }
    if !is_uniquely_sorted(&package.dependencies) {
        return Err(Error::UnsortedDependencies(package.source.clone()));
    }
    validate_sources(SourceKind::Dependency, &package.dependencies)?;
    validate_suggestions(package)?;
    validate_metadata(package)?;
    let expected_tag = package.tag.strip_prefix('v').unwrap_or(&package.tag);
    let tag_version = Version::parse(expected_tag).map_err(|source| Error::InvalidTag {
        tag: package.tag.clone(),
        source,
    })?;
    if tag_version != package.version {
        return Err(Error::TagVersionMismatch {
            tag: package.tag.clone(),
            version: package.version.clone(),
        });
    }
    validate_digest("manifest", &package.manifest)?;
    validate_digest("checksum", &package.checksum)?;
    validate_git_id("commit", &package.commit)?;
    validate_git_id("tree", &package.tree)
}

fn validate_suggestions(package: &LockedPackage) -> Result<(), Error> {
    if !package
        .suggestions
        .windows(2)
        .all(|pair| pair[0].source < pair[1].source)
    {
        return Err(Error::UnsortedSuggestions(package.source.clone()));
    }
    for suggestion in &package.suggestions {
        validate_normalized_source(SourceKind::Suggestion, &suggestion.source)?;
        VersionReq::parse(&suggestion.version).map_err(|source| {
            Error::InvalidSuggestionRequirement {
                requirement: suggestion.version.clone(),
                owner: package.source.clone(),
                source,
            }
        })?;
    }
    Ok(())
}

fn validate_metadata(package: &LockedPackage) -> Result<(), Error> {
    if let Some(license) = &package.license {
        spdx::Expression::parse(license).map_err(|source| Error::InvalidLicense {
            expression: license.clone(),
            source,
        })?;
    }
    if let Some(sponsor) = &package.sponsor {
        let url = url::Url::parse(sponsor).map_err(Error::InvalidSponsor)?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Error::SponsorCredentials);
        }
        if !matches!(url.scheme(), "https" | "http") {
            return Err(Error::UnsupportedSponsorScheme(sponsor.clone()));
        }
    }
    Ok(())
}

fn validate_references(lock: &LockFile) -> Result<(), Error> {
    let packages = lock
        .packages
        .iter()
        .map(|package| (package.source.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    for source in lock
        .root
        .runtime
        .iter()
        .chain(&lock.root.development)
        .chain(
            lock.packages
                .iter()
                .flat_map(|package| &package.dependencies),
        )
    {
        if !packages.contains_key(source.as_str()) {
            return Err(Error::MissingReference(source.clone()));
        }
    }

    let mut active = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut pending = lock
        .root
        .runtime
        .iter()
        .chain(&lock.root.development)
        .rev()
        .map(|source| (source.as_str(), false))
        .collect::<Vec<_>>();
    while let Some((source, finish)) = pending.pop() {
        if finish {
            active.remove(source);
            complete.insert(source);
            continue;
        }

        if complete.contains(source) {
            continue;
        }

        if !active.insert(source) {
            return Err(Error::DependencyCycle(source.to_owned()));
        }

        let Some(package) = packages.get(source) else {
            return Err(Error::MissingReference(source.to_owned()));
        };
        pending.push((source, true));
        pending.extend(
            package
                .dependencies
                .iter()
                .rev()
                .map(|dependency| (dependency.as_str(), false)),
        );
    }

    if let Some(source) = packages.keys().find(|source| !complete.contains(**source)) {
        return Err(Error::UnreachablePackage((*source).to_owned()));
    }

    Ok(())
}

fn validate_normalized_source(kind: SourceKind, source: &str) -> Result<(), Error> {
    let normalized =
        Source::parse(source).map_err(|source| Error::InvalidSource { kind, source })?;
    if normalized.identity() != source {
        return Err(Error::NonNormalizedSource {
            kind,
            value: source.to_owned(),
        });
    }
    Ok(())
}

fn validate_sources(kind: SourceKind, sources: &[String]) -> Result<(), Error> {
    for source in sources {
        validate_normalized_source(kind, source)?;
    }
    Ok(())
}

fn is_uniquely_sorted(sources: &[String]) -> bool {
    sources.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_digest(name: &'static str, value: &str) -> Result<(), Error> {
    let Some(hex) = value.strip_prefix("blake3:") else {
        return Err(Error::InvalidDigestAlgorithm { field: name });
    };
    validate_hex(name, hex, 64)
}

fn validate_hex(name: &'static str, value: &str, length: usize) -> Result<(), Error> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::InvalidHex {
            field: name,
            length,
        });
    }
    Ok(())
}

fn validate_git_id(name: &'static str, value: &str) -> Result<(), Error> {
    if value.len() != 40 && value.len() != 64 {
        return Err(Error::InvalidGitId { field: name });
    }
    validate_hex(name, value, value.len())
}

#[cfg(test)]
mod tests {
    use crate::package::lock::Error;
    use crate::package::lock::LockFile;

    const DIGEST: &str = "blake3:0000000000000000000000000000000000000000000000000000000000000000";

    fn package(source: &str, dependencies: &str) -> String {
        format!(
            "[[packages]]\nsource = \"{source}\"\nversion = \"1.0.0\"\ntag = \"1.0.0\"\ncommit = \"0000000000000000000000000000000000000000\"\ntree = \"0000000000000000000000000000000000000000\"\nmanifest = \"{DIGEST}\"\nchecksum = \"{DIGEST}\"\ndependencies = [{dependencies}]\n"
        )
    }

    #[test]
    fn unknown_fields_fail_closed() {
        let error = LockFile::parse(
            "lock-version = 1\nmanifest = \"blake3:0000000000000000000000000000000000000000000000000000000000000000\"\nunknown = true\n[root]\nruntime = []\ndevelopment = []\n",
        )
        .expect_err("unknown fields must fail");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn sponsor_credentials_fail_closed_without_leaking() {
        let text = format!(
            "lock-version = 1\nmanifest = \"{DIGEST}\"\n[root]\nruntime = [\"git+https://example.com/acme/package\"]\ndevelopment = []\n[[packages]]\nsource = \"git+https://example.com/acme/package\"\nversion = \"1.0.0\"\ntag = \"1.0.0\"\ncommit = \"0000000000000000000000000000000000000000\"\ntree = \"0000000000000000000000000000000000000000\"\nmanifest = \"{DIGEST}\"\nchecksum = \"{DIGEST}\"\ndependencies = []\nsponsor = \"https://user:secret@example.com\"\n"
        );
        let error = LockFile::parse(&text).expect_err("sponsor credentials must fail");

        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn unreachable_packages_fail_closed() {
        let text = format!(
            "lock-version = 1\nmanifest = \"{DIGEST}\"\n[root]\nruntime = []\ndevelopment = []\n{}",
            package("git+https://example.com/acme/orphan", ""),
        );
        let error = LockFile::parse(&text).expect_err("orphan packages must fail");

        assert!(matches!(error, Error::UnreachablePackage(_)));
    }

    #[test]
    fn dependency_cycles_fail_closed() {
        let first = "git+https://example.com/acme/a";
        let second = "git+https://example.com/acme/b";
        let text = format!(
            "lock-version = 1\nmanifest = \"{DIGEST}\"\n[root]\nruntime = [\"{first}\"]\ndevelopment = []\n{}{}",
            package(first, &format!("\"{second}\"")),
            package(second, &format!("\"{first}\"")),
        );
        let error = LockFile::parse(&text).expect_err("dependency cycles must fail");

        assert!(matches!(error, Error::DependencyCycle(_)));
    }

    #[test]
    fn shared_dependencies_are_not_cycles() {
        let first = "git+https://example.com/acme/a";
        let second = "git+https://example.com/acme/b";
        let shared = "git+https://example.com/acme/shared";
        let text = format!(
            "lock-version = 1\nmanifest = \"{DIGEST}\"\n[root]\nruntime = [\"{first}\", \"{second}\"]\ndevelopment = []\n{}{}{}",
            package(first, &format!("\"{shared}\"")),
            package(second, &format!("\"{shared}\"")),
            package(shared, ""),
        );

        LockFile::parse(&text).expect("a shared dependency is acyclic");
    }

    #[test]
    fn invalid_suggestion_names_its_owner() {
        let owner = "git+https://example.com/acme/owner";
        let suggested = "git+https://example.com/acme/suggested";
        let text = format!(
            "lock-version = 1\nmanifest = \"{DIGEST}\"\n[root]\nruntime = [\"{owner}\"]\ndevelopment = []\n[[packages]]\nsource = \"{owner}\"\nversion = \"1.0.0\"\ntag = \"1.0.0\"\ncommit = \"0000000000000000000000000000000000000000\"\ntree = \"0000000000000000000000000000000000000000\"\nmanifest = \"{DIGEST}\"\nchecksum = \"{DIGEST}\"\ndependencies = []\n[[packages.suggestions]]\nsource = \"{suggested}\"\nversion = \"not a requirement\"\n"
        );
        let error = LockFile::parse(&text).expect_err("the suggestion requirement must fail");

        assert!(error.to_string().contains(owner));
        assert!(!error.to_string().contains(&format!("for `{suggested}`")));
    }
}
