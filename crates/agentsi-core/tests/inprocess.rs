//! The server in this test process, on a real socket in a temp folder. The
//! test can publish events and choose the owner uid.

mod common;

use agentsi_core::server::{bind, euid};
use agentsi_core::state::Core;
use agentsi_proto::events::{
    DataChanged, DataChangedParams, ProgramState, TerminalState, TerminalStateParams,
};
use common::TempDir;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

struct Served {
    core: Arc<Core>,
    socket: std::path::PathBuf,
    task: tokio::task::JoinHandle<std::io::Result<agentsi_core::server::HeldLock>>,
    _dir: TempDir,
}

impl Served {
    async fn start(owner_uid: u32) -> Served {
        let dir = TempDir::new();
        let socket = dir.socket();
        let bound = bind(&socket).unwrap();
        let core = Core::new();
        let task = tokio::spawn(bound.serve(core.clone(), owner_uid, std::future::pending()));
        Served {
            core,
            socket: socket.path,
            task,
            _dir: dir,
        }
    }

    async fn stop(self) {
        self.core.ask_stop();
        drop(self.task.await.unwrap().unwrap());
        assert!(!self.socket.exists());
    }
}

struct Line {
    r: BufReader<OwnedReadHalf>,
    w: OwnedWriteHalf,
}

impl Line {
    async fn connect(socket: &std::path::Path) -> Line {
        let (r, w) = UnixStream::connect(socket).await.unwrap().into_split();
        Line {
            r: BufReader::new(r),
            w,
        }
    }

