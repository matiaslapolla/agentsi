//! `agentsi check`: the pass or fail check of a minutes file.

use super::Fail;
use crate::args::{self, Kind};
use crate::core::Core;
use crate::json;
use agentsi_proto::meeting_api::{MinutesCheck, MinutesCheckParams};
use serde_json::Value;

pub const USAGE: &str = "agentsi check <minutes.json> [--roles pm,dev]";

/// The text of a read that failed, as Node words it.
fn read_error(file: &str, e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => {
            format!("ENOENT: no such file or directory, open '{file}'")
        }
        _ => e.to_string(),
    }
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) =
        args::parse_positional(argv, &[("roles", Kind::Text)]).map_err(Fail::Error)?;
    let Some(file) = positionals.first() else {
        return Err(Fail::Usage("Missing <minutes.json>".into()));
    };
    let text = std::fs::read_to_string(file).map_err(|e| read_error(file, &e))?;
    let minutes: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let roles = a.text("roles").map(|r| {
        r.split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    });
    let mut core = Core::connect()?;
    let result = core.call::<MinutesCheck>(&MinutesCheckParams { minutes, roles })?;
    println!("{}", json::pretty(&result));
    Ok(if result.passed { 0 } else { 1 })
}
