use std::error::Error as StdError;
use std::fmt;
use std::io;

use rusqlite::Error as SQLiteError;

/// A connection or query error.
#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    code: i32,
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ErrorKind {
    Other,
    SQLite,
    ConcurrentOperation,
}

impl Error {
    /// Creates an error without a database code.
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Other,
            code: 0,
            message: message.into(),
        }
    }

    pub(crate) fn concurrent_operation() -> Self {
        Self {
            kind: ErrorKind::ConcurrentOperation,
            code: 0,
            message: "the previous SQLite operation has not finished".to_string(),
        }
    }

    /// Returns the extended database error code, or zero when unavailable.
    #[must_use]
    pub const fn code(&self) -> i32 {
        self.code
    }

    /// Returns the error message.
    #[must_use]
    pub fn message_text(&self) -> &str {
        &self.message
    }

    /// Reports a rejected overlapping operation.
    #[must_use]
    pub fn is_concurrent_operation(&self) -> bool {
        self.kind == ErrorKind::ConcurrentOperation
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for Error {}

impl From<SQLiteError> for Error {
    fn from(error: SQLiteError) -> Self {
        let code = error
            .sqlite_error()
            .map_or(0, |details| details.extended_code);
        Self {
            kind: ErrorKind::SQLite,
            code,
            message: error.to_string(),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::message(error.to_string())
    }
}
