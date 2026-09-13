use std::process::ExitCode;

use crate::config::Configuration;
use crate::error::Error;
use crate::server;
use crate::server::diagnostics::Diagnostics;

pub(super) fn execute(configuration: &Configuration) -> Result<ExitCode, Error> {
    let diagnostics = Diagnostics::new(
        configuration.lint().registry()?,
        configuration.lint().patterns()?,
        configuration.root().to_owned(),
    );
    server::serve(configuration.format().settings(), diagnostics)?;

    Ok(ExitCode::SUCCESS)
}
