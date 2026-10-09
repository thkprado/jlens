use std::fmt;

use serde_json::Value as JsonValue;

/// Result type used throughout jlens-core.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error raised while compiling or evaluating a JSONata expression.
///
/// Every error carries a stable JSONata error code (for example `S0201` for a syntax error or
/// `T2001` for a type error), so callers can match on codes rather than on messages.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub struct Error(Box<ErrorInner>);

#[derive(Debug, Clone, PartialEq)]
struct ErrorInner {
    code: String,
    message: String,
    position: Option<usize>,
    token: Option<String>,
    value: Option<JsonValue>,
}

impl Error {
    /// Code reported for features that are not implemented yet.
    pub const UNIMPLEMENTED: &'static str = "X0000";

    pub(crate) fn unimplemented(what: &str) -> Self {
        Error(Box::new(ErrorInner {
            code: Self::UNIMPLEMENTED.to_owned(),
            message: format!("{what} is not implemented yet"),
            position: None,
            token: None,
            value: None,
        }))
    }

    /// The JSONata error code, such as `S0201`.
    pub fn code(&self) -> &str {
        &self.0.code
    }

    /// The human-readable message.
    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// Character offset in the expression source where the error was detected, if known.
    pub fn position(&self) -> Option<usize> {
        self.0.position
    }

    /// The token involved in the error, if any.
    pub fn token(&self) -> Option<&str> {
        self.0.token.as_deref()
    }

    /// The value involved in the error, if any.
    pub fn value(&self) -> Option<&JsonValue> {
        self.0.value.as_ref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.0.code, self.0.message)?;
        if let Some(pos) = self.0.position {
            write!(f, " (at position {pos})")?;
        }
        Ok(())
    }
}
