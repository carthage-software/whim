use std::env;
use std::io;
use std::io::IsTerminal;

use clap::ColorChoice;

pub(crate) fn should_use_colors(choice: ColorChoice) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => environment_color_choice().map_or_else(
            || io::stderr().is_terminal(),
            |choice| matches!(choice, ColorChoice::Always),
        ),
    }
}

pub(crate) fn environment_color_choice() -> Option<ColorChoice> {
    if let Some(force) = env::var_os("FORCE_COLOR")
        && !force.is_empty()
    {
        return Some(if force == "0" {
            ColorChoice::Never
        } else {
            ColorChoice::Always
        });
    }

    if env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return Some(ColorChoice::Never);
    }

    None
}
