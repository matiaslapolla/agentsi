//! The ticket tools of `agentsi mcp` on a real core: a session adds a ticket with the
//! author id of `AGENTSI_TERMINAL`, lists, reads and moves it, and gets `conflict` when it
//! drops a ticket of the owner. Each session is one `agentsi mcp` process on stdio.

use agentsi_proto::client::Client;
use agentsi_proto::core_api::{Empty, Shutdown};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const REGISTRY: &str = r#"{"owner":{"name":"Ada","language":"English"},"products":[
  {"id":"alpha","name":"Alpha","paths":["/work/alpha"],"status":"active","roles":["pm","dev"]}]}"#;

struct Work {
    dir: tempfile::TempDir,
    socket: PathBuf,
    core: PathBuf,
}

impl Drop for Work {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.socket) {
            let _ = c.call::<Shutdown>(&Empty {});
        }
    }
}

fn core_program() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo"));
    // The build folder may hold a core of another checkout: mark the sources as changed.
    let main = manifest.join("../agentsi-core/src/main.rs");
    let file = std::fs::OpenOptions::new().write(true).open(&main).unwrap();
    file.set_modified(std::time::SystemTime::now()).unwrap();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "-q", "-p", "agentsi-core", "--bins"])
        .status()
        .expect("cargo runs");
    assert!(status.success(), "cargo build -p agentsi-core");
    Path::new(env!("CARGO_BIN_EXE_agentsi"))
        .parent()
        .unwrap()
        .join("agentsi-core")
}

fn root(w: &Work) -> PathBuf {
    w.dir.path().join("root")
}

/// A data root with one ticket of the owner, `C001`.
fn work() -> Work {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    let mut conn = agentsi_store::db::open_writer(&agentsi_store::paths::db_path(&root)).unwrap();
    agentsi_store::db::migrate(&mut conn).unwrap();
    conn.execute_batch(
        r#"insert into tickets (product, id, kind, state, title, author, updated_by, created_at, updated_at)
           values ('alpha', 'C001', 'todo', 'open', 'Of the owner', '{"by":"owner","id":"Ada"}',
                   '{"by":"owner","id":"Ada"}', '2026-11-09T10:00:00.000Z', '2026-11-09T10:00:00.000Z');"#,
    )
    .unwrap();
    Work {
        socket: dir.path().join("run/core.sock"),
        core: core_program(),
        dir,
    }
}

/// One MCP session: the requests go in on stdin, and the answers come back by id.
fn session(w: &Work, env: &[(&str, &str)], calls: &[(&str, Value)]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentsi"))
        .arg("mcp")
        .current_dir(w.dir.path())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", w.dir.path())
        .env("AGENTSI_HOME", root(w))
        .env("AGENTSI_CORE_SOCKET", &w.socket)
        .env("AGENTSI_CORE_BIN", &w.core)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = String::from(
        "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\"}}\n",
    );
    for (k, (name, args)) in calls.iter().enumerate() {
        let request = json!({"jsonrpc": "2.0", "id": k + 1, "method": "tools/call",
                             "params": {"name": name, "arguments": args}});
        input.push_str(&format!("{request}\n"));
    }
    input.push_str("{\"jsonrpc\":\"2.0\",\"id\":999,\"method\":\"tools/list\"}\n");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let mut answers: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    answers.sort_by_key(|a| a["id"].as_i64().unwrap());
    assert_eq!(answers.len(), calls.len() + 2, "{answers:?}");
    answers
}

fn text(answer: &Value) -> (&str, bool) {
    (
        answer["result"]["content"][0]["text"].as_str().unwrap(),
        answer["result"]["isError"] == true,
    )
}

