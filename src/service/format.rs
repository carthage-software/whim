use similar::TextDiff;

use whim_formatter::settings::FormatSettings;
use whim_syn::arena::LocalArena;

use crate::error::Error;
use crate::pipeline::StatelessParallelPipeline;
use crate::pipeline::files::Target;
use crate::pipeline::timed;
use crate::service::FileResult;
use crate::service::FileStatus;
use crate::service::OutputFormat;
use crate::service::RunSummary;
use crate::service::output::OutputReducer;
use crate::service::read;
use crate::service::syntax;

const MAXIMUM_DIFF_LINES: usize = 1_000;

pub(crate) struct FormatService {
    settings: FormatSettings,
    check: bool,
    color: bool,
}

impl FormatService {
    pub(crate) const fn new(settings: FormatSettings, check: bool, color: bool) -> Self {
        Self {
            settings,
            check,
            color,
        }
    }

    pub(crate) fn run(&self, targets: &[Target]) -> Result<RunSummary, Error> {
        StatelessParallelPipeline::new(targets, self).run(
            |service, arena, target| service.process(arena, target),
            OutputReducer::new(OutputFormat::Text)?,
        )
    }

    fn process(&self, arena: &LocalArena, target: &Target) -> FileResult {
        let source = read(target)?;
        let formatted = timed("format", || {
            whim_formatter::format(arena, &source, self.settings)
        })
        .map_err(|error| syntax(error, &source, target, self.color))?;

        if formatted == source {
            return Ok(FileStatus::Unchanged);
        }

        if self.check {
            let name = target.spelling.to_string_lossy();
            let diff = timed("diff", || {
                TextDiff::from_lines(&source, formatted)
                    .unified_diff()
                    .header(&name, &name)
                    .to_string()
            });

            return Ok(FileStatus::Differs(truncate_diff(diff)));
        }

        Ok(FileStatus::Changed(formatted.to_owned()))
    }
}

fn truncate_diff(diff: String) -> String {
    let Some((offset, _)) = diff.match_indices('\n').nth(MAXIMUM_DIFF_LINES - 1) else {
        return diff;
    };
    let boundary = offset + 1;
    let remaining = diff[boundary..].lines().count();
    if remaining == 0 {
        return diff;
    }

    let mut truncated = diff;
    truncated.truncate(boundary);
    truncated.push_str("... ");
    truncated.push_str(&remaining.to_string());
    truncated.push_str(if remaining == 1 {
        " more line"
    } else {
        " more lines"
    });
    truncated.push_str(" not shown\n");
    truncated
}
