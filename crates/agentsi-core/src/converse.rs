//! Conversation mode in the core.
//!
//! The mode talks to one pane. `voice.converse` starts it for the focused pane (or the pane in
//! the call), turns on voice mode of that pane, and tells the pill with `converse.state`. While
//! the mode runs, `voice.transcript` submits the text of each turn to the target pane, and a
//! reply with nothing to speak sends `converse.state` with `silent_reply`, so the pill listens
//! again. A stop puts voice mode back as it was. The mode ends when the target pane closes.
//!
//! A compositor bind of the owner runs `agentsi-core converse toggle` on press.

use crate::dictate;
use crate::state::{Core, lock};
use agentsi_proto::RpcError;
use agentsi_proto::client::Client;
use agentsi_proto::core_api::Topic;
use agentsi_proto::events::{ConverseState, ConverseStateKind, ConverseStateParams};
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::socket::{SOCKET_ENV, socket_path};
use agentsi_proto::voice_api::{Converse, ConverseAction, ConverseParams, ConverseResult};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: agentsi-core converse <start|stop|toggle>";
const CALL_LIMIT: Duration = Duration::from_secs(5);

/// The mode that runs: its pane, and voice mode of that pane before the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Active {
    pub terminal: String,
    pub voice_was_on: bool,
}

fn announce(core: &Core, state: ConverseStateKind, terminal: Option<String>) {
    core.publish::<ConverseState>(&ConverseStateParams { state, terminal });
}

/// The pane of the mode that runs.
pub fn target(core: &Core) -> Option<String> {
    lock(&core.converse).as_ref().map(|a| a.terminal.clone())
}

/// Ends the mode and puts voice mode back. It does nothing when no mode runs, or when
/// `only` names another pane than the target. Gives true when a mode ended.
pub fn end(core: &Core, only: Option<&str>) -> bool {
    let ended = {
        let mut active = lock(&core.converse);
        if only.is_some_and(|id| active.as_ref().is_none_or(|a| a.terminal != id)) {
            return false;
        }
        active.take()
    };
    let Some(ended) = ended else {
        return false;
    };
    // The pane may be gone: nothing is left to restore then.
    if let Ok(term) = core.terminals.get(&ended.terminal) {
        term.voice_mode(Some(ended.voice_was_on));
    }
    announce(core, ConverseStateKind::Stopped, Some(ended.terminal));
    true
}

/// The target pane finished a reply and nothing is spoken: the pill listens again.
pub fn silent_reply(core: &Core, terminal: &str) {
    if target(core).as_deref() == Some(terminal) {
        announce(
            core,
            ConverseStateKind::SilentReply,
            Some(terminal.to_owned()),
        );
    }
}

fn start(core: &Core, terminal: Option<String>) -> Result<ConverseResult, RpcError> {
    if !core.has_subscriber(Topic::Dictation) {
        return Err(dictate::no_pill());
    }
    if core.dictating() {
        return Err(RpcError::new(
            ErrorKind::Unavailable,
            "a dictation is running",
            "Stop the dictation, then start conversation mode.",
        ));
    }
    let Some(id) = terminal.or_else(|| core.focus()) else {
        announce(core, ConverseStateKind::NoPane, None);
        return Err(RpcError::new(
            ErrorKind::Invalid,
            "no pane has the focus",
            "Focus a pane first.",
        ));
    };
    let term = core.terminals.get(&id)?;
    // The same pane again changes nothing. Another pane takes over.
    if target(core).as_deref() == Some(id.as_str()) {
        return Ok(ConverseResult {
            on: true,
            terminal: Some(id),
        });
    }
    end(core, None);
    let voice_was_on = term.voice_on();
    term.voice_mode(Some(true));
    *lock(&core.converse) = Some(Active {
        terminal: id.clone(),
        voice_was_on,
    });
    announce(core, ConverseStateKind::Started, Some(id.clone()));
    Ok(ConverseResult {
        on: true,
        terminal: Some(id),
    })
}

pub fn command(
    core: &Core,
    action: ConverseAction,
    terminal: Option<String>,
) -> Result<ConverseResult, RpcError> {
    let stop = match action {
        ConverseAction::Start => false,
        ConverseAction::Stop => true,
        // A toggle that names another pane than the target moves the mode to that pane.
        ConverseAction::Toggle => target(core)
            .is_some_and(|running| terminal.as_deref().is_none_or(|named| named == running)),
    };
    if stop {
        end(core, None);
        return Ok(ConverseResult {
            on: false,
            terminal: None,
        });
    }
    start(core, terminal)
}

/// `agentsi-core converse <start|stop|toggle>`. It never starts a core.
pub fn run(args: &[&str]) -> ExitCode {
    let action = match args {
        ["start"] => ConverseAction::Start,
        ["stop"] => ConverseAction::Stop,
        ["toggle"] => ConverseAction::Toggle,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = (|| -> Result<(), String> {
        let socket = match std::env::var_os(SOCKET_ENV).filter(|v| !v.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => socket_path()?.path,
        };
        let mut client =
            Client::connect(&socket).map_err(|e| format!("cannot reach the core: {e}"))?;
        client
            .set_timeout(Some(CALL_LIMIT))
            .map_err(|e| e.to_string())?;
        client
            .call::<Converse>(&ConverseParams {
                action,
                terminal: None,
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agentsi-core converse: {e}");
            ExitCode::FAILURE
        }
    }
}
