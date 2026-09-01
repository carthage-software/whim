//! An opinionated code formatter for the Whim language.

pub mod settings;

mod document;
mod format;
mod printer;

use whim_syn::arena::Arena;
use whim_syn::cst::Program;
use whim_syn::error::ParseError;
use whim_syn::parser;

use crate::format::FormatterState;
use crate::printer::Printer;
use crate::settings::FormatSettings;

/// Parses `source` and formats it, returning the formatted text.
///
/// # Errors
///
/// Returns the first syntax or structural-depth error.
pub fn format<'arena, A>(
    arena: &'arena A,
    source: &str,
    settings: FormatSettings,
) -> Result<&'arena str, ParseError>
where
    A: Arena,
{
    let program = parser::parse(arena, source)?;

    Ok(format_program(arena, program, settings))
}

/// Formats an already-parsed [`Program`].
#[must_use]
fn format_program<'arena, A>(
    arena: &'arena A,
    program: &'arena Program<'arena>,
    settings: FormatSettings,
) -> &'arena str
where
    A: Arena,
{
    let source_text = program.source_text;
    let mut state = FormatterState::new(arena, program, source_text);
    let document = state.format_program(program);

    Printer::new(arena, document, source_text.len(), settings).build()
}

#[cfg(test)]
mod tests;
