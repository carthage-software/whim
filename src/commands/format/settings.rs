use whim_formatter::settings::FormatSettings;

use crate::commands::format::Arguments;
use crate::config::Configuration;
use crate::error::Error;

pub(super) fn resolve(
    arguments: &Arguments,
    configuration: &Configuration,
) -> Result<FormatSettings, Error> {
    let mut settings = configuration.format().settings();

    if let Some(print_width) = arguments.print_width {
        settings.print_width = print_width;
    }
    if let Some(tab_width) = arguments.tab_width {
        settings.tab_width = tab_width;
    }
    if let Some(use_tabs) = arguments.use_tabs {
        settings.use_tabs = use_tabs;
    }
    if let Some(end_of_line) = arguments.end_of_line {
        settings.end_of_line = end_of_line.into();
    }

    settings.validate().map_err(Error::InvalidFormatSettings)?;
    Ok(settings)
}
