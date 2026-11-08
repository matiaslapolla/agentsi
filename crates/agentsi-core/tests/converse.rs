//! Conversation mode on a real core: `voice.converse` turns on voice mode of the target pane and
//! puts it back, `voice.transcript` submits a turn to the target pane with a Return, a reply
//! with nothing to speak sends `converse.state` with `silent_reply`, and the mode ends when the
//! target pane closes or a dictation starts. The pane runs `cat` on a raw terminal and writes
//! each byte it gets to a file.

mod common;

use agentsi_proto::Event as _;
use agentsi_proto::client::{Client, ClientError};
use agentsi_proto::core_api::{Subscribe, SubscribeParams, Topic};
use agentsi_proto::events::{
    ConverseState, ConverseStateKind, ConverseStateParams, VoiceSpeak, VoiceStop,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::terminal_api::{Create, CreateParams, Kill, TerminalId};
use agentsi_proto::vocabulary_api::{VocabularyEntry, VocabularySet, VocabularySetParams};
use agentsi_proto::voice_api::{
    Converse, ConverseAction, ConverseParams, ConverseResult, Dictate, DictateAction,
    DictateParams, FocusReport, FocusReportParams, HookEvent, HookEventParams, HookKind, Mode,
    ModeParams, Transcript, TranscriptParams,
};
use common::{BIN, CoreProc, TempDir, wait_until};
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn core(dir: &TempDir) -> (CoreProc, Client) {
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let config = dir.0.join("config");
    let core = CoreProc::start_with(
        dir,
        &[("AGENTSI_HOME", &home), ("XDG_CONFIG_HOME", &config)],
    );
    let client = Client::connect(&core.socket.path).unwrap();
    client.set_timeout(Some(Duration::from_secs(10))).unwrap();
    (core, client)
}

fn pane(client: &mut Client, dir: &Path, out: &Path, focus: bool) -> String {
    let id = client
        .call::<Create>(&CreateParams {
            cwd: dir.display().to_string(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                format!("stty raw -echo; exec cat > '{}'", out.display()),
            ],
            env: Default::default(),
            product: None,
            cols: None,
            rows: None,
        })
        .unwrap()
        .id;
    if focus {
        focus_on(client, Some(id.clone()));
    }
    id
}

fn focus_on(client: &mut Client, terminal: Option<String>) {
    client
        .call::<FocusReport>(&FocusReportParams { terminal })
        .unwrap();
}

fn pill(core: &CoreProc, topics: Vec<Topic>) -> Client {
    let mut pill = Client::connect(&core.socket.path).unwrap();
    pill.set_timeout(Some(Duration::from_secs(10))).unwrap();
    pill.call::<Subscribe>(&SubscribeParams { topics }).unwrap();
    pill
}

fn converse(client: &mut Client, action: ConverseAction, terminal: Option<&str>) -> ConverseResult {
    client
        .call::<Converse>(&ConverseParams {
            action,
            terminal: terminal.map(str::to_owned),
        })
        .unwrap()
}

/// The next event, without the moves of the dictation target, which the focus changes cause.
fn next(pill: &mut Client) -> agentsi_proto::rpc::Notification {
    loop {
        let n = pill.next_notification().unwrap();
        if n.method != "dictation.target" {
            return n;
        }
    }
}

fn state_of(pill: &mut Client) -> (ConverseStateKind, Option<String>) {
    let n = next(pill);
    assert_eq!(n.method, ConverseState::NAME, "{n:?}");
    let ConverseStateParams { state, terminal } = n.parse::<ConverseState>().unwrap();
    (state, terminal)
}

fn voice(client: &mut Client, id: &str, on: Option<bool>) -> bool {
    client
        .call::<Mode>(&ModeParams {
            terminal: id.into(),
            on,
        })
        .unwrap()
        .on
}

fn hook(client: &mut Client, id: &str, event: HookKind, payload: serde_json::Value) {
    client
        .call::<HookEvent>(&HookEventParams {
            terminal: id.into(),
            session: "s1".into(),
            event,
            payload,
        })
        .unwrap();
}

fn rpc_kind(e: ClientError) -> ErrorKind {
    match e {
        ClientError::Rpc(r) => r.kind(),
        other => panic!("{other}"),
    }
}

fn bytes(out: &Path) -> Vec<u8> {
    std::fs::read(out).unwrap_or_default()
}

#[test]
fn a_start_needs_a_pill_and_a_pane_and_no_dictation() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir);
    let out = dir.0.join("pane.bytes");
    // No pill: the core says so.
    let e = client
        .call::<Converse>(&ConverseParams {
            action: ConverseAction::Start,
            terminal: None,
        })
        .unwrap_err();
    assert_eq!(rpc_kind(e), ErrorKind::Unavailable);
    let cli = Command::new(BIN)
        .args(["converse", "toggle"])
        .env("AGENTSI_CORE_SOCKET", &core.socket.path)
        .output()
        .unwrap();
    assert!(!cli.status.success());
    assert!(String::from_utf8_lossy(&cli.stderr).contains("pill is not running"));
    assert_eq!(
        Command::new(BIN)
            .args(["converse", "sideways"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );

    // A pill, and no focused pane: the pill is told, and the call fails.
    let mut pill = pill(&core, vec![Topic::Dictation, Topic::Voice]);
    let e = client
        .call::<Converse>(&ConverseParams {
            action: ConverseAction::Start,
            terminal: None,
        })
        .unwrap_err();
    assert_eq!(rpc_kind(e), ErrorKind::Invalid);
    assert_eq!(state_of(&mut pill), (ConverseStateKind::NoPane, None));

    // A pane that does not exist.
    let e = client
        .call::<Converse>(&ConverseParams {
            action: ConverseAction::Start,
            terminal: Some("t-nope".into()),
        })
        .unwrap_err();
    assert_eq!(rpc_kind(e), ErrorKind::NotFound);

    // A dictation that runs blocks the start.
    let id = pane(&mut client, &dir.0, &out, true);
    client
        .call::<Dictate>(&DictateParams {
            action: DictateAction::Start,
            at_us: None,
        })
        .unwrap();
    assert_eq!(next(&mut pill).method, "dictation.start");
    let e = client
        .call::<Converse>(&ConverseParams {
            action: ConverseAction::Start,
            terminal: None,
        })
        .unwrap_err();
    assert_eq!(rpc_kind(e), ErrorKind::Unavailable);
    assert!(!voice(&mut client, &id, None));
}

#[test]
fn the_mode_turns_voice_mode_on_and_puts_it_back_and_toggles() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir);
    let mut pill = pill(&core, vec![Topic::Dictation, Topic::Voice]);
    let (a, b) = (dir.0.join("a.bytes"), dir.0.join("b.bytes"));
    let off = pane(&mut client, &dir.0, &a, true);
    let on = pane(&mut client, &dir.0, &b, false);
    voice(&mut client, &on, Some(true));

    // The focused pane is the target. Voice mode was off, and goes on.
    let r = converse(&mut client, ConverseAction::Toggle, None);
    assert_eq!(
        r,
        ConverseResult {
            on: true,
            terminal: Some(off.clone())
        }
    );
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Started, Some(off.clone()))
    );
    assert!(voice(&mut client, &off, None));
    // A later focus change does not move the target.
    focus_on(&mut client, Some(on.clone()));

    // The next toggle stops, and voice mode is back off. A stop twice sends nothing more.
    let r = converse(&mut client, ConverseAction::Toggle, None);
    assert_eq!(
        r,
        ConverseResult {
            on: false,
            terminal: None
        }
    );
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Stopped, Some(off.clone()))
    );
    assert!(!voice(&mut client, &off, None));
    converse(&mut client, ConverseAction::Stop, None);

    // Voice mode that was on before stays on after the stop.
    converse(&mut client, ConverseAction::Start, Some(&on));
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Started, Some(on.clone()))
    );
    converse(&mut client, ConverseAction::Stop, None);
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Stopped, Some(on.clone()))
    );
    assert!(voice(&mut client, &on, None));

    // A start for another pane takes over: the old pane gets its voice mode back first.
    converse(&mut client, ConverseAction::Start, Some(&off));
    converse(&mut client, ConverseAction::Start, Some(&on));
    assert_eq!(state_of(&mut pill).0, ConverseStateKind::Started);
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Stopped, Some(off.clone()))
    );
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Started, Some(on.clone()))
    );
    assert!(!voice(&mut client, &off, None));
    // The same pane again changes nothing.
    converse(&mut client, ConverseAction::Start, Some(&on));
    // A toggle that names another pane moves the mode. A toggle that names the target stops it.
    converse(&mut client, ConverseAction::Toggle, Some(&off));
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Stopped, Some(on.clone()))
    );
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Started, Some(off.clone()))
    );
    let r = converse(&mut client, ConverseAction::Toggle, Some(&off));
    assert!(!r.on);
    assert_eq!(state_of(&mut pill).0, ConverseStateKind::Stopped);
    converse(&mut client, ConverseAction::Stop, None);
}

