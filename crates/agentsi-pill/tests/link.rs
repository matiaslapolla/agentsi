//! The link to the core, against a fake core on a socket in a throwaway folder.
//! One test only: it sets `AGENTSI_CORE_SOCKET`, which the whole process shares.

use agentsi_pill::link::{self, LinkEvent};
use agentsi_pill::state::Input;
use futures::StreamExt;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::time::Duration;

/// Answers each request of one connection with `{}`-like results. Reports each
/// request on `seen`. On `core.subscribe` it sends the events of `after_subscribe`.
fn serve(stream: UnixStream, seen: mpsc::Sender<Value>, after_subscribe: Vec<Value>) {
    let mut writer = stream.try_clone().unwrap();
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        let request: Value = serde_json::from_str(&line).unwrap();
        let _ = seen.send(request.clone());
        let result = match request["method"].as_str().unwrap() {
            "core.hello" => json!({"protocol": 1, "version": "0.0.0", "features": ["core"]}),
            "core.subscribe" => json!({"topics": request["params"]["topics"]}),
            _ => json!({}),
        };
        let reply = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
        writeln!(writer, "{reply}").unwrap();
        if request["method"] == "core.subscribe" {
            for event in &after_subscribe {
                writeln!(writer, "{event}").unwrap();
            }
        }
    }
}

#[test]
fn the_pill_subscribes_reads_voice_events_and_sends_stop() {
    let dir = std::env::temp_dir().join(format!("agentsi-pill-link-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("core.sock");
    // SAFETY: this is the only test of this process, and no other thread runs yet.
    unsafe { std::env::set_var("AGENTSI_CORE_SOCKET", &socket) };
    let listener = UnixListener::bind(&socket).unwrap();
    let (seen_tx, seen) = mpsc::channel();
    let events = vec![
        json!({"jsonrpc": "2.0", "method": "terminal.exit", "params": {"id": "t1", "code": 0}}),
        json!({"jsonrpc": "2.0", "method": "voice.speak", "params": {"terminal": "t1", "role": "pm", "text": "Hello."}}),
        json!({"jsonrpc": "2.0", "method": "voice.stop", "params": {"terminal": "t1"}}),
    ];
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let (seen, events) = (seen_tx.clone(), events.clone());
            std::thread::spawn(move || serve(stream.unwrap(), seen, events));
        }
    });

    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    link::spawn(tx.clone());
    let mut next = || futures::executor::block_on(within_10_seconds(rx.next())).flatten();
    assert_eq!(
        next(),
        Some(LinkEvent::Input(Input::Speak {
            terminal: "t1".into(),
            role: "pm".into(),
            text: "Hello.".into()
        }))
    );
    assert_eq!(
        next(),
        Some(LinkEvent::Input(Input::CoreStop {
            terminal: "t1".into()
        }))
    );

    let hello = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(hello["method"], "core.hello");
    assert_eq!(hello["params"]["client"], "agentsi-pill");
    let subscribe = seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(subscribe["params"]["topics"], json!(["voice", "dictation"]));

    link::stop_recording(tx);
    let mut methods = Vec::new();
    while let Ok(request) = seen.recv_timeout(Duration::from_secs(5)) {
        let done = request["method"] == "voice.dictate";
        if done {
            assert_eq!(request["params"], json!({"action": "stop"}));
        }
        methods.push(request["method"].as_str().unwrap().to_owned());
        if done {
            break;
        }
    }
    assert_eq!(methods, ["core.hello", "voice.dictate"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Waits for a future for 10 s, with a helper thread as the timer, so the test needs no async runtime.
async fn within_10_seconds<F: std::future::Future>(f: F) -> Option<F::Output> {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        let _ = tx.send(());
    });
    futures::pin_mut!(f);
    match futures::future::select(f, rx).await {
        futures::future::Either::Left((v, _)) => Some(v),
        futures::future::Either::Right(_) => None,
    }
}
