//! `agentsi backup [--keep <n>]`: a SQLite online backup of the database and a copy of
//! the registry, both into `<data root>/backups/`. The core makes the copy with
//! `data.backup`, so the file is complete while the core writes.

use super::Fail;
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::ops_api::{DataBackup, DataBackupParams};

pub const USAGE: &str = "agentsi backup [--keep <n>]   (default 14)";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) =
        args::parse_positional(argv, &[("keep", Kind::Text)]).map_err(Fail::Error)?;
    if !positionals.is_empty() {
        return Err(Fail::Usage(USAGE.into()));
    }
    let keep = match a.text("keep") {
        None => None,
        Some(text) => match text.trim().parse::<u32>() {
            Ok(k) if k >= 1 => Some(k),
            _ => {
                return Err(Fail::Error(format!(
                    "--keep must be a whole number of 1 or more, not \"{text}\""
                )));
            }
        },
    };
    let mut core = Core::connect()?;
    let done = core.call::<DataBackup>(&DataBackupParams { keep })?;
    println!("Backup: {}", done.file);
    if let Some(registry) = done.registry {
        println!("Registry: {registry}");
    }
    if !done.removed.is_empty() {
        println!("Removed: {}", done.removed.join(", "));
    }
    Ok(0)
}
