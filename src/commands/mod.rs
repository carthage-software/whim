mod add;
mod disassemble;
mod format;
mod fund;
mod init;
mod install;
mod language_server;
mod remove;
mod run;
mod show;
mod suggestions;
mod update;
mod why;
mod why_not;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ColorChoice;
use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;
use clap::Subcommand;
use tracing_subscriber::filter::LevelFilter;

use crate::color::environment_color_choice;
use crate::config::Configuration;
use crate::error::Error;
use crate::logger;

const ABOUT: &str = "An experimental programming language built for exploration.";
const LONG_ABOUT: &str = indoc::indoc! {"
    Whim is an experimental programming language built for exploration. Every
    release may add, remove, or redesign any part of the language. It has no
    compatibility promise, release schedule, or production-support commitment.\
"};

#[derive(Parser)]
#[command(
    name = "whim",
    version,
    author,
    about = ABOUT,
    long_about = LONG_ABOUT,
    subcommand_negates_reqs = true,
    trailing_var_arg = true,
    override_usage = "whim [OPTIONS] <FILE> [ARGS]...\n       whim <COMMAND>",
    styles = crate::style::CLAP_STYLING
)]
struct Cli {
    /// When to use colored output.
    #[arg(long, default_value_t = ColorChoice::Auto, global = true)]
    colors: ColorChoice,

    /// Load settings from this `whim.toml` file.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    run: run::Arguments,
}

#[derive(Subcommand)]
enum Command {
    /// Run a Whim program.
    #[command(
        trailing_var_arg = true,
        override_usage = "whim run [OPTIONS] <FILE> [ARGS]..."
    )]
    Run(run::Arguments),

    /// Format Whim source files and directories.
    Fmt(format::Arguments),

    /// Print the bytecode of a Whim program.
    Disassemble(disassemble::Arguments),

    /// Create a Whim project.
    Init(init::Arguments),

    /// Add a Git dependency.
    Add(add::Arguments),

    /// Remove a direct Git dependency.
    Remove(remove::Arguments),

    /// Install the locked dependency graph.
    Install(install::Arguments),

    /// Resolve newer dependency releases.
    Update(update::Arguments),

    /// Explain why a Git dependency is installed.
    Why(why::Arguments),

    /// Explain why a Git dependency cannot be installed.
    WhyNot(why_not::Arguments),

    /// Show information about an installed Git dependency.
    Show(show::Arguments),

    /// Show packages suggested by the installed graph.
    Suggestions,

    /// Show ways to sponsor Whim and installed packages.
    Fund,

    /// Start the language server over standard input and output.
    LanguageServer,
}

impl Command {
    const fn name(&self) -> &'static str {
        match self {
            Self::Run(_) => "run",
            Self::Fmt(_) => "fmt",
            Self::Disassemble(_) => "disassemble",
            Self::Init(_) => "init",
            Self::Add(_) => "add",
            Self::Remove(_) => "remove",
            Self::Install(_) => "install",
            Self::Update(_) => "update",
            Self::Why(_) => "why",
            Self::WhyNot(_) => "why-not",
            Self::Show(_) => "show",
            Self::Suggestions => "suggestions",
            Self::Fund => "fund",
            Self::LanguageServer => "language-server",
        }
    }
}

pub(super) fn execute() -> Result<ExitCode, Error> {
    let mut matches = Cli::command().color(requested_colors()).get_matches();
    let arguments = Cli::from_arg_matches_mut(&mut matches).unwrap_or_else(|error| error.exit());
    logger::initialize(
        if cfg!(debug_assertions) {
            LevelFilter::DEBUG
        } else {
            LevelFilter::INFO
        },
        "WHIM_LOG",
        arguments.colors,
    );

    tracing::trace!(
        command = arguments.command.as_ref().map_or("run", Command::name),
        "parsed command line"
    );

    if let Some(Command::Init(command)) = &arguments.command {
        return init::execute(command).map(|()| ExitCode::SUCCESS);
    }

    let configuration = Configuration::load(arguments.config.as_deref())?;

    match arguments.command {
        Some(Command::Run(command)) => {
            run::execute(command, configuration.runtime()?, arguments.colors)
        }
        Some(Command::Fmt(command)) => format::execute(&command, &configuration, arguments.colors),
        Some(Command::Disassemble(command)) => {
            disassemble::execute(command, configuration.runtime()?, arguments.colors)
        }
        Some(Command::Add(command)) => {
            add::execute(command, &configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Remove(command)) => {
            remove::execute(&command, &configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Install(command)) => {
            install::execute(&command, &configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Update(command)) => {
            update::execute(&command, &configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Why(command)) => {
            why::execute(&command, &configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::WhyNot(command)) => {
            why_not::execute(&command, &configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Show(command)) => {
            show::execute(&command, &configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Suggestions) => {
            suggestions::execute(&configuration).map(|()| ExitCode::SUCCESS)
        }
        Some(Command::Fund) => fund::execute(&configuration).map(|()| ExitCode::SUCCESS),
        Some(Command::LanguageServer) => language_server::execute(&configuration),
        Some(Command::Init(_)) => {
            unreachable!("the init command is dispatched before configuration")
        }
        None => run::execute(arguments.run, configuration.runtime()?, arguments.colors),
    }
}

fn requested_colors() -> ColorChoice {
    let mut arguments = env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--colors" {
            return arguments
                .next()
                .and_then(|value| value.to_str()?.parse().ok())
                .unwrap_or_default();
        }

        if let Some(value) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix("--colors="))
        {
            return value.parse().unwrap_or_default();
        }
    }

    environment_color_choice().unwrap_or(ColorChoice::Auto)
}
