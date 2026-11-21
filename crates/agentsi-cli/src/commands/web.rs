//! `agentsi web`: the web app is gone. The command opens `agentsi-app`, the desktop app.

use super::Fail;
use agentsi_proto::path::{self, is_executable_file};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const USAGE: &str = "agentsi web";

const APP_BIN_ENV: &str = "AGENTSI_APP_BIN";

/// `AGENTSI_APP_BIN`, else the program next to this one, else the first one on the `PATH`.
fn find_app() -> Option<PathBuf> {
    if let Some(bin) = std::env::var_os(APP_BIN_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(bin)).filter(|p| is_executable_file(p));
    }
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("agentsi-app")));
    sibling
        .filter(|p| is_executable_file(p))
        .or_else(|| path::which("agentsi-app"))
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    if !argv.is_empty() {
        return Err(Fail::Usage(USAGE.into()));
    }
    let app = find_app().ok_or_else(|| Fail::Error(
        "The web app is gone. The desktop app is agentsi-app, and it is not here. Put it next to agentsi, on the PATH, or in AGENTSI_APP_BIN.".into(),
    ))?;
    // The app outlives this command: it gets its own process group and no terminal streams.
    Command::new(&app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", app.display()))?;
    println!("The web app is gone. Opening agentsi-app.");
    Ok(0)
}
