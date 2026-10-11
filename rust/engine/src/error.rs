//! Structured errors for Rust callers; Display retains the text ABI.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
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
        let text = match self {
            Self::InvalidRules(text)
            | Self::InvalidDevice(text)
            | Self::InvalidInput(text)
            | Self::Evaluation(text)
            | Self::InvalidValue(text) => text,
            Self::InvalidOverride(text) => return write!(f, "override: {text}"),
            Self::Unsupported(text) => return write!(f, "unsupported: {text}"),
        };
        f.write_str(text)
    }
}
impl std::error::Error for EngineError {}

impl From<String> for EngineError {
    fn from(value: String) -> Self {
        Self::Evaluation(value)
    }
}
impl From<&str> for EngineError {
    fn from(value: &str) -> Self {
        Self::Evaluation(value.into())
    }
}

impl EngineError {
    pub(crate) fn reject_code(&self) -> crate::RejectCode {
        match self {
            Self::Unsupported(_) => crate::RejectCode::Unsupported,
            _ => crate::RejectCode::InvalidValue,
        }
    }
}
