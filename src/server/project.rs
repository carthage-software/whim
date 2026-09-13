use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use lsp_types::Uri;
use whim_formatter::settings::FormatSettings;
use whim_linter::registry::RuleRegistry;

use crate::config::Configuration;
use crate::config::Error;
use crate::config::FilePatterns;
use crate::config::FormatConfiguration;
use crate::config::LintConfiguration;
use crate::server::diagnostics::file_path;

pub(super) struct Project {
    pub(super) root: PathBuf,
    pub(super) has_manifest: bool,
    pub(super) registry: Arc<RuleRegistry>,
    pub(super) lint_patterns: FilePatterns,
    pub(super) format: FormatSettings,
    format_patterns: FilePatterns,
}

impl Project {
    pub(super) fn load(root: &Path, path: Option<&Path>) -> Result<Self, Error> {
        let configuration = Configuration::load_from(root, path)?;
        Self::new(
            configuration.root().to_owned(),
            configuration.manifest().is_ok(),
            configuration.lint(),
            configuration.format(),
        )
    }

    pub(super) fn standalone() -> Result<Self, Error> {
        Self::new(
            PathBuf::new(),
            false,
            &LintConfiguration::default(),
            &FormatConfiguration::default(),
        )
    }

    fn new(
        root: PathBuf,
        has_manifest: bool,
        lint: &LintConfiguration,
        format: &FormatConfiguration,
    ) -> Result<Self, Error> {
        Ok(Self {
            root,
            has_manifest,
            registry: Arc::new(lint.registry()?),
            lint_patterns: lint.patterns()?,
            format: format.settings(),
            format_patterns: format.patterns()?,
        })
    }

    pub(super) fn path(&self, uri: &Uri) -> Option<PathBuf> {
        let path = file_path(uri)?;
        Some(path.strip_prefix(&self.root).unwrap_or(&path).to_owned())
    }

    pub(super) fn can_format(&self, uri: &Uri) -> bool {
        self.path(uri).is_none_or(|path| {
            self.format_patterns.includes(&path) && !self.format_patterns.excludes(&path)
        })
    }
}
