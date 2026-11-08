//! `reflect.*` and `memory.*` on a real core with a fake `claude`: the results, the
//! errors, the `data.changed` events and the index. The rows against the TypeScript
//! CLI are in the golden test of `agentsi-cli`.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FAKE: &str = r#"#!/bin/sh
here=$(dirname "$0")
cat > /dev/null
: > "$here/started"
while [ -f "$here/hold" ]; do sleep 0.02; done
printf '{"type":"result","subtype":"success","total_cost_usd":0.125,"duration_ms":1,"structured_output":%s}\n' "$(cat "$here/answer.json")"
"#;

const ANSWER: &str = r#"{"add":[{"text":"Cite what you read.","kind":"lesson","evidence":["m1"]}],"retire":[{"id":"m-a1","why":"Stale."}],"helpful":[],"harmful":[]}"#;

struct World {
    home: PathBuf,
    bin: PathBuf,
}

fn setup(dir: &TempDir) -> World {
    let home = dir.0.join("root");
    let bin = dir.0.join("bin");
    let repo = dir.0.join("repo");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    let registry = json!({
        "owner": {"name": "Ada", "language": "English"},
        "products": [{"id": "alpha", "name": "Alpha", "paths": [repo], "status": "active", "roles": ["pm", "dev"]}],
    });
    std::fs::write(home.join("agentsi.config.json"), registry.to_string()).unwrap();
    let file = bin.join("claude");
    std::fs::write(&file, FAKE).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(bin.join("answer.json"), ANSWER).unwrap();
    let conn = agentsi_store::db::open_writer(&home.join("agentsi.db")).unwrap();
    let minutes = json!({"id": "m1", "product": "alpha", "status": "valid", "question": "q",
        "created_at": "2026-10-01T10:00:00.000Z", "roles": {}, "decisions": [],
        "priorities": [{"title": "A", "owner_role": "dev", "mark": "rejected"}]});
    conn.execute_batch(&format!(
        "insert into products (id, name, paths, status) values ('alpha', 'Alpha', '[]', 'active');
         insert into minutes (id, product, status, created_at, data)
           values ('m1', 'alpha', 'valid', '2026-10-01T10:00:00.000Z', '{minutes}');
         insert into memory (product, agent, id, position, text, kind, helpful, harmful, source, created_at)
           values ('alpha', 'dev', 'm-a1', 0, 'Old note.', 'fact', 0, 0, 'seed', '2026-10-01T10:00:00.000Z'),
                  ('alpha', 'dev', 'm-a2', 1, 'Keep this.', 'fact', 0, 0, 'seed', '2026-10-02T10:00:00.000Z');"
    ))
    .unwrap();
    World { home, bin }
}

fn start(dir: &TempDir, w: &World) -> CoreProc {
    let mut path = OsString::from(w.bin.as_os_str());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    CoreProc::start_with(
        dir,
        &[
            ("AGENTSI_HOME", w.home.as_path()),
            ("PATH", Path::new(&path)),
        ],
    )
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

fn memory_doc(home: &Path) -> Option<String> {
    let c = rusqlite::Connection::open(home.join("agentsi.db")).unwrap();
    c.query_row(
        "select body from documents where kind = 'memory' and ref = 'dev'",
        [],
        |r| r.get(0),
    )
    .ok()
}

fn wait_for(file: &Path) {
    let start = Instant::now();
    while !file.exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "no call started");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_list_and_retire_change_the_memory_and_the_index() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let r = c.ok("memory.list", json!({"product": "alpha"}));
    let names: Vec<&str> = r["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["agent"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["pm", "dev"]);
    assert_eq!(r["agents"][1]["items"][0]["id"], "m-a2", "newest first");

    let r = c.ok(
        "memory.retire",
        json!({"product": "alpha", "agent": "dev", "id": "m-a1", "why": "Stale."}),
    );
    assert_eq!(r["item"]["retired_why"], "Stale.");
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "alpha", "kind": "memory"})
    );
    let doc = memory_doc(&w.home).unwrap();
    assert!(
        doc.contains("Keep this.") && !doc.contains("Old note."),
        "{doc}"
    );

    let again = json!({"product": "alpha", "agent": "dev", "id": "m-a1", "why": "x"});
    assert_eq!(kind(&c.call("memory.retire", again)), "conflict");
    let unknown = json!({"product": "alpha", "agent": "dev", "id": "m-zz", "why": "x"});
    assert_eq!(kind(&c.call("memory.retire", unknown)), "not_found");
    let product = json!({"product": "nope"});
    assert_eq!(kind(&c.call("memory.list", product)), "not_found");
}

