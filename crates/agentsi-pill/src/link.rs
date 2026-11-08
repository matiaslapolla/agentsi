//! The link to the local core: one thread reads the voice events, and a click
//! that stops a recording opens a short second connection.

use crate::state::{self, Input};
use agentsi_proto::RpcError;
use agentsi_proto::client::{Client, ClientError, connect_or_start, find_core_bin};
use agentsi_proto::core_api::{Hello, HelloParams, Subscribe, SubscribeParams, Topic};
use agentsi_proto::events::{
    ConverseState, ConverseStateKind, DictationStart, DictationStop, DictationTarget, Lagged,
    VoiceSpeak, VoiceStop,
};
use agentsi_proto::rpc::Notification;
use agentsi_proto::socket::socket_path;
use agentsi_proto::voice_api::{
    Converse, ConverseAction, ConverseParams, Dictate, DictateAction, DictateParams, Transcript,
    TranscriptParams, TranscriptResult,
};
use futures::channel::mpsc::UnboundedSender;
use std::time::Duration;

const CLIENT_NAME: &str = "agentsi-pill";
const RETRY_FIRST: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(5);
const CALL_LIMIT: Duration = Duration::from_secs(5);
/// The line that the pill shows when the core does not end a recording.
pub const STOP_FAILED: &str = "The core did not stop the recording";
/// The line that the pill shows when the core does not end conversation mode.
pub const STOP_FAILED_CONVERSATION: &str = "The core did not stop conversation mode";

/// What the link sends to the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    Input(Input),
    /// `dictation.target`: the terminal that gets dictated text.
    DictationTarget(Option<String>),
}

pub type LinkSender = UnboundedSender<LinkEvent>;

/// Maps one notification of the core to what the window needs, or `None`.
pub fn translate(n: &Notification) -> Option<LinkEvent> {
    if let Some(p) = n.parse::<VoiceSpeak>() {
        return Some(LinkEvent::Input(Input::Speak {
            terminal: p.terminal,
            role: p.role,
            text: p.text,
        }));
    }
    if let Some(p) = n.parse::<VoiceStop>() {
        return Some(LinkEvent::Input(Input::CoreStop {
            terminal: p.terminal,
        }));
    }
    if let Some(p) = n.parse::<DictationStart>() {
        // The mode is not in the event. The session reads it from the key file.
        return Some(LinkEvent::Input(Input::DictationStart {
            at_us: p.at_us,
            toggle: false,
        }));
    }
    if let Some(p) = n.parse::<DictationStop>() {
        return Some(LinkEvent::Input(Input::DictationStop { at_us: p.at_us }));
    }
    if let Some(p) = n.parse::<DictationTarget>() {
        return Some(LinkEvent::DictationTarget(p.terminal));
    }
    if let Some(p) = n.parse::<ConverseState>() {
        let terminal = p.terminal.unwrap_or_default();
        let event = match p.state {
            ConverseStateKind::Started => state::Converse::Started { terminal },
            ConverseStateKind::Stopped => state::Converse::Stopped,
            ConverseStateKind::SilentReply => state::Converse::SilentReply { terminal },
            ConverseStateKind::NoPane => state::Converse::NoPane,
        };
        return Some(LinkEvent::Input(Input::Converse(event)));
    }
    // Events are missing. A speech in progress may be stale, so the pill hides it.
    if n.parse::<Lagged>().is_some() {
        return Some(LinkEvent::Input(Input::Disconnected));
    }
    None
}

