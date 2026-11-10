//! The show tools of `agentsi mcp` on a real core. With `AGENTSI_DESK_SESSION`, the server
//! lists the `show_*` tools after the ticket tools, sends `desk.show`, and gives an error result
//! for a bad call. A desk session cannot write tickets. Without the variable, nothing changes.

use agentsi_proto::client::Client;
use agentsi_proto::core_api::{Empty, Shutdown};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const REGISTRY: &str = r#"{"owner":{"name":"Ada","language":"English"},"products":[
  {"id":"alpha","name":"Alpha","paths":["/work/alpha"],"status":"active","roles":["pm"]}]}"#;

const SESSION: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";

const BASE: [&str; 9] = [
    "search",
    "get_doc",
    "list_minutes",
    "pending_decisions",
    "role_memory",
    "list_tickets",
    "get_ticket",
    "add_ticket",
    "update_ticket",
];

/// The show tools, in the order of `View`.
const SHOW: [&str; 9] = [
    "show_decisions",
    "show_priorities",
    "show_story",
    "show_diff",
    "show_table",
    "show_chart",
    "show_checklist",
    "show_page",
    "show_tickets",
];

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

/// A data root with one desk session of `alpha` and one decision.
fn work() -> Work {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    let mut conn = agentsi_store::db::open_writer(&agentsi_store::paths::db_path(&root)).unwrap();
    agentsi_store::db::migrate(&mut conn).unwrap();
    conn.execute_batch(&format!(
        "insert into decisions (id, product, text, why, status, kind) values (1, 'alpha', 'Ship it', 'w', 'pending', 'product');
         insert into desk_sessions values ('alpha', '{SESSION}', '2026-11-09T10:00:00.000Z');"
    ))
    .unwrap();
    Work {
        socket: dir.path().join("run/core.sock"),
        core: core_program(),
        dir,
    }
}

/// One MCP session: the requests go in on stdin, and the answers come back by id. The last
/// answer is `tools/list`.
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

fn names(list: &Value) -> Vec<&str> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect()
}

fn view_items(w: &Work) -> Vec<String> {
    let conn = rusqlite::Connection::open(agentsi_store::paths::db_path(&root(w))).unwrap();
    let mut stmt = conn
        .prepare(
            "select body from desk_items where product = 'alpha' and kind = 'view' order by seq",
        )
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn a_desk_session_shows_a_view_and_a_bad_call_is_an_error_result() {
    let w = work();
    let a = session(
        &w,
        &[("AGENTSI_DESK_SESSION", SESSION)],
        &[
            ("show_decisions", json!({"ids": [1]})),
            ("show_decisions", json!({"ids": [1, 77]})),
            ("show_decisions", json!({"ids": "x"})),
            ("show_decisions", json!({})),
            (
                "add_ticket",
                json!({"product": "alpha", "kind": "todo", "title": "No"}),
            ),
            ("show_story", json!({"id": "S999"})),
            (
                "show_table",
                json!({"title": "T", "columns": ["a"], "rows": [["1", "2"]]}),
            ),
        ],
    );
    assert_eq!(
        text(&a[1]),
        ("The view is in the desk conversation.", false)
    );
    let (missing, err) = text(&a[2]);
    assert!(err && missing.contains("77"), "{missing}");
    let (bad, err) = text(&a[3]);
    assert!(
        err && bad.starts_with("Invalid arguments of show_decisions"),
        "{bad}"
    );
    let (none, err) = text(&a[4]);
    assert!(err && none.contains("ids"), "{none}");
    let (refused, err) = text(&a[5]);
    assert!(err && refused.contains("only reads"), "{refused}");

    let (story, err) = text(&a[6]);
    assert!(err && story.contains("S999"), "{story}");

    let (row, err) = text(&a[7]);
    assert!(err && row.contains("Row 1 has 2 cells"), "{row}");

    let list = names(&a[8]);
    assert_eq!(list[..9], BASE);
    assert_eq!(list[9..], SHOW);
    let show = &a[8]["result"]["tools"][9];
    assert_eq!(show["inputSchema"]["required"], json!(["ids"]));
    assert!(
        show["description"]
            .as_str()
            .unwrap()
            .contains("pending decisions")
    );

    assert_eq!(
        view_items(&w),
        [r#"{"kind":"view","view":{"kind":"decisions","ids":[1]}}"#]
    );
    let conn = rusqlite::Connection::open(agentsi_store::paths::db_path(&root(&w))).unwrap();
    let tickets: i64 = conn
        .query_row("select count(*) from tickets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tickets, 0);
}

#[test]
fn an_unknown_session_is_an_error_and_no_session_lists_no_show_tool() {
    let w = work();
    let a = session(
        &w,
        &[("AGENTSI_DESK_SESSION", "not-a-session")],
        &[("show_decisions", json!({"ids": [1]}))],
    );
    let (unknown, err) = text(&a[1]);
    assert!(err && unknown.contains("not-a-session"), "{unknown}");
    assert!(view_items(&w).is_empty());

    let b = session(&w, &[], &[("show_decisions", json!({"ids": [1]}))]);
    assert_eq!(
        text(&b[1]),
        ("MCP error -32602: Tool show_decisions not found", true)
    );
    assert_eq!(names(&b[2]), BASE);
    let c = session(&w, &[("AGENTSI_DESK_SESSION", " ")], &[]);
    assert_eq!(names(&c[1]), BASE);
}
