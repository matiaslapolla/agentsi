//! Dictation and the vocabulary on a real core: `voice.dictate` reaches the subscribers of the
//! `dictation` topic, `voice.transcript` applies the vocabulary and types into the focused pane
//! with no Return, and `vocabulary.get` and `vocabulary.set` keep the file.

mod common;

use agentsi_proto::Event as _;
use agentsi_proto::client::Client;
use agentsi_proto::client::ClientError;
use agentsi_proto::core_api::{Empty, Subscribe, SubscribeParams, Topic};
use agentsi_proto::events::{DictationStart, DictationStop, VoiceMic};
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::terminal_api::{Create, CreateParams};
use agentsi_proto::vocabulary_api::{
    VocabularyEntry, VocabularyGet, VocabularySet, VocabularySetParams,
};
use agentsi_proto::voice_api::{
    Dictate, DictateAction, DictateParams, FocusReport, FocusReportParams, Transcript,
    TranscriptParams,
};
use common::{BIN, CoreProc, TempDir, wait_until};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn core(dir: &TempDir, config: &Path) -> (CoreProc, Client) {
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let core = CoreProc::start_with(dir, &[("AGENTSI_HOME", &home), ("XDG_CONFIG_HOME", config)]);
    let client = Client::connect(&core.socket.path).unwrap();
    client.set_timeout(Some(Duration::from_secs(10))).unwrap();
    (core, client)
}

fn dictate_cli(socket: &Path, action: &str) -> std::process::Output {
    Command::new(BIN)
        .args(["dictate", action])
        .env("AGENTSI_CORE_SOCKET", socket)
        .output()
        .unwrap()
}

/// A pane that writes the exact bytes it gets to `out`.
fn pane(client: &mut Client, dir: &Path, out: &Path) -> String {
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
    client
        .call::<FocusReport>(&FocusReportParams {
            terminal: Some(id.clone()),
        })
        .unwrap();
    id
}

fn entry(term: &str, heard: &[&str]) -> VocabularyEntry {
    VocabularyEntry {
        term: term.into(),
        heard: heard.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn the_dictate_command_reaches_only_a_subscribed_pill() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir, &dir.0.join("config"));
    // No pill: the core says so, and the command exits with a failure.
    let e = client
        .call::<Dictate>(&DictateParams {
            action: DictateAction::Start,
            at_us: None,
        })
        .unwrap_err();
    assert!(
        matches!(&e, ClientError::Rpc(r) if r.kind() == ErrorKind::Unavailable),
        "{e}"
    );
    let out = dictate_cli(&core.socket.path, "start");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pill is not running"));

    // A subscriber of the dictation topic gets the key press and the release.
    let mut pill = Client::connect(&core.socket.path).unwrap();
    pill.call::<Subscribe>(&SubscribeParams {
        topics: vec![Topic::Dictation],
    })
    .unwrap();
    assert!(dictate_cli(&core.socket.path, "start").status.success());
    assert!(dictate_cli(&core.socket.path, "stop").status.success());
    // Each event says when the key edge happened, so the pill can order a quick tap whose
    // two commands arrive swapped.
    let start = pill.next_notification().unwrap();
    let stop = pill.next_notification().unwrap();
    assert_eq!(
        (start.method.as_str(), stop.method.as_str()),
        (DictationStart::NAME, DictationStop::NAME)
    );
    let (start, stop) = (
        start.parse::<DictationStart>().unwrap().at_us,
        stop.parse::<DictationStop>().unwrap().at_us,
    );
    assert!(start > 0 && start <= stop, "{start} {stop}");
    client
        .call::<Dictate>(&DictateParams {
            action: DictateAction::Stop,
            at_us: Some(7),
        })
        .unwrap();
    let given = pill.next_notification().unwrap();
    assert_eq!(given.parse::<DictationStop>().unwrap().at_us, 7);
    assert_eq!(
        dictate_cli(&core.socket.path, "sideways").status.code(),
        Some(2)
    );
}