/// Starts the thread that keeps the subscription. It retries with a growing wait.
pub fn spawn(tx: LinkSender) {
    std::thread::Builder::new()
        .name("core-link".into())
        .spawn(move || {
            let mut wait = RETRY_FIRST;
            // Only the first connect starts a core. A core that stops later stopped on purpose.
            let mut start = true;
            loop {
                match session(&tx, start, &mut wait) {
                    Ok(()) => return,
                    Err(e) => {
                        eprintln!("agentsi-pill: the link to the core ended: {e}");
                        if tx
                            .unbounded_send(LinkEvent::Input(Input::Disconnected))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                start = false;
                std::thread::sleep(wait);
                wait = (wait * 2).min(RETRY_MAX);
            }
        })
        .expect("the OS starts a thread");
}

/// A connection with the hello done. It starts the core if none runs.
fn connect(start: bool) -> Result<Client, String> {
    let socket = socket_path()?;
    let stream = match std::os::unix::net::UnixStream::connect(&socket.path) {
        Ok(stream) => stream,
        Err(e) if !start => return Err(e.to_string()),
        Err(_) => {
            let bin =
                find_core_bin().ok_or("agentsi-core is not on the PATH. Set AGENTSI_CORE_BIN.")?;
            connect_or_start(&socket, &bin).map_err(|e| e.to_string())?
        }
    };
    let mut client = Client::new(stream).map_err(|e| e.to_string())?;
    client
        .set_timeout(Some(CALL_LIMIT))
        .map_err(|e| e.to_string())?;
    client
        .call::<Hello>(&HelloParams {
            framed: false,
            client: Some(CLIENT_NAME.into()),
        })
        .map_err(|e| describe(&e))?;
    Ok(client)
}

fn describe(e: &ClientError) -> String {
    match e {
        ClientError::Rpc(RpcError { message, .. }) => message.clone(),
        other => other.to_string(),
    }
}

/// One connection, until it breaks. `Ok` only when the window is gone.
/// After the subscribe, it sets `wait` back to the first retry wait.
fn session(tx: &LinkSender, start: bool, wait: &mut Duration) -> Result<(), String> {
    let mut client = connect(start)?;
    client
        .call::<Subscribe>(&SubscribeParams {
            topics: vec![Topic::Voice, Topic::Dictation],
        })
        .map_err(|e| describe(&e))?;
    *wait = RETRY_FIRST;
    // The wait for an event has no limit. The core sends nothing while the pill is idle.
    client.set_timeout(None).map_err(|e| e.to_string())?;
    loop {
        let n = client.next_notification().map_err(|e| describe(&e))?;
        if let Some(mut event) = translate(&n) {
            if let LinkEvent::Input(Input::DictationStart { toggle, .. }) = &mut event {
                *toggle = crate::config::dictation_toggle();
            }
            if tx.unbounded_send(event).is_err() {
                return Ok(());
            }
        }
    }
}

/// Tells the core to end the recording. The call runs in its own thread, on its
/// own connection, because the reader thread waits for events. A failure goes
/// back to the window as an error line.
pub fn stop_recording(tx: LinkSender) {
    std::thread::spawn(move || {
        let result = connect(false).and_then(|mut c| {
            c.call::<Dictate>(&DictateParams {
                action: DictateAction::Stop,
                at_us: None,
            })
            .map_err(|e| describe(&e))
        });
        if let Err(e) = result {
            eprintln!("agentsi-pill: cannot stop the recording: {e}");
            let _ = tx.unbounded_send(LinkEvent::Input(Input::Fail(STOP_FAILED.into())));
        }
    });
}

/// Tells the core to end conversation mode, for a click, the word "stop" or a failure. The call
/// runs in its own thread on its own connection, as [`stop_recording`] does.
pub fn stop_conversation(tx: LinkSender) {
    std::thread::spawn(move || {
        let result = connect(false).and_then(|mut c| {
            c.call::<Converse>(&ConverseParams {
                action: ConverseAction::Stop,
                terminal: None,
            })
            .map_err(|e| describe(&e))
        });
        if let Err(e) = result {
            eprintln!("agentsi-pill: cannot stop conversation mode: {e}");
            let _ = tx.unbounded_send(LinkEvent::Input(Input::Fail(
                STOP_FAILED_CONVERSATION.into(),
            )));
        }
    });
}

/// Gives the text of a dictation to the core, which types it into the focused pane.
/// It runs on the calling thread, on its own connection.
pub fn send_transcript(text: &str) -> Result<TranscriptResult, String> {
    let mut client = connect(false)?;
    client
        .call::<Transcript>(&TranscriptParams {
            text: text.to_owned(),
        })
        .map_err(|e| describe(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_proto::events::{
        DictationEdgeParams, DictationTargetParams, LaggedParams, VoiceSpeakParams, VoiceStopParams,
    };

    #[test]
    fn the_dictation_events_become_inputs() {
        let start = Notification::of::<DictationStart>(&DictationEdgeParams { at_us: 5 });
        assert_eq!(
            translate(&start),
            Some(LinkEvent::Input(Input::DictationStart {
                at_us: 5,
                toggle: false
            }))
        );
        let stop = Notification::of::<DictationStop>(&DictationEdgeParams { at_us: 9 });
        assert_eq!(
            translate(&stop),
            Some(LinkEvent::Input(Input::DictationStop { at_us: 9 }))
        );
    }

    #[test]
    fn the_converse_events_become_inputs() {
        use agentsi_proto::events::ConverseStateParams;
        let n = |state, terminal: Option<&str>| {
            Notification::of::<ConverseState>(&ConverseStateParams {
                state,
                terminal: terminal.map(str::to_owned),
            })
        };
        let input = |c| Some(LinkEvent::Input(Input::Converse(c)));
        assert_eq!(
            translate(&n(ConverseStateKind::Started, Some("t1"))),
            input(state::Converse::Started {
                terminal: "t1".into()
            })
        );
        assert_eq!(
            translate(&n(ConverseStateKind::Stopped, Some("t1"))),
            input(state::Converse::Stopped)
        );
        assert_eq!(
            translate(&n(ConverseStateKind::SilentReply, Some("t1"))),
            input(state::Converse::SilentReply {
                terminal: "t1".into()
            })
        );
        assert_eq!(
            translate(&n(ConverseStateKind::NoPane, None)),
            input(state::Converse::NoPane)
        );
    }

    #[test]
    fn a_speak_event_becomes_a_speak_input() {
        let n = Notification::of::<VoiceSpeak>(&VoiceSpeakParams {
            terminal: "t1".into(),
            role: "pm".into(),
            text: "Hi.".into(),
        });
        assert_eq!(
            translate(&n),
            Some(LinkEvent::Input(Input::Speak {
                terminal: "t1".into(),
                role: "pm".into(),
                text: "Hi.".into()
            }))
        );
    }

    #[test]
    fn a_stop_event_becomes_a_stop_input() {
        let n = Notification::of::<VoiceStop>(&VoiceStopParams {
            terminal: "t1".into(),
        });
        assert_eq!(
            translate(&n),
            Some(LinkEvent::Input(Input::CoreStop {
                terminal: "t1".into()
            }))
        );
    }

    #[test]
    fn a_dictation_target_keeps_its_terminal() {
        let n = Notification::of::<DictationTarget>(&DictationTargetParams { terminal: None });
        assert_eq!(translate(&n), Some(LinkEvent::DictationTarget(None)));
    }

    #[test]
    fn a_lagged_notice_hides_the_speech() {
        let n = Notification::of::<Lagged>(&LaggedParams { missed: 3 });
        assert_eq!(translate(&n), Some(LinkEvent::Input(Input::Disconnected)));
    }

    #[test]
    fn another_event_is_ignored() {
        let n = Notification {
            jsonrpc: agentsi_proto::rpc::V2,
            method: "terminal.exit".into(),
            params: serde_json::json!({}),
        };
        assert_eq!(translate(&n), None);
    }
}
