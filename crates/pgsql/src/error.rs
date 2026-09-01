use std::error::Error as StdError;
use std::fmt;

/// A connection or query error.
#[derive(Clone, Debug)]
pub struct Error {
    pub(crate) message: String,
    pub(crate) sqlstate: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) hint: Option<String>,
}

impl Error {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            sqlstate: None,
            detail: None,
            hint: None,
        }
    }

    /// Returns the primary error message.
    #[must_use]
    pub fn message_text(&self) -> &str {
        &self.message
    }

    /// Returns the five-character SQLSTATE code, when available.
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        self.sqlstate.as_deref()
    }

    /// Returns the server's error detail, when available.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Returns the server's recovery hint, when available.
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for Error {}