#[test]
fn a_turn_goes_to_the_target_pane_with_a_return_and_the_vocabulary() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir);
    let _pill = pill(&core, vec![Topic::Dictation, Topic::Voice]);
    let (a, b) = (dir.0.join("a.bytes"), dir.0.join("b.bytes"));
    let target = pane(&mut client, &dir.0, &a, true);
    let other = pane(&mut client, &dir.0, &b, false);
    client
        .call::<VocabularySet>(&VocabularySetParams {
            entries: vec![VocabularyEntry {
                term: "AgentSI".into(),
                heard: vec!["Agent C".into()],
            }],
        })
        .unwrap();
    converse(&mut client, ConverseAction::Start, None);
    // The focus moves to the other pane. The turn still goes to the target.
    focus_on(&mut client, Some(other));
    let r = client
        .call::<Transcript>(&TranscriptParams {
            text: "open Agent C\nplease".into(),
        })
        .unwrap();
    assert_eq!(r.terminal.as_deref(), Some(target.as_str()));
    assert_eq!(r.text, "open AgentSI please");
    wait_until("the turn and the Return are in the pane", || {
        bytes(&a) == b"open AgentSI please\r"
    });
    assert!(bytes(&b).is_empty());

    // A turn with no text sends nothing.
    let r = client
        .call::<Transcript>(&TranscriptParams {
            text: " \n ".into(),
        })
        .unwrap();
    assert_eq!(r.terminal, None);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(bytes(&a), b"open AgentSI please\r");

    // After the stop, a transcript is dictation again: the focused pane, and no Return.
    converse(&mut client, ConverseAction::Stop, None);
    let r = client
        .call::<Transcript>(&TranscriptParams {
            text: "typed".into(),
        })
        .unwrap();
    assert!(r.terminal.is_some());
    wait_until("the text is in the other pane", || bytes(&b) == b"typed");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(bytes(&b), b"typed");
}

