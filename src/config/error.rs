use std::env::VarError;
use std::io::Error as IoError;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::str::ParseBoolError;

use globset::Error as GlobError;
use semver::Error as SemVerError;
use semver::Version;
use semver::VersionReq;
use spdx::ParseError as SpdxParseError;
use thiserror::Error as ThisError;
use toml::de::Error as TomlDecodingError;
use toml_edit::TomlError;
use url::ParseError as UrlParseError;
use whim_formatter::settings::SettingsError;

use crate::package::SourceError;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("could not read current directory: {0}")]
    CurrentDirectory(#[source] IoError),
    #[error("could not inspect manifest `{}`: {source}", path.display())]
    InspectManifest {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("could not read manifest `{}`: {source}", path.display())]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("manifest path `{}` is not a regular file", .0.display())]
    InvalidManifestPath(PathBuf),
    #[error("could not resolve `{}` while searching for a manifest: {source}", path.display())]
    ResolveSearchPath {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("config file `{}` does not exist", .0.display())]
    ConfigurationNotFound(PathBuf),
    #[error("could not resolve config path `{}`: {source}", path.display())]
    ResolveConfigurationPath {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("invalid manifest `{}`: {source}", path.display())]
    Manifest {
        path: PathBuf,
        #[source]
        source: Box<Self>,
    },
    #[error("could not decode the manifest: {0}")]
    DecodeManifest(#[source] TomlDecodingError),
    #[error("could not edit manifest `{}`: {source}", path.display())]
    EditManifest {
        path: PathBuf,
        #[source]
        source: TomlError,
    },
    #[error("manifest exceeds the {limit} byte limit")]
    ManifestTooLarge { limit: u64 },
    #[error("unsupported manifest version {actual}; expected {expected}")]
    UnsupportedManifestVersion { actual: u32, expected: u32 },
    #[error("invalid Whim version requirement `{requirement}`: {source}")]
    InvalidWhimRequirement {
        requirement: String,
        #[source]
        source: SemVerError,
    },
    #[error("the Whim build version is invalid")]
    InvalidCurrentVersion(#[source] SemVerError),
    #[error("invalid SPDX license expression `{expression}`: {source}")]
    InvalidLicenseExpression {
        expression: String,
        #[source]
        source: SpdxParseError,
    },
    #[error("invalid sponsor URL: {0}")]
    InvalidSponsorUrl(#[source] UrlParseError),
    #[error("sponsor URLs may not contain credentials")]
    SponsorCredentials,
    #[error("sponsor URL `{0}` must use HTTPS or HTTP")]
    UnsupportedSponsorScheme(String),
    #[error("invalid {group} Git source: {source}")]
    InvalidDependencySource {
        group: &'static str,
        #[source]
        source: SourceError,
    },
    #[error("invalid override {kind} Git source: {source}")]
    InvalidOverrideSource {
        kind: &'static str,
        #[source]
        source: SourceError,
    },
    #[error("invalid version requirement `{requirement}` for `{owner}`: {source}")]
    InvalidVersionRequirement {
        requirement: String,
        owner: String,
        #[source]
        source: SemVerError,
    },
    #[error("dependency `{0}` appears in both runtime and development dependencies")]
    DuplicateDependencyGroup(String),
    #[error("dependency manifests may not contain overrides")]
    DependencyOverride,
    #[error("duplicate normalized override for `{0}`")]
    DuplicateOverride(String),
    #[error("override for `{0}` resolves to itself")]
    SelfOverride(String),
    #[error("duplicate normalized {group} source `{dependency}`")]
    DuplicateNormalizedDependency {
        group: &'static str,
        dependency: String,
    },
    #[error("autoload prefix `{0}` must end in `\\`")]
    UnterminatedAutoloadPrefix(String),
    #[error("autoload prefix `{0}` is reserved or invalid")]
    ReservedAutoloadPrefix(String),
    #[error("autoload prefix `{0}` contains an empty segment")]
    EmptyAutoloadSegment(String),
    #[error("autoload prefix `{0}` contains an invalid segment")]
    InvalidAutoloadSegment(String),
    #[error("autoload directory for prefix `{0}` must not be empty")]
    EmptyAutoloadDirectory(String),
    #[error("autoload directory `{0}` must be relative")]
    AbsoluteAutoloadDirectory(String),
    #[error("autoload directory `{0}` may not escape the repository")]
    EscapingAutoloadDirectory(String),
    #[error("autoload directories may not contain control characters")]
    ControlAutoloadDirectory,
    #[error("invalid format settings: {0}")]
    InvalidFormatSettings(#[source] SettingsError),
    #[error("format.{setting} contains an empty pattern")]
    EmptyFormatPattern { setting: &'static str },
    #[error("format.{setting} pattern `{pattern}` must be relative")]
    AbsoluteFormatPattern {
        setting: &'static str,
        pattern: String,
    },
    #[error("format.{setting} pattern `{pattern}` may not escape the project")]
    EscapingFormatPattern {
        setting: &'static str,
        pattern: String,
    },
    #[error("format.{setting} pattern `{pattern}` must use `/` as its separator")]
    BackslashFormatPattern {
        setting: &'static str,
        pattern: String,
    },
    #[error("format.{setting} pattern `{pattern}` contains a control character")]
    ControlFormatPattern {
        setting: &'static str,
        pattern: String,
    },
    #[error("invalid format.{setting} pattern `{pattern}`: {source}")]
    InvalidFormatPattern {
        setting: &'static str,
        pattern: String,
        #[source]
        source: GlobError,
    },
    #[error("could not compile format.{setting} patterns: {source}")]
    CompileFormatPatterns {
        setting: &'static str,
        #[source]
        source: GlobError,
    },
    #[error("{owner} requires Whim {requirement}, but this is Whim {current}")]
    IncompatibleWhim {
        owner: String,
        requirement: VersionReq,
        current: Version,
    },
    #[error("could not find `whim.toml` from `{}` or any parent directory", .0.display())]
    ManifestNotFound(PathBuf),
    #[error("`{0}` must be a table")]
    ExpectedTable(&'static str),
    #[error("environment variable `{variable}` is not valid Unicode")]
    InvalidEnvironment {
        variable: &'static str,
        #[source]
        source: VarError,
    },
    #[error("`{variable}` must be `on` or `off`, not `{value}`")]
    InvalidOptimizationMode {
        variable: &'static str,
        value: String,
    },
    #[error("`{variable}` must be a non-negative integer, not `{value}`")]
    InvalidIntegerEnvironment {
        variable: &'static str,
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("`{variable}` must be `true` or `false`, not `{value}`")]
    InvalidBooleanEnvironment {
        variable: &'static str,
        value: String,
        #[source]
        source: ParseBoolError,
    },
}

impl Error {
    pub(crate) fn at(self, path: PathBuf) -> Self {
        Self::Manifest {
            path,
            source: Box::new(self),
        }
    }
}