    async fn send(&mut self, line: &str) {
        self.w
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn next(&mut self) -> Option<Value> {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(5), self.r.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        (n > 0).then(|| serde_json::from_str(&line).unwrap())
    }

    async fn call(&mut self, request: Value) -> Value {
        self.send(&request.to_string()).await;
        self.next().await.unwrap()
    }
}

fn state(id: &str) -> TerminalStateParams {
    TerminalStateParams {
        id: id.into(),
        state: ProgramState::Blocked,
        kind: Some("permission".into()),
        msg: None,
    }
}

#[tokio::test]
async fn a_peer_with_another_uid_is_closed_before_any_reply() {
    let served = Served::start(euid() + 1).await;
    let mut stream = UnixStream::connect(&served.socket).await.unwrap();
    stream
        .write_all(format!("{}\n", common::HELLO).as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
        .await
        .unwrap();
    assert!(
        read.is_err() || buf.is_empty(),
        "got {:?}",
        String::from_utf8_lossy(&buf)
    );
    served.stop().await;
}

#[tokio::test]
async fn subscribe_sends_the_events_of_its_topics_only() {
    let served = Served::start(euid()).await;
    let mut c = Line::connect(&served.socket).await;
    let reply = c.call(json!({"jsonrpc": "2.0", "id": 1, "method": "core.subscribe", "params": {"topics": ["terminal"]}})).await;
    assert_eq!(reply["result"]["topics"], json!(["terminal"]));

    served.core.publish::<DataChanged>(&DataChangedParams {
        product: "p".into(),
        kind: "minutes".into(),
    });
    served.core.publish::<TerminalState>(&state("t1"));
    let event = c.next().await.unwrap();
    assert_eq!(
        event,
        json!({"jsonrpc": "2.0", "method": "terminal.state", "params": {"id": "t1", "state": "blocked", "kind": "permission"}})
    );

    // A second call adds a topic. Requests still work on the connection.
    let reply = c.call(json!({"jsonrpc": "2.0", "id": 2, "method": "core.subscribe", "params": {"topics": ["data"]}})).await;
    assert_eq!(reply["result"]["topics"], json!(["terminal", "data"]));
    served.core.publish::<DataChanged>(&DataChangedParams {
        product: "p".into(),
        kind: "story".into(),
    });
    assert_eq!(c.next().await.unwrap()["method"], "data.changed");
    assert_eq!(
        c.call(json!({"jsonrpc": "2.0", "id": 3, "method": "core.hello"}))
            .await["id"],
        3
    );
    served.stop().await;
}

#[tokio::test]
async fn a_subscriber_that_falls_behind_gets_core_lagged() {
    let served = Served::start(euid()).await;
    let mut c = Line::connect(&served.socket).await;
    c.call(json!({"jsonrpc": "2.0", "id": 1, "method": "core.subscribe", "params": {"topics": ["terminal"]}})).await;
    // The test does not yield, so the core cannot send while it publishes.
    for i in 0..1500 {
        served
            .core
            .publish::<TerminalState>(&state(&format!("t{i}")));
    }
    // The queue keeps the first 1024 events. The notice comes where the
    // events are missing, and the next event comes after it.
    for i in 0..1024 {
        assert_eq!(c.next().await.unwrap()["params"]["id"], format!("t{i}"));
    }
    let lagged = c.next().await.unwrap();
    assert_eq!(lagged["method"], "core.lagged");
    assert_eq!(lagged["params"]["missed"], 1500 - 1024);
    served.core.publish::<TerminalState>(&state("after"));
    assert_eq!(c.next().await.unwrap()["params"]["id"], "after");
    served.stop().await;
}

/// The lag of a connection counts the events of its own topics only.
#[tokio::test]
async fn events_of_other_topics_do_not_make_a_connection_lag() {
    let served = Served::start(euid()).await;
    let mut c = Line::connect(&served.socket).await;
    c.call(json!({"jsonrpc": "2.0", "id": 1, "method": "core.subscribe", "params": {"topics": ["terminal"]}})).await;
    for _ in 0..1500 {
        served.core.publish::<DataChanged>(&DataChangedParams {
            product: "p".into(),
            kind: "minutes".into(),
        });
    }
    served.core.publish::<TerminalState>(&state("t1"));
    assert_eq!(c.next().await.unwrap()["method"], "terminal.state");
    served.stop().await;
}

/// B2: after the reply to `core.hello` with framed: true, each byte is a frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_event_line_comes_after_the_reply_that_starts_framed_mode() {
    let served = Served::start(euid()).await;
    let mut c = Line::connect(&served.socket).await;
    c.call(json!({"jsonrpc": "2.0", "id": 1, "method": "core.subscribe", "params": {"topics": ["terminal"]}})).await;
    let big = TerminalStateParams {
        msg: Some("m".repeat(10_000)),
        ..state("t")
    };
    for _ in 0..1000 {
        served.core.publish::<TerminalState>(&big);
    }
    c.send(r#"{"jsonrpc":"2.0","id":2,"method":"core.hello","params":{"framed":true}}"#)
        .await;
    loop {
        let message = c.next().await.unwrap();
        if message["id"] == 2 {
            break;
        }
        assert_eq!(message["method"], "terminal.state");
    }
    let mut header = [0u8; agentsi_proto::frame::HEADER_LEN];
    c.r.read_exact(&mut header).await.unwrap();
    assert_eq!(
        &header[..5],
        &[0, 0, 0, 0, 0],
        "the next bytes after the reply: {:?}",
        String::from_utf8_lossy(&header)
    );
    served.stop().await;
}

#[tokio::test]
async fn each_error_has_a_stable_kind_and_a_hint() {
    let served = Served::start(euid()).await;
    let mut c = Line::connect(&served.socket).await;
    let cases = [
        (
            json!({"jsonrpc": "2.0", "id": 1, "method": "no.such"}).to_string(),
            -32601,
            "not_found",
            json!(1),
        ),
        (
            json!({"jsonrpc": "2.0", "id": 2, "method": "core.subscribe", "params": {}})
                .to_string(),
            -32602,
            "invalid",
            json!(2),
        ),
        (
            json!({"jsonrpc": "2.0", "id": 3, "method": "core.hello", "params": [true]})
                .to_string(),
            -32602,
            "invalid",
            json!(3),
        ),
        (
            json!({"jsonrpc": "1.0", "id": 4, "method": "core.hello"}).to_string(),
            -32600,
            "invalid",
            json!(4),
        ),
        (
            json!([{"jsonrpc": "2.0", "id": 5, "method": "core.hello"}]).to_string(),
            -32600,
            "invalid",
            Value::Null,
        ),
        ("{not json".to_string(), -32700, "invalid", Value::Null),
    ];
    for (line, code, kind, id) in cases {
        c.send(&line).await;
        let reply = c.next().await.unwrap();
        assert_eq!(reply["id"], id, "{line}");
        assert_eq!(reply["error"]["code"], code, "{line}");
        assert_eq!(reply["error"]["data"]["kind"], kind, "{line}");
        assert!(
            !reply["error"]["data"]["hint"].as_str().unwrap().is_empty(),
            "{line}"
        );
    }
    // A request without an id gets no reply.
    c.send(r#"{"jsonrpc":"2.0","method":"core.hello"}"#).await;
    assert_eq!(
        c.call(json!({"jsonrpc": "2.0", "id": "s", "method": "core.hello"}))
            .await["id"],
        "s"
    );
    served.stop().await;
}

#[tokio::test]
async fn rpc_discover_gives_the_schema_of_the_file() {
    let served = Served::start(euid()).await;
    let mut c = Line::connect(&served.socket).await;
    let reply = c
        .call(json!({"jsonrpc": "2.0", "id": 1, "method": "rpc.discover"}))
        .await;
    // Read at run time: the shared CARGO_TARGET_DIR reuses this binary across worktrees, so a compile-time path can point at another worktree.
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let file = std::fs::read_to_string(format!("{dir}/../../schema/core.openrpc.json")).unwrap();
    assert_eq!(
        reply["result"],
        serde_json::from_str::<Value>(&file).unwrap()
    );
    served.stop().await;
}
