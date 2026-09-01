mod files;
mod settings;

use std::fs;
use std::io::Error as IoError;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ArgAction;
use clap::Args;
use clap::ColorChoice;
use clap::ValueEnum;
use rayon::prelude::*;
use similar::TextDiff;
use thiserror::Error as ThisError;
use whim_formatter::settings::EndOfLine;
use whim_formatter::settings::FormatSettings;
use whim_syn::arena::Arena;
use whim_syn::arena::LocalArena;

use crate::color::should_use_colors;
use crate::commands::format::files::Target;
use crate::commands::format::files::discover;
use crate::commands::format::settings::resolve;
use crate::config::Configuration;
use crate::error::Error;
use crate::filesystem;
use crate::output;

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

const REPORTING_BATCH: usize = 64;
const MAXIMUM_DIFF_LINES: usize = 1_000;

enum Outcome {
    Unchanged,
    Rewritten,
    Differs(String),
    Failed(FileError),
}

#[derive(Debug, ThisError)]
enum FileError {
    #[error("could not read: {0}")]
    Read(#[source] IoError),
    #[error("{0}")]
    Syntax(String),
    #[error("could not write: {0}")]
    Write(#[source] filesystem::Error),
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(check = arguments.check, targets = arguments.paths.len()),
)]
pub(super) fn execute(
    arguments: &Arguments,
    configuration: &Configuration,
    colors: ColorChoice,
) -> Result<ExitCode, Error> {
    let settings = resolve(arguments, configuration)?;
    let patterns = configuration.format().patterns()?;
    let root = if arguments.paths.is_empty() {
        configuration.project_root()?
    } else {
        configuration.root()
    };
    let targets = discover(&arguments.paths, root, &patterns)?;
    let color = should_use_colors(colors);
    tracing::debug!(files = targets.len(), "discovered format targets");

    let mut any_failed = false;
    let mut any_unformatted = false;
    for batch in targets.chunks(REPORTING_BATCH) {
        let outcomes: Vec<(&Path, Outcome)> = batch
            .par_iter()
            .map_init(LocalArena::new, |arena, target| {
                arena.reset();
                (
                    target.spelling.as_path(),
                    format_file(arena, target, settings, arguments.check, color),
                )
            })
            .collect();

        for (path, outcome) in &outcomes {
            match outcome {
                Outcome::Unchanged | Outcome::Rewritten => {}
                Outcome::Differs(diff) => {
                    any_unformatted = true;

                    if !output::write(diff.as_str())? {
                        return Ok(ExitCode::SUCCESS);
                    }
                }
                Outcome::Failed(error) => {
                    any_failed = true;
                    tracing::debug!(file = %path.display(), error = ?error, "could not format file");
                    if let FileError::Syntax(diagnostic) = error {
                        if tracing::enabled!(tracing::Level::ERROR) {
                            output::write_error(diagnostic)?;
                        }
                    } else {
                        tracing::error!("{}: {error}", path.display());
                    }
                }
            }
        }
    }

    if any_failed || (arguments.check && any_unformatted) {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn format_file<A: Arena>(
    arena: &A,
    target: &Target,
    settings: FormatSettings,
    check: bool,
    color: bool,
) -> Outcome {
    let source = match fs::read_to_string(&target.path) {
        Ok(source) => source,
        Err(error) => return Outcome::Failed(FileError::Read(error)),
    };

    let formatted = match whim_formatter::format(arena, &source, settings) {
        Ok(formatted) => formatted,
        Err(errors) => {
            return Outcome::Failed(FileError::Syntax(errors.render_with_color(
                &source,
                &target.spelling.display().to_string(),
                color,
            )));
        }
    };

    if formatted == source.as_str() {
        return Outcome::Unchanged;
    }

    if check {
        let name = target.spelling.display().to_string();
        let diff = TextDiff::from_lines(source.as_str(), formatted)
            .unified_diff()
            .header(&name, &name)
            .to_string();
        return Outcome::Differs(truncate_diff(diff));
    }

    match filesystem::replace(&target.path, formatted) {
        Ok(()) => Outcome::Rewritten,
        Err(error) => Outcome::Failed(FileError::Write(error)),
    }
}

fn truncate_diff(diff: String) -> String {
    let mut boundary = None;
    for (count, (offset, _)) in diff.match_indices('\n').enumerate() {
        if count + 1 == MAXIMUM_DIFF_LINES {
            boundary = Some(offset + 1);
            break;
        }
    }

    let Some(boundary) = boundary else {
        return diff;
    };

    let remaining = diff[boundary..].lines().count();
    if remaining == 0 {
        return diff;
    }

    let mut truncated = diff;
    truncated.truncate(boundary);
    truncated.push_str("... ");
    truncated.push_str(&remaining.to_string());
    truncated.push_str(" more line");
    if remaining != 1 {
        truncated.push('s');
    }
    truncated.push_str(" not shown\n");
    truncated
}
