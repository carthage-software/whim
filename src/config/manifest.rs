use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Component;
use std::path::Path;

use semver::Version;
use semver::VersionReq;
use serde::Deserialize;
use serde::Serialize;
use spdx::Expression;
use url::Url;

use crate::config::Error;
use crate::config::FormatConfiguration;
use crate::config::RuntimeSettings;
use crate::filesystem;
use crate::filesystem::LimitedString;
use crate::package::Source;

pub(crate) const MANIFEST_NAME: &str = "whim.toml";
pub(crate) const LOCK_NAME: &str = "whim.lock";
pub(crate) const MAXIMUM_MANIFEST_BYTES: u64 = 1_048_576;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    #[serde(rename = "manifest-version")]
    version: u32,
    #[serde(rename = "package", default)]
    pub(crate) package: Package,
    #[serde(default)]
    pub(crate) requirements: Requirements,
    #[serde(default)]
    pub(crate) autoload: Autoload,
    #[serde(default)]
    pub(crate) dependencies: BTreeMap<String, Dependency>,
    #[serde(rename = "dev-dependencies", default)]
    pub(crate) development_dependencies: BTreeMap<String, Dependency>,
    #[serde(default)]
    pub(crate) conflicts: BTreeMap<String, Dependency>,
    #[serde(default)]
    pub(crate) suggests: BTreeMap<String, Dependency>,
    #[serde(default)]
    pub(crate) overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) format: FormatConfiguration,
    #[serde(default)]
    pub(crate) runtime: RuntimeSettings,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Package {
    pub(crate) repository: Option<String>,
    pub(crate) homepage: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) sponsor: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Requirements {
    pub(crate) whim: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Autoload {
    pub(crate) namespaces: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Dependency {
    Compact(String),
    Detailed(DependencyDetail),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DependencyDetail {
    version: String,
}

impl Dependency {
    pub(crate) fn requirement(&self) -> &str {
        match self {
            Self::Compact(requirement) => requirement,
            Self::Detailed(detail) => &detail.version,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DependencyRequirement {
    pub(crate) source: Source,
    pub(crate) requirement: VersionReq,
}

impl Manifest {
    pub(crate) fn parse(text: &str, root: bool) -> Result<Self, Error> {
        if text.len() as u64 > MAXIMUM_MANIFEST_BYTES {
            return Err(Error::ManifestTooLarge {
                limit: MAXIMUM_MANIFEST_BYTES,
            });
        }

        let manifest: Self = toml::from_str(text).map_err(Error::DecodeManifest)?;
        manifest.validate(root)?;
        Ok(manifest)
    }

    pub(crate) fn read(path: &Path, root: bool) -> Result<(Self, String), Error> {
        let text = read_text(path)?;
        let manifest = Self::parse(&text, root).map_err(|error| error.at(path.to_path_buf()))?;
        Ok((manifest, text))
    }

    fn validate(&self, root: bool) -> Result<(), Error> {
        if self.version != 1 {
            return Err(Error::UnsupportedManifestVersion {
                actual: self.version,
                expected: 1,
            });
        }

        if let Some(requirement) = &self.requirements.whim {
            VersionReq::parse(requirement).map_err(|source| Error::InvalidWhimRequirement {
                requirement: requirement.clone(),
                source,
            })?;
        }

        if let Some(license) = &self.package.license {
            Expression::parse(license).map_err(|source| Error::InvalidLicenseExpression {
                expression: license.clone(),
                source,
            })?;
        }

        if let Some(sponsor) = &self.package.sponsor {
            let url = Url::parse(sponsor).map_err(Error::InvalidSponsorUrl)?;
            if !url.username().is_empty() || url.password().is_some() {
                return Err(Error::SponsorCredentials);
            }

            if !matches!(url.scheme(), "https" | "http") {
                return Err(Error::UnsupportedSponsorScheme(sponsor.clone()));
            }
        }

        let runtime = Self::normalized_group(&self.dependencies, "dependencies")?;
        let development =
            Self::normalized_group(&self.development_dependencies, "development dependencies")?;
        Self::normalized_group(&self.conflicts, "conflicts")?;
        Self::normalized_group(&self.suggests, "suggestions")?;
        for source in runtime.keys() {
            if development.contains_key(source) {
                return Err(Error::DuplicateDependencyGroup(source.to_string()));
            }
        }

        if !root && !self.overrides.is_empty() {
            return Err(Error::DependencyOverride);
        }

        let mut overrides = BTreeSet::new();
        for (original, replacement) in &self.overrides {
            let original =
                Source::parse(original).map_err(|source| Error::InvalidOverrideSource {
                    kind: "source",
                    source,
                })?;

            let replacement =
                Source::parse(replacement).map_err(|source| Error::InvalidOverrideSource {
                    kind: "target",
                    source,
                })?;

            if !overrides.insert(original.clone()) {
                return Err(Error::DuplicateOverride(original.to_string()));
            }

            if original == replacement {
                return Err(Error::SelfOverride(original.to_string()));
            }
        }

        for (prefix, directory) in &self.autoload.namespaces {
            validate_prefix(prefix)?;
            validate_autoload_directory(prefix, directory)?;
        }

        self.format.validate()?;

        Ok(())
    }

    fn normalized_group(
        dependencies: &BTreeMap<String, Dependency>,
        group: &'static str,
    ) -> Result<BTreeMap<Source, VersionReq>, Error> {
        let mut normalized = BTreeMap::new();
        for (spelling, dependency) in dependencies {
            let source = Source::parse(spelling)
                .map_err(|source| Error::InvalidDependencySource { group, source })?;
            let requirement = VersionReq::parse(dependency.requirement()).map_err(|error| {
                Error::InvalidVersionRequirement {
                    requirement: dependency.requirement().to_owned(),
                    owner: source.to_string(),
                    source: error,
                }
            })?;

            if normalized.insert(source.clone(), requirement).is_some() {
                return Err(Error::DuplicateNormalizedDependency {
                    group,
                    dependency: source.to_string(),
                });
            }
        }

        Ok(normalized)
    }

    pub(crate) fn runtime_requirements(&self) -> Result<Vec<DependencyRequirement>, Error> {
        Self::requirements_for(&self.dependencies, "dependencies")
    }

    pub(crate) fn development_requirements(&self) -> Result<Vec<DependencyRequirement>, Error> {
        Self::requirements_for(&self.development_dependencies, "development dependencies")
    }

    pub(crate) fn conflict_requirements(&self) -> Result<Vec<DependencyRequirement>, Error> {
        Self::requirements_for(&self.conflicts, "conflicts")
    }

    pub(crate) fn suggestion_requirements(&self) -> Result<Vec<DependencyRequirement>, Error> {
        Self::requirements_for(&self.suggests, "suggestions")
    }

    fn requirements_for(
        dependencies: &BTreeMap<String, Dependency>,
        group: &'static str,
    ) -> Result<Vec<DependencyRequirement>, Error> {
        Ok(Self::normalized_group(dependencies, group)?
            .into_iter()
            .map(|(source, requirement)| DependencyRequirement {
                source,
                requirement,
            })
            .collect())
    }

    pub(crate) fn normalized_overrides(&self) -> Result<BTreeMap<Source, Source>, Error> {
        self.overrides
            .iter()
            .map(|(original, replacement)| {
                let original =
                    Source::parse(original).map_err(|source| Error::InvalidOverrideSource {
                        kind: "source",
                        source,
                    })?;

                let replacement =
                    Source::parse(replacement).map_err(|source| Error::InvalidOverrideSource {
                        kind: "target",
                        source,
                    })?;

                Ok((original, replacement))
            })
            .collect()
    }

    pub(crate) fn check_current_whim(&self, owner: &str) -> Result<(), Error> {
        let Some(requirement) = &self.requirements.whim else {
            return Ok(());
        };

        let requirement =
            VersionReq::parse(requirement).map_err(|source| Error::InvalidWhimRequirement {
                requirement: requirement.clone(),
                source,
            })?;

        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).map_err(Error::InvalidCurrentVersion)?;

        if requirement.matches(&current) {
            Ok(())
        } else {
            Err(Error::IncompatibleWhim {
                owner: owner.to_owned(),
                requirement,
                current,
            })
        }
    }
}

pub(super) fn read_text(path: &Path) -> Result<String, Error> {
    match filesystem::read_limited_string(path, MAXIMUM_MANIFEST_BYTES).map_err(|source| {
        Error::ReadManifest {
            path: path.to_path_buf(),
            source,
        }
    })? {
        LimitedString::Contents(text) => Ok(text),
        LimitedString::TooLarge => Err(Error::ManifestTooLarge {
            limit: MAXIMUM_MANIFEST_BYTES,
        }
        .at(path.to_path_buf())),
    }
}

fn validate_prefix(prefix: &str) -> Result<(), Error> {
    if prefix.is_empty() || !prefix.ends_with('\\') {
        return Err(Error::UnterminatedAutoloadPrefix(prefix.to_owned()));
    }

    if prefix.starts_with('\\') || prefix.starts_with("Whim\\") {
        return Err(Error::ReservedAutoloadPrefix(prefix.to_owned()));
    }

    for segment in prefix.trim_end_matches('\\').split('\\') {
        let mut bytes = segment.bytes();
        let Some(first) = bytes.next() else {
            return Err(Error::EmptyAutoloadSegment(prefix.to_owned()));
        };

        if segment == "_"
            || !(first == b'_' || first.is_ascii_alphabetic())
            || !bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidAutoloadSegment(prefix.to_owned()));
        }
    }

    Ok(())
}

fn validate_autoload_directory(prefix: &str, directory: &str) -> Result<(), Error> {
    if directory.is_empty() {
        return Err(Error::EmptyAutoloadDirectory(prefix.to_owned()));
    }

    let path = Path::new(directory);
    if path.is_absolute() {
        return Err(Error::AbsoluteAutoloadDirectory(directory.to_owned()));
    }

    if directory.chars().any(char::is_control) {
        return Err(Error::ControlAutoloadDirectory);
    }

    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::EscapingAutoloadDirectory(directory.to_owned()));
    }

    Ok(())
}