#[test]
fn a_reflection_and_its_revert_update_the_index_and_send_events() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let r = c.ok(
        "reflect.run",
        json!({"product": "alpha", "memory_only": true}),
    );
    let id = r["entry"]["id"].as_str().unwrap().to_string();
    assert_eq!(r["entry"]["memory"][0]["retired"], json!(["m-a1"]));
    assert_eq!(r["candidates"], json!([]));
    assert_eq!(c.next_event("data.changed")["kind"], "memory");
    let doc = memory_doc(&w.home).unwrap();
    assert!(
        doc.contains("Cite what you read.") && !doc.contains("Old note."),
        "{doc}"
    );

    let r = c.ok(
        "reflect.revert",
        json!({"product": "alpha", "reflection": id}),
    );
    assert_eq!(r["entry"]["reverts"], json!(id));
    assert_eq!(c.next_event("data.changed")["kind"], "memory");
    let doc = memory_doc(&w.home).unwrap();
    assert!(
        doc.contains("Old note.") && !doc.contains("Cite what you read."),
        "{doc}"
    );

    let listed = c.ok("memory.list", json!({"product": "alpha"}));
    assert_eq!(listed["reflections"].as_array().unwrap().len(), 2);
    assert_eq!(
        listed["reflections"][0]["reverts"],
        json!(id),
        "newest first"
    );
    assert_eq!(listed["reflections"][1]["status"], "reverted");

    let twice = json!({"product": "alpha", "reflection": id});
    assert_eq!(kind(&c.call("reflect.revert", twice)), "conflict");
    let unknown = json!({"product": "alpha", "reflection": "r-nope"});
    assert_eq!(kind(&c.call("reflect.revert", unknown)), "not_found");
}

#[test]
fn a_second_reflection_waits_and_a_cancel_writes_nothing() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let (hold, started) = (w.bin.join("hold"), w.bin.join("started"));
    std::fs::write(&hold, "").unwrap();

    let socket = core.socket.path.clone();
    let run = std::thread::spawn(move || {
        let mut c = Framed::connect(&socket);
        c.call(
            "reflect.run",
            json!({"product": "alpha", "memory_only": true}),
        )
    });
    wait_for(&started);

    let mut c = Framed::connect(&core.socket.path);
    let second = c.call("reflect.run", json!({"product": "alpha"}));
    assert_eq!(kind(&second), "conflict", "{second}");
    let other = c.ok("reflect.cancel", json!({"product": "beta"}));
    assert_eq!(other, json!({"canceled": false}));
    let done = c.ok("reflect.cancel", json!({"product": "alpha"}));
    assert_eq!(done, json!({"canceled": true}));

    let reply = run.join().unwrap();
    assert_eq!(kind(&reply), "conflict", "{reply}");
    let listed = c.ok("memory.list", json!({"product": "alpha"}));
    assert_eq!(listed["reflections"], json!([]));
    assert_eq!(listed["agents"][1]["items"][1]["retired_at"], Value::Null);
    std::fs::remove_file(&hold).unwrap();
}

#[test]
fn a_core_that_stops_cancels_the_reflection_and_writes_nothing() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let mut core = start(&dir, &w);
    let (hold, started) = (w.bin.join("hold"), w.bin.join("started"));
    std::fs::write(&hold, "").unwrap();

    let socket = core.socket.path.clone();
    let run = std::thread::spawn(move || {
        let mut c = Framed::connect(&socket);
        c.call(
            "reflect.run",
            json!({"product": "alpha", "memory_only": true}),
        )
    });
    wait_for(&started);
    // SAFETY: the pid of the core that this test started.
    unsafe { libc::kill(core.pid(), libc::SIGTERM) };
    let reply = run.join().unwrap();
    assert_eq!(kind(&reply), "conflict", "{reply}");
    let child = core.child.as_mut().unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "the core still runs"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let db = rusqlite::Connection::open(w.home.join("agentsi.db")).unwrap();
    let rows: i64 = db
        .query_row("select count(*) from reflections", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    std::fs::remove_file(&hold).unwrap();
}
