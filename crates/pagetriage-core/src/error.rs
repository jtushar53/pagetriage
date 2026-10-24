use std::fmt;

/// Every fallible core entry point returns this. The variants map to what the caller got wrong, so
/// the CLI, Python and WASM bindings can surface them without string matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Pixel buffers, dimensions or DPI are inconsistent or out of range.
    InvalidInput(String),
    /// `opts_json` is not valid JSON, has unknown keys, or names an unknown detector.
    InvalidOptions(String),
    /// A params TOML (embedded default or runtime override) failed to parse or validate.
    InvalidParams(String),
    /// A detector produced a non-finite number. This is a bug in the detector, never in the input.
    NonFinite(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidInput(m) => write!(f, "invalid input: {m}"),
            Error::InvalidOptions(m) => write!(f, "invalid options: {m}"),
            Error::InvalidParams(m) => write!(f, "invalid params: {m}"),
            Error::NonFinite(m) => write!(f, "non-finite value: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