#[test]
fn a_reply_with_nothing_to_speak_tells_the_pill_to_listen_again() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir);
    let mut pill = pill(&core, vec![Topic::Dictation, Topic::Voice]);
    let out = dir.0.join("a.bytes");
    let id = pane(&mut client, &dir.0, &out, true);
    converse(&mut client, ConverseAction::Start, None);
    assert_eq!(state_of(&mut pill).0, ConverseStateKind::Started);

    // The prompt of the turn stops the speech of the last reply, as in voice mode.
    hook(
        &mut client,
        &id,
        HookKind::UserPromptSubmit,
        json!({"prompt": "hello"}),
    );
    assert_eq!(next(&mut pill).method, VoiceStop::NAME);
    // A reply with a sentence is spoken, and the pill hears no silent reply.
    hook(
        &mut client,
        &id,
        HookKind::Stop,
        json!({"last_assistant_message": "Done. All good."}),
    );
    let speak = next(&mut pill);
    assert_eq!(speak.method, VoiceSpeak::NAME);

    // A reply with no spoken sentence ends in a silent reply. The code block is on screen only.
    hook(
        &mut client,
        &id,
        HookKind::UserPromptSubmit,
        json!({"prompt": "again"}),
    );
    assert_eq!(next(&mut pill).method, VoiceStop::NAME);
    hook(
        &mut client,
        &id,
        HookKind::Stop,
        json!({"last_assistant_message": "   "}),
    );
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::SilentReply, Some(id.clone()))
    );

    // The owner turns voice mode off in the pane: the next Stop is silent too.
    voice(&mut client, &id, Some(false));
    hook(
        &mut client,
        &id,
        HookKind::UserPromptSubmit,
        json!({"prompt": "third"}),
    );
    hook(
        &mut client,
        &id,
        HookKind::Stop,
        json!({"last_assistant_message": "A reply."}),
    );
    assert_eq!(state_of(&mut pill).0, ConverseStateKind::SilentReply);
}

#[test]
fn the_mode_ends_when_the_target_pane_closes_or_a_dictation_starts() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir);
    let mut pill = pill(&core, vec![Topic::Dictation, Topic::Voice, Topic::Terminal]);
    let out = dir.0.join("a.bytes");
    let id = pane(&mut client, &dir.0, &out, true);
    converse(&mut client, ConverseAction::Start, None);
    assert_eq!(state_of(&mut pill).0, ConverseStateKind::Started);
    client.call::<Kill>(&TerminalId { id: id.clone() }).unwrap();
    // The pane closes: the mode ends, and a later stop sends nothing more.
    let ended = loop {
        let n = next(&mut pill);
        if n.method == ConverseState::NAME {
            break n.parse::<ConverseState>().unwrap();
        }
    };
    assert_eq!(ended.state, ConverseStateKind::Stopped);
    assert_eq!(ended.terminal, Some(id));
    converse(&mut client, ConverseAction::Stop, None);

    // A dictation start ends the mode before it tells the pill to record.
    let b = dir.0.join("b.bytes");
    let second = pane(&mut client, &dir.0, &b, true);
    converse(&mut client, ConverseAction::Start, None);
    assert_eq!(state_of(&mut pill).0, ConverseStateKind::Started);
    client
        .call::<Dictate>(&DictateParams {
            action: DictateAction::Start,
            at_us: None,
        })
        .unwrap();
    assert_eq!(
        state_of(&mut pill),
        (ConverseStateKind::Stopped, Some(second.clone()))
    );
    assert_eq!(next(&mut pill).method, "dictation.start");
    assert!(!voice(&mut client, &second, None));
}
