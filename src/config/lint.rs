use annotate_snippets::Level;
use serde::Deserialize;
use whim_linter::registry::RuleRegistry;
use whim_linter::settings::RulesSettings;
use whim_linter::settings::Settings;

use crate::config::Error;
use crate::config::FilePatterns;
use crate::config::files::DEFAULT_INCLUDE;
use crate::config::files::validate_patterns;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LintConfiguration {
    include: Vec<String>,
    exclude: Vec<String>,
    pub(crate) rules: RulesSettings,
    #[serde(with = "whim_linter::settings::level")]
    pub(crate) minimum_fail_level: Level<'static>,
}

impl Default for LintConfiguration {
    fn default() -> Self {
        Self {
            include: DEFAULT_INCLUDE.map(str::to_owned).into(),
            exclude: Vec::new(),
            rules: RulesSettings::default(),
            minimum_fail_level: Level::ERROR,
        }
    }
}

impl LintConfiguration {
    pub(crate) fn registry(&self) -> Result<RuleRegistry, Error> {
        RuleRegistry::build(
            &Settings {
                rules: self.rules.clone(),
            },
            None,
            false,
        )
        .map_err(Error::InvalidLintSettings)
    }

    pub(crate) fn patterns(&self) -> Result<FilePatterns, Error> {
        FilePatterns::new("lint.include", &self.include, "lint.exclude", &self.exclude)
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        validate_patterns("lint.include", &self.include)?;
        validate_patterns("lint.exclude", &self.exclude)
    }
}
