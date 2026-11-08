//! The errors of the agency. The core maps them to the error kinds of the protocol.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// The product or the id does not exist.
    NotFound(String),
    /// The registry or the store cannot be read.
    Unavailable(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(m) | Error::Unavailable(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<agentsi_store::Error> for Error {
    fn from(e: agentsi_store::Error) -> Error {
        Error::Unavailable(e.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Unavailable(e.to_string())
    }
}
