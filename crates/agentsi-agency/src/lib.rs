//! The agency: a port of `packages/core` of the TypeScript code, on the SQLite
//! store of `agentsi-store`. U16 ports the registry, `whoami`, `status`,
//! `snapshot` and `audit`. Each function takes the connection that it reads
//! from and the registry, so a test can run it on a fixed database.

pub mod archive;
pub mod audit;
pub mod call;
pub mod clock;
pub mod error;
pub mod git_env;
pub mod gitpanel;
pub mod knowledge;
pub mod marks;
pub mod meeting;
pub mod prompt_inputs;
pub mod reflect;
pub mod registry;
pub mod roles;
pub mod roster;
pub mod sandbox;
pub mod snapshot;
pub mod status;
pub mod story;

pub use error::{Error, Result};
