//! Formatter configuration.

use core::error;
use core::fmt;

#[cfg(feature = "serde")]
use serde::Deserialize;
#[cfg(feature = "serde")]
use serde::Serialize;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Deserialize, Serialize),
    serde(default, deny_unknown_fields)
)]
pub struct FormatSettings {
    /// The column the formatter tries to keep lines within. Default `80`.
    pub print_width: usize,
    /// The number of columns one indentation level occupies. Default `2`.
    pub tab_width: usize,
    /// Indent with tab characters instead of spaces. Default `false`.
    pub use_tabs: bool,
    /// The line ending emitted between lines. Default [`EndOfLine::Lf`].
    pub end_of_line: EndOfLine,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Deserialize, Serialize),
    serde(rename_all = "lowercase")
)]
pub enum EndOfLine {
    Lf,
    Crlf,
}

impl EndOfLine {
    /// The byte sequence this line ending renders as.
    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }
}

impl Default for FormatSettings {
    #[inline]
    fn default() -> Self {
        Self {
            print_width: 80,
            tab_width: 2,
            use_tabs: false,
            end_of_line: EndOfLine::Lf,
        }
    }
}

/// Why a settings combination is not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError {
    /// The setting at fault.
    pub setting: &'static str,
    /// What is wrong with it, phrased for a person.
    pub reason: String,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.setting, self.reason)
    }
}

impl error::Error for SettingsError {}

const MAXIMUM_PRINT_WIDTH: usize = 10_000;

/// The widest one indentation level may be. Deep nesting at a large tab width
/// would exceed any print width before printing anything.
const MAXIMUM_TAB_WIDTH: usize = 256;

impl FormatSettings {
    /// Checks that these settings describe a layout the printer can produce.
    ///
    /// # Errors
    ///
    /// Returns the first setting that cannot produce a valid layout.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.print_width == 0 {
            return Err(SettingsError {
                setting: "print_width",
                reason: "must be at least 1; a width of zero leaves no room for any line"
                    .to_string(),
            });
        }
        if self.print_width > MAXIMUM_PRINT_WIDTH {
            return Err(SettingsError {
                setting: "print_width",
                reason: format!("must be at most {MAXIMUM_PRINT_WIDTH}"),
            });
        }
        if self.tab_width == 0 {
            return Err(SettingsError {
                setting: "tab_width",
                reason: "must be at least 1; a width of zero makes every indentation level \
                         invisible"
                    .to_string(),
            });
        }
        if self.tab_width > MAXIMUM_TAB_WIDTH {
            return Err(SettingsError {
                setting: "tab_width",
                reason: format!("must be at most {MAXIMUM_TAB_WIDTH}"),
            });
        }
        if self.tab_width > self.print_width {
            return Err(SettingsError {
                setting: "tab_width",
                reason: format!(
                    "must not exceed print_width ({}); one indentation level would overflow \
                     every line",
                    self.print_width
                ),
            });
        }
        Ok(())
    }
}

impl Default for EndOfLine {
    #[inline]
    fn default() -> Self {
        Self::Lf
    }
}
