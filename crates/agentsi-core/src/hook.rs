//! `agentsi-core hook <event>`: the bridge from a Claude Code hook to the
//! core of its terminal.
//!
//! A hook must never slow or break a Claude session. So the bridge always
//! exits with 0, in 200 ms or less, and prints nothing on a failure. It
//! writes each error as one line to `<data root>/logs/hook.log`. It never
//! starts a core.
//!
//! Outside the app (`AGENTSI_TERMINAL` unset) the bridge serves only a Claude Code
//! session with voice mode on, and only `UserPromptSubmit` and `Stop`. A session
//! without a file in `<data root>/voice/sessions/` costs one directory read.

use crate::clock::iso_now;
use crate::voice_session;
use agentsi_proto::client::Client;
use agentsi_proto::socket::{SOCKET_ENV, socket_path};
use agentsi_proto::terminal_api::TERMINAL_ENV;
use agentsi_proto::voice_api::{HookEvent, HookEventParams, HookKind};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// The whole run of the bridge. The limit of the spec is 200 ms.
const BUDGET: Duration = Duration::from_millis(170);
/// The most that the bridge reads from stdin.
const MAX_PAYLOAD: u64 = 8 << 20;

pub fn run(args: &[&str]) -> ExitCode {
    let terminal = std::env::var(TERMINAL_ENV).unwrap_or_default();
    let event = args.first().and_then(|w| HookKind::parse(w));
    if terminal.is_empty() && !session_may_speak(event) {
        return ExitCode::SUCCESS;
    }
    let start = Instant::now();
    // Stdin or the socket can block. This thread ends the process when the budget is spent.
    let name = args.first().copied().unwrap_or("").to_string();
    let log_name = name.clone();
    let log_terminal = terminal.clone();
    std::thread::spawn(move || {
        std::thread::sleep(BUDGET);
        log(&log_terminal, &log_name, "no answer within the time limit");
        std::process::exit(0);
    });
    match event {
        None => log(&terminal, &name, "unknown hook event"),
        Some(event) => {
            if let Err(e) = forward(&terminal, event, start) {
                log(&terminal, &name, &e);
            }
        }
    }
    ExitCode::SUCCESS
}

/// True when a session outside the app could have voice mode on. It reads no stdin.
fn session_may_speak(event: Option<HookKind>) -> bool {
    matches!(event, Some(HookKind::UserPromptSubmit | HookKind::Stop))
        && agentsi_store::paths::data_root(None).is_ok_and(|root| voice_session::any_on(&root))
}

fn forward(terminal: &str, event: HookKind, start: Instant) -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_PAYLOAD)
        .read_to_end(&mut input)
        .map_err(|e| format!("cannot read stdin: {e}"))?;
    let (payload, bad) = match serde_json::from_slice::<Value>(&input) {
        Ok(v @ Value::Object(_)) => (v, None),
        Ok(_) => (json!({}), Some("stdin is not a JSON object".to_string())),
        Err(e) => (json!({}), Some(format!("bad JSON on stdin: {e}"))),
    };
    if let Some(bad) = bad {
        // A session outside the app is unknown without its JSON. Nobody asked for its hooks.
        if terminal.is_empty() {
            return Ok(());
        }
        // The state still reaches the core.
        log(terminal, &format!("{event:?}"), &bad);
    }
    let session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if terminal.is_empty() {
        let on = agentsi_store::paths::data_root(None)
            .is_ok_and(|root| voice_session::is_on(&root, &session));
        if !on {
            return Ok(());
        }
    }
    let socket = match std::env::var_os(SOCKET_ENV).filter(|v| !v.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => socket_path()?.path,
    };
    let mut client = Client::connect(&socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let left = BUDGET
        .saturating_sub(start.elapsed())
        .max(Duration::from_millis(1));
    client
        .set_timeout(Some(left))
        .map_err(|e| format!("cannot set the time limit: {e}"))?;
    let result = client
        .call::<HookEvent>(&HookEventParams {
            terminal: terminal.to_string(),
            session,
            event,
            payload,
        })
        .map_err(|e| e.to_string())?;
    // The only output of the bridge. Claude Code gives it to the model as context.
    if event == HookKind::UserPromptSubmit
        && let Some(context) = result.context
    {
        let out = json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": context}});
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{out}");
        let _ = stdout.flush();
    }
    Ok(())
}

/// One line in `<data root>/logs/hook.log`. It never fails.
fn log(terminal: &str, event: &str, message: &str) {
    let Ok(root) = agentsi_store::paths::data_root(None) else {
        return;
    };
    let dir = root.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let terminal = if terminal.is_empty() { "-" } else { terminal };
    let line = format!(
        "{} {event} {terminal} {}\n",
        iso_now(),
        message.replace('\n', " ").trim()
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("hook.log"))
    {
        let _ = file.write_all(line.as_bytes());
    }
}
