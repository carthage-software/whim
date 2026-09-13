use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::ArgAction;
use clap::Args;
use clap::ColorChoice;

use clap::ValueEnum;
use whim_formatter::settings::EndOfLine;
use whim_formatter::settings::FormatSettings;

use crate::color::should_use_colors;
use crate::config::Configuration;
use crate::error::Error;
use crate::pipeline::files::Selection;
use crate::service::format::FormatService;

#[derive(Args)]
pub(super) struct Arguments {
    /// Set the maximum line width.
    #[arg(long, value_name = "N")]
    print_width: Option<usize>,

    /// Set the indent width.
    #[arg(long, visible_alias = "tab-size", value_name = "N")]
    tab_width: Option<usize>,

    /// Use tabs for indentation.
    #[arg(long, value_name = "BOOL", action = ArgAction::Set)]
    use_tabs: Option<bool>,

    /// Set the line ending.
    #[arg(long, value_enum, value_name = "EOL")]
    end_of_line: Option<EndOfLineArgument>,

    /// Report unformatted files without changing them.
    #[arg(long)]
    check: bool,

    /// Files and directories to format. Omit to format the project.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum EndOfLineArgument {
    Lf,
    Crlf,
}

impl From<EndOfLineArgument> for EndOfLine {
    fn from(value: EndOfLineArgument) -> Self {
        match value {
            EndOfLineArgument::Lf => Self::Lf,
            EndOfLineArgument::Crlf => Self::Crlf,
        }
    }
}

#[tracing::instrument(
    name = "fmt",
    level = "info",
    skip_all,
    fields(root = %configuration.root().display(), check = arguments.check, targets = arguments.paths.len()),
    err(level = "debug"),
)]
pub(super) fn execute(
    arguments: &Arguments,
    configuration: &Configuration,
    colors: ColorChoice,
) -> Result<ExitCode, Error> {
    let start = Instant::now();
    let settings = resolve(arguments, configuration)?;
    tracing::debug!(?settings, "resolved format settings");
    let patterns = configuration.format().patterns()?;
    let selection = Selection::new(&arguments.paths, configuration, &patterns)?;
    tracing::info!(files = selection.targets.len(), "formatting source files");
    let summary = FormatService::new(settings, arguments.check, should_use_colors(colors))
        .run(&selection.targets)?;
    Ok(summary.report(start.elapsed()))
}

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