#[test]
fn dictate_toggle_starts_when_none_runs_and_stops_when_one_runs() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir, &dir.0.join("config"));
    // No pill: the toggle fails like start does, and it starts nothing.
    let out = dictate_cli(&core.socket.path, "toggle");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pill is not running"));

    let mut pill = Client::connect(&core.socket.path).unwrap();
    pill.call::<Subscribe>(&SubscribeParams {
        topics: vec![Topic::Dictation],
    })
    .unwrap();
    let mut seen = Vec::new();
    let mut press = |client: &mut Client, pill: &mut Client, action| {
        client
            .call::<Dictate>(&DictateParams {
                action,
                at_us: None,
            })
            .unwrap();
        seen.push(pill.next_notification().unwrap().method);
    };
    press(&mut client, &mut pill, DictateAction::Toggle);
    press(&mut client, &mut pill, DictateAction::Toggle);
    press(&mut client, &mut pill, DictateAction::Toggle);
    // A stop from anywhere (a click on the pill, an error of the pill) resets the state, so the
    // next toggle starts.
    press(&mut client, &mut pill, DictateAction::Stop);
    press(&mut client, &mut pill, DictateAction::Toggle);
    // A start of the hold form counts too: the toggle after it stops.
    press(&mut client, &mut pill, DictateAction::Stop);
    press(&mut client, &mut pill, DictateAction::Start);
    press(&mut client, &mut pill, DictateAction::Toggle);
    assert_eq!(
        seen,
        [
            DictationStart::NAME,
            DictationStop::NAME,
            DictationStart::NAME,
            DictationStop::NAME,
            DictationStart::NAME,
            DictationStop::NAME,
            DictationStart::NAME,
            DictationStop::NAME,
        ]
    );
    // The command line does the same.
    assert!(dictate_cli(&core.socket.path, "toggle").status.success());
    assert!(dictate_cli(&core.socket.path, "toggle").status.success());
    let start = pill.next_notification().unwrap();
    let stop = pill.next_notification().unwrap();
    assert_eq!(
        (start.method.as_str(), stop.method.as_str()),
        (DictationStart::NAME, DictationStop::NAME)
    );
}

#[test]
fn a_piece_of_toggle_dictation_keeps_its_leading_space() {
    let dir = TempDir::new();
    let config = dir.0.join("config");
    let (_core, mut client) = core(&dir, &config);
    let out = dir.0.join("pane.bytes");
    let _ = pane(&mut client, &dir.0, &out);
    client
        .call::<VocabularySet>(&VocabularySetParams {
            entries: vec![entry("AgentSI", &["Agent C"])],
        })
        .unwrap();
    let first = client
        .call::<Transcript>(&TranscriptParams {
            text: "Open Agent C.".into(),
        })
        .unwrap();
    let second = client
        .call::<Transcript>(&TranscriptParams {
            text: " Then \n stop Agent C.".into(),
        })
        .unwrap();
    assert_eq!(first.text, "Open AgentSI.");
    assert_eq!(second.text, " Then stop AgentSI.");
    // Only a space counts, and a text with nothing in it types nothing.
    let none = client
        .call::<Transcript>(&TranscriptParams { text: "  ".into() })
        .unwrap();
    assert_eq!((none.text.as_str(), none.terminal), ("", None));
    wait_until("both pieces are in the pane", || {
        std::fs::read(&out).is_ok_and(|b| b == b"Open AgentSI. Then stop AgentSI.")
    });
}

#[test]
fn a_transcript_gets_the_vocabulary_and_types_into_the_pane_with_no_return() {
    let dir = TempDir::new();
    let config = dir.0.join("config");
    let (_core, mut client) = core(&dir, &config);
    let out = dir.0.join("pane.bytes");
    let _ = pane(&mut client, &dir.0, &out);

    client
        .call::<VocabularySet>(&VocabularySetParams {
            entries: vec![entry("AgentSI", &["Agent C"])],
        })
        .unwrap();
    let r = client
        .call::<Transcript>(&TranscriptParams {
            text: "Open Agent C now.\nrm -rf x\x1b[2J".into(),
        })
        .unwrap();
    // Each control character is a space, so the line cannot run and the screen cannot clear.
    assert_eq!(r.text, "Open AgentSI now. rm -rf x [2J");
    assert!(r.terminal.is_some());
    wait_until("the text is in the pane", || {
        std::fs::read(&out).is_ok_and(|b| b == r.text.as_bytes())
    });
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(std::fs::read(&out).unwrap(), r.text.as_bytes());

    // With no focused pane nothing is typed, and the answer tells the pill to copy.
    client
        .call::<FocusReport>(&FocusReportParams { terminal: None })
        .unwrap();
    let r = client
        .call::<Transcript>(&TranscriptParams {
            text: "Agent C again".into(),
        })
        .unwrap();
    assert_eq!((r.text.as_str(), r.terminal), ("AgentSI again", None));
}

