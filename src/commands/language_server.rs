use std::path::Path;
use std::path::absolute;
use std::process::ExitCode;

use crate::config::Error as ConfigurationError;
use crate::error::Error;
use crate::server;

pub(super) fn execute(path: Option<&Path>) -> Result<ExitCode, Error> {
    let path = path
        .map(absolute)
        .transpose()
        .map_err(ConfigurationError::CurrentDirectory)?;
    server::serve(path.as_deref())?;

    Ok(ExitCode::SUCCESS)
}
