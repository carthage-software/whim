use std::process::ExitCode;

use crate::config::Configuration;
use crate::error::Error;
use crate::server;

pub(super) fn execute(configuration: &Configuration) -> Result<ExitCode, Error> {
    server::serve(configuration.format().settings())?;

    Ok(ExitCode::SUCCESS)
}
