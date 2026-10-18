//! Structured errors for Rust callers; Display retains the text ABI.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineError {
    InvalidRules(String),
    InvalidDevice(String),
    InvalidOverride(String),
    InvalidInput(String),
    Evaluation(String),
    Unsupported(String),
    InvalidValue(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOverride(text) => write!(f, "override: {text}"),
            Self::Unsupported(text) => write!(f, "unsupported: {text}"),
            _ => f.write_str(self.message()),
        }
    }
}
impl std::error::Error for EngineError {}

impl EngineError {
    pub(crate) fn reject_code(&self) -> crate::RejectCode {
        match self {
            Self::Unsupported(_) => crate::RejectCode::Unsupported,
            _ => crate::RejectCode::InvalidValue,
        }
    }
}

impl EngineError {
    /// The error text without its category prefix.
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidRules(text)
            | Self::InvalidDevice(text)
            | Self::InvalidOverride(text)
            | Self::InvalidInput(text)
            | Self::Evaluation(text)
            | Self::Unsupported(text)
            | Self::InvalidValue(text) => text,
        }
    }
    pub(crate) fn eval(message: impl Into<String>) -> Self {
        Self::Evaluation(message.into())
    }
}

/// Lifts the crate's internal text errors into [`EngineError::Evaluation`].
pub(crate) trait OrEval<T> {
    fn or_eval(self) -> Result<T, EngineError>;
}
impl<T, E: Into<String>> OrEval<T> for Result<T, E> {
    fn or_eval(self) -> Result<T, EngineError> {
        self.map_err(EngineError::eval)
    }
}
