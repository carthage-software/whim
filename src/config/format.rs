use serde::Deserialize;
use whim_formatter::settings::EndOfLine;
use whim_formatter::settings::FormatSettings;

use crate::config::Error;
use crate::config::FilePatterns;
use crate::config::files::DEFAULT_INCLUDE;
use crate::config::files::validate_patterns;

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

    pub(crate) fn patterns(&self) -> Result<FilePatterns, Error> {
        FilePatterns::new(
            "format.include",
            &self.include,
            "format.exclude",
            &self.exclude,
        )
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.settings()
            .validate()
            .map_err(Error::InvalidFormatSettings)?;
        validate_patterns("format.include", &self.include)?;
        validate_patterns("format.exclude", &self.exclude)
    }
}
