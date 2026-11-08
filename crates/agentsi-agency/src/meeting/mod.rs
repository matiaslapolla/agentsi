//! Meetings: `agentsi meet`, `ask` and `check`. U22 ports `meeting.ts`,
//! `meeting-log.ts`, `check.ts` and `claude.ts`. A role is one `claude -p` run.
//! The functions block, so the core calls them off its event loop.

pub mod check;
pub mod claude;
pub mod log;
pub mod minutes;
pub mod prompts;
mod run;
pub mod running;

pub use claude::Cancel;
pub(crate) use run::product_cwd;
pub use run::{AskOutcome, Env, MeetingOptions, MeetingOutcome, Sink, ask_role, run_meeting};

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Agency(crate::Error),
    /// A role that does not resolve, or a product with no folder.
    Invalid(String),
    /// The product is archived, or the owner canceled the meeting.
    Conflict(String),
    /// The Claude run of `ask` failed.
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Agency(e) => e.fmt(f),
            Error::Invalid(m) | Error::Conflict(m) | Error::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::Error> for Error {
    fn from(e: crate::Error) -> Error {
        Error::Agency(e)
    }
}

impl From<agentsi_store::Error> for Error {
    fn from(e: agentsi_store::Error) -> Error {
        Error::Agency(e.into())
    }
}
