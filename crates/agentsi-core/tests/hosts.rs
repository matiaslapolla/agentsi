//! `voice.relay` on a real core: the voice events of a remote core reach the
//! subscribers of this core once, with the host in the terminal id.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::json;

#[test]
fn a_relay_reaches_the_voice_subscribers_as_speak_and_stop() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut pill = Framed::connect(&core.socket.path);
    pill.ok("core.subscribe", json!({"topics": ["voice"]}));
    let mut app = Framed::connect(&core.socket.path);
    app.ok(
        "voice.relay",
        json!({"host": "studio", "terminal": "t1-abc123", "role": "qa", "text": "Done."}),
    );
    assert_eq!(
        pill.next_event("voice.speak"),
        json!({"terminal": "studio/t1-abc123", "role": "qa", "text": "Done."})
    );
    app.ok(
        "voice.relay",
        json!({"host": "studio", "terminal": "t1-abc123"}),
    );
    assert_eq!(
        pill.next_event("voice.stop"),
        json!({"terminal": "studio/t1-abc123"})
    );
}

#[test]
fn a_relay_of_a_local_terminal_or_a_relayed_id_is_refused() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut app = Framed::connect(&core.socket.path);
    let id = app.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sleep", "30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let kind = |v: serde_json::Value| v["error"]["data"]["kind"].clone();
    let same = app.call(
        "voice.relay",
        json!({"host": "self", "terminal": id, "text": "x"}),
    );
    assert_eq!(kind(same), "conflict");
    let relayed = app.call(
        "voice.relay",
        json!({"host": "a", "terminal": "b/t1-abc123", "text": "x"}),
    );
    assert_eq!(kind(relayed), "invalid");
    let no_host = app.call("voice.relay", json!({"host": "", "terminal": "t1-abc123"}));
    assert_eq!(kind(no_host), "invalid");
}
