use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use clap::ColorChoice;

use whim_runtime::disassembly::Disassembly;
use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;
use whim_syn::arena::LocalArena;
use whim_syn::diagnostic;
use whim_syn::parser;

use crate::color::should_use_colors;
use crate::engine;
use crate::error::Error;
use crate::output;
use crate::source::Source;

#[derive(Args)]
pub(super) struct Arguments {
    /// The source file to disassemble, or `-` for standard input.
    #[arg(required = true, value_name = "FILE")]
    file: PathBuf,
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(file = %arguments.file.display(), optimize = configuration.optimize),
)]
pub(super) fn execute(
    arguments: Arguments,
    configuration: EngineConfiguration,
    colors: ColorChoice,
) -> Result<ExitCode, Error> {
    let source = Source::read(arguments.file)?;
    let mut engine = engine::create(configuration, colors)?;

    disassemble(
        &mut engine,
        &source.text,
        &source.path,
        should_use_colors(colors),
    )
}

fn disassemble(
    engine: &mut Engine,
    source: &str,
    path: &Path,
    color: bool,
) -> Result<ExitCode, Error> {
    let diagnostic_path = path.to_string_lossy();
    let arena = LocalArena::new();
    let program = match parser::parse(&arena, source) {
        Ok(program) => program,
        Err(errors) => {
            report_diagnostic(&errors.render_with_color(source, &diagnostic_path, color))?;
            return Ok(ExitCode::from(255));
        }
    };

    let disassembly = match Disassembly::compile(engine, program, path) {
        Ok(disassembly) => disassembly,
        Err(error) => {
            report_diagnostic(&diagnostic::render_with_color(
                source,
                &diagnostic_path,
                &error.labels(),
                color,
            ))?;

            return Ok(ExitCode::from(255));
        }
    };

    let descriptor = io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .map_err(Error::WriteDisassembly)?;
    let mut output = BufWriter::new(File::from(descriptor));
    match disassembly
        .write_to(&mut output)
        .and_then(|()| output.flush())
    {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(ExitCode::SUCCESS),
        Err(error) => Err(Error::WriteDisassembly(error)),
    }
}

fn report_diagnostic(diagnostic: &str) -> Result<(), Error> {
    if tracing::enabled!(tracing::Level::ERROR) {
        output::write_error(diagnostic)?;
    }

    Ok(())
}
