use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use clap::ColorChoice;
use tracing::field;

use whim_runtime::engine::EngineConfiguration;
use whim_runtime::path::path_bytes;

use crate::engine;
use crate::error::Error;
use crate::source::Source;

#[derive(Args)]
pub(super) struct Arguments {
    #[arg(
        required = true,
        value_names = ["FILE", "ARGS"],
        num_args = 1..,
        trailing_var_arg = true,
        help = "The entry file, or `-` for standard input, followed by program arguments"
    )]
    invocation: Vec<OsString>,
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(
        file = field::Empty,
        optimize = configuration.optimize,
        full_trace = configuration.full_trace,
    ),
)]
pub(super) fn execute(
    arguments: Arguments,
    configuration: EngineConfiguration,
    colors: ColorChoice,
) -> Result<ExitCode, Error> {
    let mut invocation = arguments.invocation.into_iter();
    let Some(file) = invocation.next().map(PathBuf::from) else {
        return Err(Error::MissingEntryFile);
    };

    tracing::Span::current().record("file", field::display(file.display()));
    let source = Source::read(file)?;
    let mut engine = engine::create(configuration, colors)?;

    engine.set_argument_bytes(
        invocation
            .map(|argument| path_bytes(Path::new(&argument)))
            .collect(),
    );

    engine.set_script_bytes(Some(path_bytes(&source.file)));
    let outcome = engine.run_source(&source.text, &source.path);
    if let Some(error) = engine.take_output_failure() {
        if error.kind() == io::ErrorKind::BrokenPipe {
            return Ok(ExitCode::SUCCESS);
        }

        return Err(Error::WriteOutput(error));
    }

    Ok(ExitCode::from(outcome.exit_code()))
}
