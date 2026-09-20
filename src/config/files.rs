use std::path::Component;
use std::path::Path;

use globset::Glob;
use globset::GlobBuilder;
use globset::GlobSet;
use globset::GlobSetBuilder;

use crate::config::Error;

pub(super) const DEFAULT_INCLUDE: [&str; 1] = ["**/*.whim"];
const BUILT_IN_EXCLUDE: [&str; 2] = ["vendor", ".git"];

pub(crate) struct FilePatterns {
    include: GlobSet,
    exclude: GlobSet,
}

impl FilePatterns {
    pub(super) fn new(
        include_setting: &'static str,
        include: &[String],
        exclude_setting: &'static str,
        exclude: &[String],
    ) -> Result<Self, Error> {
        tracing::debug!(
            setting = include_setting,
            ?include,
            ?exclude,
            built_in_exclude = ?BUILT_IN_EXCLUDE,
            "building file filters",
        );
        Ok(Self {
            include: compile(include_setting, include.iter().map(String::as_str))?,
            exclude: compile(
                exclude_setting,
                BUILT_IN_EXCLUDE
                    .into_iter()
                    .chain(exclude.iter().map(String::as_str)),
            )?,
        })
    }

    pub(crate) fn includes(&self, path: &Path) -> bool {
        self.include.is_match(path)
    }
    pub(crate) fn excludes(&self, path: &Path) -> bool {
        self.exclude.is_match(path)
    }
}

fn compile<'pattern>(
    setting: &'static str,
    patterns: impl IntoIterator<Item = &'pattern str>,
) -> Result<GlobSet, Error> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let normalized = validate(setting, pattern)?;
        set.add(glob(setting, pattern, normalized)?);

        if normalized != "**" {
            let descendants = format!("{normalized}/**");
            set.add(glob(setting, pattern, &descendants)?);
        }
    }

    set.build()
        .map_err(|source| Error::CompileFilePatterns { setting, source })
}

pub(super) fn validate_patterns(setting: &'static str, patterns: &[String]) -> Result<(), Error> {
    for pattern in patterns {
        let normalized = validate(setting, pattern)?;
        glob(setting, pattern, normalized)?;
    }
    Ok(())
}

fn validate<'pattern>(
    setting: &'static str,
    pattern: &'pattern str,
) -> Result<&'pattern str, Error> {
    if pattern.is_empty() {
        return Err(Error::EmptyFilePattern { setting });
    }
    if pattern.chars().any(char::is_control) {
        return Err(Error::ControlFilePattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }
    if pattern.contains('\\') {
        return Err(Error::BackslashFilePattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }

    let path = Path::new(pattern);
    if path.has_root()
        || path
            .components()
            .any(|component| matches!(component, Component::Prefix(_)))
    {
        return Err(Error::AbsoluteFilePattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(Error::EscapingFilePattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }

    let normalized = pattern
        .strip_prefix("./")
        .unwrap_or(pattern)
        .trim_end_matches('/');
    if normalized.is_empty() {
        return Err(Error::EmptyFilePattern { setting });
    }
    Ok(if normalized == "." { "**" } else { normalized })
}

fn glob(setting: &'static str, original: &str, pattern: &str) -> Result<Glob, Error> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .map_err(|source| Error::InvalidFilePattern {
            setting,
            pattern: original.to_owned(),
            source,
        })
}
