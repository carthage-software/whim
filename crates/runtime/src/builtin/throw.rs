//! The thrown-error carrier built-in handlers surface failure through.

use std::fmt;

use crate::value::Value;

pub(crate) struct Throw(pub Value);

impl fmt::Debug for Throw {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Throw")
            .field(&self.0.kind_name())
            .finish()
    }
}
