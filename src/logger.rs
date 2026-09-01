use std::env;
use std::io::stderr;
use std::str::FromStr;

use clap::ColorChoice;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::Directive;
use tracing_subscriber::fmt;

use crate::color::should_use_colors;

pub(crate) fn initialize(
    directive: impl Into<Directive>,
    environment: impl Into<String>,
    colors: ColorChoice,
) {
    let environment = environment.into();
    let configured = env::var_os(&environment).is_some();
    let mut filter = EnvFilter::builder()
        .with_default_directive(directive.into())
        .with_env_var(environment)
        .from_env_lossy();

    if !configured && let Ok(directive) = Directive::from_str("pubgrub=warn") {
        filter = filter.add_directive(directive);
    }

    let logger = fmt()
        .with_env_filter(filter)
        .with_ansi(should_use_colors(colors))
        .with_writer(stderr);

    if cfg!(debug_assertions) {
        logger.with_target(true).with_thread_names(true).init();
    } else {
        logger
            .without_time()
            .with_target(false)
            .with_thread_names(false)
            .compact()
            .init();
    }
}
