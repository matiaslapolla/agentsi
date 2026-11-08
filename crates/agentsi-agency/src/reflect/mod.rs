//! Reflection (U25): `agentsi reflect`, `reflect revert`, the replay of ADR 0001 and
//! the settling of a reflection decision. A port of `reflect.ts`, `reflections.ts`,
//! `replay.ts`, `proposal.ts` and `reflection-decision.ts` of `packages/core`.
//!
//! A model proposes memory changes only as deltas, and plain code validates and
//! applies them. A prompt change is never applied here: replay judges it, and the
//! owner approves it with `decide`.

mod curate;
mod decision;
mod diff;
pub mod memory;
mod proposal;
mod replay;
mod run;
mod signals;

pub use decision::{RoleFile, Settled, settle};
use diff::unified_diff;
pub use run::{Env, Options, ReflectOutcome, reflect, revert};

use agentsi_proto::reflect_api::ReflectionEntry;
use serde_json::Value;
use std::fmt;

/// The synthesis is written by `pm`, so its failures are charged to that agent.
pub const SYNTHESIS_AGENT: &str = "pm";

#[derive(Debug)]
pub enum Error {
    Agency(crate::Error),
    /// The reflection or the item does not exist.
    NotFound(String),
    /// The reflection is in another state, or the owner canceled it.
    Conflict(String),
    /// A model call failed, so nothing was written.
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Agency(e) => e.fmt(f),
            Error::NotFound(m) | Error::Conflict(m) | Error::Failed(m) => f.write_str(m),
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

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Agency(e.into())
    }
}

/// One stored entry as its type. A stored line that does not fit reads as `None`.
fn typed_entry(line: &Value) -> Option<ReflectionEntry> {
    serde_json::from_value(line.clone()).ok()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}
