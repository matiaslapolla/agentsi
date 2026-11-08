//! `agentsi voice on|off|status`: voice mode of the pane or the Claude Code session that
//! runs the command. In a pane of the app the core keeps the state for the terminal named
//! by `AGENTSI_TERMINAL`, which the core sets in the environment of each program it
//! starts. In a plain Claude Code session the state belongs to `CLAUDE_CODE_SESSION_ID`.

use super::Fail;
use crate::core::Core;
use agentsi_proto::core_api::Empty;
use agentsi_proto::ops_api::CoreStatus;
use agentsi_proto::terminal_api::TERMINAL_ENV;
use agentsi_proto::voice_api::{Mode, ModeParams, SessionMode, SessionModeParams};

const SESSION_ENV: &str = "CLAUDE_CODE_SESSION_ID";

pub const USAGE: &str = "agentsi voice on|off|status";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let action = match argv {
        [a] if matches!(a.as_str(), "on" | "off" | "status") => a.as_str(),
        _ => return Err(Fail::Usage(USAGE.into())),
    };
    let var = |name: &str| std::env::var(name).unwrap_or_default().trim().to_string();
    let (terminal, session) = (var(TERMINAL_ENV), var(SESSION_ENV));
    if terminal.is_empty() && session.is_empty() {
        eprintln!(
            "Run this command in a pane of AgentSI or in a Claude Code session: {TERMINAL_ENV} and {SESSION_ENV} are not set."
        );
        return Ok(1);
    }
    let mut core = Core::connect()?;
    let on = match action {
        "status" => None,
        a => Some(a == "on"),
    };
    let (now_on, place) = if terminal.is_empty() {
        let r = core.call::<SessionMode>(&SessionModeParams { session, on })?;
        (r.on, "session")
    } else {
        let r = core.call::<Mode>(&ModeParams { terminal, on })?;
        (r.on, "pane")
    };
    println!(
        "Voice mode is {} for this {place}.",
        if now_on { "on" } else { "off" }
    );
    if action == "on" {
        let status = core.call::<CoreStatus>(&Empty {})?;
        println!("Transcripts: {}/voice/transcripts", status.data_root);
        if !status.pill {
            eprintln!(
                "Warning: agentsi-pill is not running, so nothing is spoken and the dictation key does nothing. Start agentsi-pill."
            );
        }
    }
    Ok(0)
}
