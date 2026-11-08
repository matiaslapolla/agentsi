//! SQLite schema, migrations, stores, Postgres import.
//! Spec: `docs/specs/agentsi-desktop-data.md`. ADR 0006.

pub mod backup;
pub mod calls;
pub mod clear;
pub mod db;
pub mod error;
pub mod import;
pub mod meeting;
pub mod paths;
pub mod rank;
pub mod read;
pub mod reflection;
pub mod schema;
pub mod search;
pub mod stories;

pub use db::Store;
pub use error::{Error, Result};
