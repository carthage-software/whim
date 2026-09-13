use std::fs;
use std::io;

use whim_syn::error::ParseError;

use crate::filesystem;
use crate::pipeline::files::Target;
use crate::pipeline::timed;

pub(crate) mod format;
mod json;
pub(crate) mod lint;
mod output;

use output::DiagnosticCounts;
pub(crate) use output::RunSummary;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum OutputFormat {
    Text,
    Json,
}

enum FileStatus {
    Unchanged,
    Diagnostics {
        text: String,
        counts: DiagnosticCounts,
        failed: bool,
    },
    Changed(String),
    Differs(String),
}

enum FileError {
    Read(io::Error),
    Syntax(String),
    Write(filesystem::Error),
}

type FileResult = Result<FileStatus, FileError>;

fn read(target: &Target) -> Result<String, FileError> {
    timed("read", || fs::read_to_string(&target.path)).map_err(FileError::Read)
}

fn syntax(error: ParseError, source: &str, target: &Target, color: bool) -> FileError {
    FileError::Syntax(timed("render", || {
        error.render_with_color(source, &target.spelling.to_string_lossy(), color)
    }))
}