fn author(w: &Work, id: &str) -> Value {
    let conn = rusqlite::Connection::open(agentsi_store::paths::db_path(&root(w))).unwrap();
    let text: String = conn
        .query_row(
            "select author from tickets where product = 'alpha' and id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn a_session_writes_its_tickets_and_cannot_drop_a_ticket_of_the_owner() {
    let w = work();
    let a = session(
        &w,
        &[
            ("AGENTSI_TERMINAL", "t-42"),
            ("CLAUDE_CODE_SESSION_ID", "c-1"),
        ],
        &[
            (
                "add_ticket",
                json!({"product": "alpha", "kind": "todo", "title": "From the pane",
                       "links": [{"type": "commit", "ref": "1a2b3c4"}]}),
            ),
            ("list_tickets", json!({"product": "alpha"})),
            ("get_ticket", json!({"product": "alpha", "id": "C002"})),
            (
                "update_ticket",
                json!({"product": "alpha", "id": "C001", "state": "dropped"}),
            ),
            (
                "update_ticket",
                json!({"product": "alpha", "id": "C002", "waiting_on": "CI", "assignee": "dev"}),
            ),
            (
                "add_ticket",
                json!({"product": "alpha", "kind": "note", "title": "Bad link",
                       "links": [{"type": "story", "ref": "S404"}]}),
            ),
            (
                "list_tickets",
                json!({"product": "alpha", "state": "waiting", "limit": 5}),
            ),
        ],
    );
    assert_eq!(text(&a[1]), ("Added C002 [todo] From the pane", false));
    let (list, err) = text(&a[2]);
    assert!(!err);
    assert_eq!(
        list,
        "Tickets of alpha (2):\nC002 [todo] From the pane\nC001 [todo] Of the owner"
    );
    let (md, err) = text(&a[3]);
    assert!(!err && md.starts_with("# C002 From the pane\n"), "{md}");
    assert!(
        md.contains("- Author: session t-42 · Last write: session t-42\n"),
        "{md}"
    );
    assert!(md.contains("- Links: commit 1a2b3c4\n"), "{md}");
    assert_eq!(text(&a[4]), ("Ticket C001 belongs to the owner.", true));
    assert_eq!(
        text(&a[5]),
        ("C002 [todo, waiting on CI] From the pane (for dev)", false)
    );
    let (bad, err) = text(&a[6]);
    assert!(err && bad.contains("S404"), "{bad}");
    assert_eq!(
        text(&a[7]),
        (
            "Tickets of alpha (1):\nC002 [todo, waiting on CI] From the pane (for dev)",
            false
        )
    );
    let names: Vec<&str> = a[8]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "search",
            "get_doc",
            "list_minutes",
            "pending_decisions",
            "role_memory",
            "list_tickets",
            "get_ticket",
            "add_ticket",
            "update_ticket"
        ]
    );
    assert_eq!(author(&w, "C002"), json!({"by": "session", "id": "t-42"}));

    // Another session cannot touch the ticket of t-42. A session with no id is `unknown`,
    // which matches no ticket, not even its own.
    let b = session(
        &w,
        &[("CLAUDE_CODE_SESSION_ID", "c-9")],
        &[(
            "update_ticket",
            json!({"product": "alpha", "id": "C002", "state": "done"}),
        )],
    );
    assert_eq!(text(&b[1]), ("Ticket C002 belongs to the owner.", true));
    let c = session(
        &w,
        &[],
        &[
            (
                "add_ticket",
                json!({"product": "alpha", "kind": "question", "title": "Who?"}),
            ),
            (
                "update_ticket",
                json!({"product": "alpha", "id": "C003", "title": "Mine?"}),
            ),
            ("update_ticket", json!({"product": "alpha", "id": "C003"})),
        ],
    );
    assert_eq!(text(&c[1]), ("Added C003 [question] Who?", false));
    assert_eq!(text(&c[2]), ("Ticket C003 belongs to the owner.", true));
    assert_eq!(text(&c[3]), ("Nothing to change in ticket C003.", true));
    assert_eq!(
        author(&w, "C003"),
        json!({"by": "session", "id": "unknown"})
    );
}

#[test]
fn a_refused_move_with_fields_changes_nothing() {
    let w = work();
    let a = session(
        &w,
        &[("AGENTSI_TERMINAL", "t-7")],
        &[
            (
                "add_ticket",
                json!({"product": "alpha", "kind": "todo", "title": "Before"}),
            ),
            (
                "update_ticket",
                json!({"product": "alpha", "id": "C002", "state": "done"}),
            ),
            (
                "update_ticket",
                json!({"product": "alpha", "id": "C002", "title": "After", "state": "waiting"}),
            ),
            ("get_ticket", json!({"product": "alpha", "id": "C002"})),
        ],
    );
    assert_eq!(text(&a[2]), ("C002 [todo] Before", false));
    assert_eq!(
        text(&a[3]),
        ("Ticket C002 cannot move from done to waiting.", true)
    );
    let (md, err) = text(&a[4]);
    assert!(!err && md.starts_with("# C002 Before\n"), "{md}");
}
