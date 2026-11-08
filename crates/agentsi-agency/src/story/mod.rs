//! Stories mode (U23): a port of `story.ts`, `story-write.ts`,
//! `story-log.ts`, `story-propose.ts` and `story-run.ts` of `packages/core`.
//! The stories and their logs are in the store. A story session writes code
//! in a worktree of the product repo, and the runner runs its tests and gates
//! in the sandbox of `crate::sandbox` (ADR 0002).

pub mod claude;
pub mod code_graph;
pub mod config;
pub mod git;
pub mod inputs;
pub mod lifecycle;
pub mod propose;
pub mod record;
pub mod run;
pub mod store;

use crate::error::Error as AgencyError;
use std::fmt;

/// Why a story operation failed. `Invalid` and `NotFound` are messages for
/// the user, as the TypeScript code throws them.
#[derive(Debug)]
pub enum Error {
    Invalid(String),
    NotFound(String),
    /// The session was rejected while it ran. It never reaches a client.
    Gone(String),
    Agency(AgencyError),
}

impl Error {
    pub fn invalid(text: impl Into<String>) -> Error {
        Error::Invalid(text.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(m) | Error::NotFound(m) | Error::Gone(m) => f.write_str(m),
            Error::Agency(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<AgencyError> for Error {
    fn from(e: AgencyError) -> Error {
        match e {
            AgencyError::NotFound(m) => Error::NotFound(m),
            other => Error::Agency(other),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Agency(e.into())
    }
}

impl From<agentsi_store::Error> for Error {
    fn from(e: agentsi_store::Error) -> Error {
        Error::Agency(e.into())
    }
}
