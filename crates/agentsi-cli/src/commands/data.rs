//! `agentsi data import <dir>` is gone. `agentsi-core import-postgres` took its place.

use super::Fail;

pub const USAGE: &str = "agentsi data import <dir>";

pub fn run(_: &[String]) -> Result<i32, Fail> {
    Err(Fail::Error(
        "Removed. Use `agentsi-core import-postgres` to move data of an old data root.".into(),
    ))
}
