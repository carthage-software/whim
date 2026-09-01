use std::path::Component;
use std::path::Path;

use globset::Glob;
use globset::GlobBuilder;
use globset::GlobSet;
use globset::GlobSetBuilder;
use serde::Deserialize;
use whim_formatter::settings::EndOfLine;
use whim_formatter::settings::FormatSettings;

use crate::config::Error;

const DEFAULT_INCLUDE: [&str; 1] = ["**/*.whim"];
const BUILT_IN_EXCLUDE: [&str; 2] = ["vendor", ".git"];

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FormatConfiguration {
    print_width: usize,
    tab_width: usize,
    use_tabs: bool,
    end_of_line: EndOfLine,
    include: Vec<String>,
    exclude: Vec<String>,
}

pub(crate) struct FormatPatterns {
    include: GlobSet,
    exclude: GlobSet,
}

impl Default for FormatConfiguration {
    fn default() -> Self {
        Self {
            print_width: 80,
            tab_width: 2,
            use_tabs: false,
            end_of_line: EndOfLine::Lf,
            include: DEFAULT_INCLUDE.map(str::to_owned).into(),
            exclude: Vec::new(),
        }
    }
}

impl FormatConfiguration {
    pub(crate) const fn settings(&self) -> FormatSettings {
        FormatSettings {
            print_width: self.print_width,
            tab_width: self.tab_width,
            use_tabs: self.use_tabs,
            end_of_line: self.end_of_line,
        }
    }

    pub(crate) fn patterns(&self) -> Result<FormatPatterns, Error> {
        let include = compile("include", self.include.iter().map(String::as_str))?;
        let exclude = compile(
            "exclude",
            BUILT_IN_EXCLUDE
                .into_iter()
                .chain(self.exclude.iter().map(String::as_str)),
        )?;

        Ok(FormatPatterns { include, exclude })
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.settings()
            .validate()
            .map_err(Error::InvalidFormatSettings)?;
        validate_patterns("include", &self.include)?;
        validate_patterns("exclude", &self.exclude)
    }
}

impl FormatPatterns {
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
        .map_err(|source| Error::CompileFormatPatterns { setting, source })
}

fn validate_patterns(setting: &'static str, patterns: &[String]) -> Result<(), Error> {
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
        return Err(Error::EmptyFormatPattern { setting });
    }
    if pattern.chars().any(char::is_control) {
        return Err(Error::ControlFormatPattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }
    if pattern.contains('\\') {
        return Err(Error::BackslashFormatPattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }

    let path = Path::new(pattern);
    if path.is_absolute() {
        return Err(Error::AbsoluteFormatPattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(Error::EscapingFormatPattern {
            setting,
            pattern: pattern.to_owned(),
        });
    }

    let normalized = pattern
        .strip_prefix("./")
        .unwrap_or(pattern)
        .trim_end_matches('/');
    if normalized.is_empty() {
        return Err(Error::EmptyFormatPattern { setting });
    }
    Ok(if normalized == "." { "**" } else { normalized })
}

fn glob(setting: &'static str, original: &str, pattern: &str) -> Result<Glob, Error> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .map_err(|source| Error::InvalidFormatPattern {
            setting,
            pattern: original.to_owned(),
            source,
        })
}
