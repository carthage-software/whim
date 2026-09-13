use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::ops::ControlFlow;
use std::os::fd::AsFd;
use std::process::ExitCode;
use std::time::Duration;

use annotate_snippets::Level;
use rayon::prelude::*;
use tracing::Span;

use crate::error::Error;
use crate::filesystem;
use crate::output;
use crate::pipeline::Reducer;
use crate::pipeline::files::Target;
use crate::pipeline::timed;
use crate::service::FileError;
use crate::service::FileResult;
use crate::service::FileStatus;

#[derive(Default)]
pub(super) struct DiagnosticCounts {
    errors: usize,
    warnings: usize,
    info: usize,
    notes: usize,
    help: usize,
}

impl DiagnosticCounts {
    pub(super) fn record(&mut self, level: &Level<'static>) {
        if *level == Level::ERROR {
            self.errors += 1;
        } else if *level == Level::WARNING {
            self.warnings += 1;
        } else if *level == Level::INFO {
            self.info += 1;
        } else if *level == Level::NOTE {
            self.notes += 1;
        } else {
            self.help += 1;
        }
    }
}

#[derive(Default)]
pub(crate) struct RunSummary {
    processed: usize,
    clean: usize,
    changed: usize,
    file_errors: usize,
    diagnostics: DiagnosticCounts,
    failed: bool,
    output_closed: bool,
}

impl RunSummary {
    pub(crate) fn report(&self, elapsed: Duration) -> ExitCode {
        if self.output_closed {
            tracing::debug!(
                processed = self.processed,
                ?elapsed,
                "stopped because standard output closed"
            );
            return ExitCode::SUCCESS;
        }

        tracing::info!(
            processed = self.processed,
            clean = self.clean,
            changed = self.changed,
            file_errors = self.file_errors,
            errors = self.diagnostics.errors,
            warnings = self.diagnostics.warnings,
            info = self.diagnostics.info,
            notes = self.diagnostics.notes,
            help = self.diagnostics.help,
            failed = self.failed,
            ?elapsed,
            "finished processing files",
        );

        if self.failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

pub(super) struct OutputReducer {
    output: BufWriter<File>,
    summary: RunSummary,
}

impl OutputReducer {
    pub(super) fn new() -> Result<Self, Error> {
        let descriptor = io::stdout()
            .as_fd()
            .try_clone_to_owned()
            .map_err(Error::WriteOutput)?;
        Ok(Self {
            output: BufWriter::new(File::from(descriptor)),
            summary: RunSummary::default(),
        })
    }

    fn written(&mut self, result: io::Result<()>) -> Result<ControlFlow<()>, Error> {
        match result {
            Ok(()) => Ok(ControlFlow::Continue(())),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                self.summary.output_closed = true;
                Ok(ControlFlow::Break(()))
            }
            Err(error) => Err(Error::WriteOutput(error)),
        }
    }
    fn report(&mut self, target: &Target, result: FileResult) -> Result<ControlFlow<()>, Error> {
        self.summary.processed += 1;
        match result {
            Ok(FileStatus::Unchanged) => self.summary.clean += 1,
            Ok(FileStatus::Diagnostics {
                text,
                counts,
                failed,
            }) => {
                self.summary.diagnostics.errors += counts.errors;
                self.summary.diagnostics.warnings += counts.warnings;
                self.summary.diagnostics.info += counts.info;
                self.summary.diagnostics.notes += counts.notes;
                self.summary.diagnostics.help += counts.help;
                self.summary.failed |= failed;
                let result = self.output.write_all(text.as_bytes());
                return self.written(result);
            }
            Ok(FileStatus::Differs(diff)) => {
                self.summary.changed += 1;
                self.summary.failed = true;
                let result = self.output.write_all(diff.as_bytes());
                return self.written(result);
            }
            Ok(FileStatus::Changed(_)) => {
                self.summary.changed += 1;
                tracing::debug!(file = %target.spelling.display(), "formatted file");
            }
            Err(error) => {
                self.summary.file_errors += 1;
                self.summary.failed = true;
                match error {
                    FileError::Read(error) => {
                        tracing::error!(file = %target.spelling.display(), phase = "read", %error, "could not process file");
                    }
                    FileError::Write(error) => {
                        tracing::error!(file = %target.spelling.display(), phase = "write", %error, "could not process file");
                    }
                    FileError::Syntax(diagnostic) => {
                        tracing::debug!(file = %target.spelling.display(), phase = "parse", "could not parse file");
                        if tracing::enabled!(tracing::Level::ERROR) {
                            output::write_error(&diagnostic)?;
                        }
                    }
                }
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

impl Reducer<FileResult> for OutputReducer {
    type Output = RunSummary;

    fn reduce(
        &mut self,
        targets: &[Target],
        mut results: Vec<FileResult>,
    ) -> Result<ControlFlow<()>, Error> {
        if results
            .iter()
            .any(|result| matches!(result, Ok(FileStatus::Changed(_))))
        {
            let parent = Span::current();
            timed("apply", || {
                results
                    .par_iter_mut()
                    .zip(targets)
                    .for_each(|(result, target)| {
                        if let Ok(FileStatus::Changed(contents)) = result {
                            let written = parent.in_scope(|| {
                                let file_span =
                                    tracing::trace_span!("file", path = %target.spelling.display());
                                let _entered = file_span.enter();
                                timed("write", || filesystem::replace(&target.path, contents))
                            });
                            if let Err(error) = written {
                                *result = Err(FileError::Write(error));
                            }
                        }
                    });
            });
        }
        for (target, result) in targets.iter().zip(results) {
            let file_span = tracing::trace_span!("file", path = %target.spelling.display());
            let _entered = file_span.enter();
            if self.report(target, result)?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    fn finish(mut self) -> Result<Self::Output, Error> {
        let result = if self.summary.output_closed {
            Ok(ControlFlow::Break(()))
        } else {
            let result = self.output.flush();
            self.written(result)
        };
        let _ = self.output.into_parts();
        let _ = result?;
        Ok(self.summary)
    }
}
