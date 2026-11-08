//! The roles (U21): the roster, the overlays, the specialists, and the
//! `apply` and `reset` that write them. Ports `roles.ts`, `overlays.ts`,
//! `roster.ts`, `roster-apply.ts`, `reset.ts` and `registry-edit.ts` of
//! `packages/core`. `validate` is `roster::validate_roster` of U16.

pub mod apply;
pub mod commit;
pub mod files;
pub mod load;
pub mod registry_edit;
pub mod reset;

use crate::error::Error as AgencyError;

/// Why a roles operation failed. `Invalid` is a message for the user: a bad
/// stage, an unknown agent or a bad option.
#[derive(Debug)]
pub enum Error {
    Invalid(String),
    Agency(AgencyError),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<AgencyError> for Error {
    fn from(e: AgencyError) -> Error {
        Error::Agency(e)
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

pub(crate) fn invalid<T>(text: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(text.into()))
}