#[test]
fn a_broken_vocabulary_file_does_not_lose_the_sentence() {
    let dir = TempDir::new();
    let config = dir.0.join("config");
    std::fs::create_dir_all(config.join("agentsi")).unwrap();
    std::fs::write(
        config.join("agentsi/vocabulary.toml"),
        "[[entry]]\nterm = \"AgentSI\"\nheard = [\"Agent C\"\n",
    )
    .unwrap();
    let (_core, mut client) = core(&dir, &config);
    let out = dir.0.join("pane.bytes");
    let _ = pane(&mut client, &dir.0, &out);
    let r = client
        .call::<Transcript>(&TranscriptParams {
            text: "Open Agent C.".into(),
        })
        .unwrap();
    assert_eq!(r.text, "Open Agent C.");
    wait_until("the text is in the pane", || {
        std::fs::read(&out).is_ok_and(|b| b == b"Open Agent C.")
    });
}

#[test]
fn send_return_is_off_unless_the_owner_turns_it_on() {
    let dir = TempDir::new();
    let config = dir.0.join("config");
    std::fs::create_dir_all(config.join("agentsi")).unwrap();
    std::fs::write(
        config.join("agentsi/dictation.toml"),
        "send_return = true\n",
    )
    .unwrap();
    let (_core, mut client) = core(&dir, &config);
    let out = dir.0.join("pane.bytes");
    let _ = pane(&mut client, &dir.0, &out);
    client
        .call::<Transcript>(&TranscriptParams {
            text: "hello".into(),
        })
        .unwrap();
    wait_until("text and Return", || {
        std::fs::read(&out).is_ok_and(|b| b == b"hello\r")
    });
}

#[test]
fn the_vocabulary_is_seeded_from_the_registry_once_and_the_owner_edits_it() {
    let dir = TempDir::new();
    let config = dir.0.join("config");
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("agentsi.config.json"),
        r#"{"products":[{"id":"a","name":"AgentSI"},{"id":"h","name":"Herdr"}]}"#,
    )
    .unwrap();
    let (_core, mut client) = core(&dir, &config);
    let file = config.join("agentsi/vocabulary.toml");
    assert!(!file.exists());

    let first = client.call::<VocabularyGet>(&Empty {}).unwrap();
    let terms: Vec<&str> = first.entries.iter().map(|e| e.term.as_str()).collect();
    assert_eq!(terms, ["AgentSI", "Herdr"]);
    assert_eq!(first.path, file.display().to_string());
    assert!(file.is_file(), "the first read writes the seed");

    // The owner removes one entry. A later read does not bring it back.
    let saved = client
        .call::<VocabularySet>(&VocabularySetParams {
            entries: vec![
                entry(" AgentSI ", &["Agent C", "agent  c", ""]),
                entry("", &["x"]),
            ],
        })
        .unwrap();
    assert_eq!(saved.entries, vec![entry("AgentSI", &["Agent C"])]);
    let again = client.call::<VocabularyGet>(&Empty {}).unwrap();
    assert_eq!(again.entries, saved.entries);
    assert!(std::fs::read_to_string(&file).unwrap().contains("Agent C"));

    // A file that does not parse is an error with a hint, and the core does not overwrite it.
    std::fs::write(&file, "not [toml").unwrap();
    let e = client.call::<VocabularyGet>(&Empty {}).unwrap_err();
    assert!(
        matches!(&e, ClientError::Rpc(r) if r.kind() == ErrorKind::Unavailable),
        "{e}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "not [toml");
}

#[test]
fn each_dictate_start_and_stop_also_sends_voice_mic_on_the_voice_topic() {
    let dir = TempDir::new();
    let (core, mut client) = core(&dir, &dir.0.join("config"));
    let mut pill = Client::connect(&core.socket.path).unwrap();
    pill.call::<Subscribe>(&SubscribeParams {
        topics: vec![Topic::Dictation],
    })
    .unwrap();
    let mut app = Client::connect(&core.socket.path).unwrap();
    app.set_timeout(Some(Duration::from_secs(10))).unwrap();
    app.call::<Subscribe>(&SubscribeParams {
        topics: vec![Topic::Voice],
    })
    .unwrap();
    let mut mic = Vec::new();
    for action in [
        DictateAction::Start,
        DictateAction::Stop,
        DictateAction::Toggle,
        DictateAction::Toggle,
    ] {
        client
            .call::<Dictate>(&DictateParams {
                action,
                at_us: None,
            })
            .unwrap();
        loop {
            let n = app.next_notification().unwrap();
            if n.method == VoiceMic::NAME {
                mic.push(n.parse::<VoiceMic>().unwrap().open);
                break;
            }
        }
    }
    assert_eq!(mic, [true, false, true, false]);
    // The pill keeps only its own topic: its next events are the dictation edges.
    let first = pill.next_notification().unwrap();
    assert_eq!(first.method, DictationStart::NAME);
}
