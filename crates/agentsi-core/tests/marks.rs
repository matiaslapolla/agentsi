//! `mark.*`, `decision.*` and `archive.*` on a real core: the results, the
//! errors and the `data.changed` event. The rows against the TypeScript CLI
//! are in the golden test of `agentsi-cli`.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};

const REGISTRY: &str = r#"{"owner":{"name":"Ada","language":"English"},"products":[
  {"id":"alpha","name":"Alpha","paths":["/work/alpha"],"status":"active","roles":["pm"]}]}"#;

const MINUTES: &str = r#"{"id":"m1","product":"alpha","status":"valid","priorities":[
  {"title":"a","why":"w","owner_role":"pm","objective":null,"sources":[],"mark":null,"is_new":false},
  {"title":"b","why":"w","owner_role":"pm","objective":null,"sources":[],"mark":null,"is_new":false}],
  "decisions":[{"text":"go","why":"w","priorities":[1]}]}"#;

fn setup(dir: &TempDir) -> std::path::PathBuf {
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    conn.execute_batch(&format!(
        "insert into products (id, name, paths, status) values ('alpha', 'Alpha', '[]', 'active');
         insert into minutes (id, product, status, created_at, data) values ('m1', 'alpha', 'valid', '2026-10-01T10:00:00.000Z', '{}');
         insert into priorities (minutes_id, idx, title) values ('m1', 0, 'a'), ('m1', 1, 'b');
         insert into decisions (id, product, minutes_id, text, why, status, kind, action)
           values (1, 'alpha', 'm1', 'go', 'w', 'pending', 'minutes', 'Run it'),
                  (2, 'alpha', null, 'reflect', 'w', 'pending', 'reflection', null);",
        MINUTES.replace('\n', "")
    ))
    .unwrap();
    root
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

#[test]
fn the_owner_marks_decides_adds_and_archives_and_each_write_sends_an_event() {
    let dir = TempDir::new();
    let home = setup(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let r = c.ok("mark.pending", json!({"product": "alpha"}));
    let kinds: Vec<String> = r["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| format!("{}", i["kind"]))
        .collect();
    assert_eq!(
        kinds,
        [
            "\"priority\"",
            "\"priority\"",
            "\"decision\"",
            "\"decision\""
        ]
    );
    assert_eq!(r["items"][2]["priorities"], json!([1]));

    let r = c.ok(
        "mark.set",
        json!({"entries": [{"minutes_id": "m1", "idx": 0, "mark": "changed", "is_new": true}]}),
    );
    assert_eq!(r, json!({"marked": 1}));
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "alpha", "kind": "minutes"})
    );
    let r = c.call(
        "mark.set",
        json!({"entries": [{"minutes_id": "m1", "idx": 0, "mark": "rejected"}], "unmarked_only": true}),
    );
    assert_eq!(kind(&r), "conflict", "{r}");

    let r = c.ok(
        "decision.decide",
        json!({"decision": 1, "status": "approved"}),
    );
    assert_eq!(
        r,
        json!({"product": "alpha", "minutes_id": "m1", "status": "approved", "action": "Run it"})
    );
    assert_eq!(c.next_event("data.changed")["kind"], "decision");
    assert_eq!(c.next_event("data.changed")["kind"], "minutes");
    let r = c.call(
        "decision.decide",
        json!({"decision": 2, "status": "approved"}),
    );
    assert_eq!(kind(&r), "unavailable", "{r}");
    let r = c.call(
        "decision.decide",
        json!({"decision": 77, "status": "approved"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");

    let r = c.ok(
        "decision.add",
        json!({"product": "alpha", "text": " Pick a name ", "why": "It is open", "action": ""}),
    );
    assert_eq!(r, json!({"id": 3}));
    assert_eq!(c.next_event("data.changed")["kind"], "decision");
    let r = c.call(
        "decision.add",
        json!({"product": "alpha", "text": " ", "why": "x"}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call(
        "decision.add",
        json!({"product": "nope", "text": "t", "why": "w"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");

    c.ok(
        "archive.archive",
        json!({"product": "alpha", "kind": "decision", "id": "3"}),
    );
    assert_eq!(c.next_event("data.changed")["kind"], "decision");
    let r = c.call(
        "decision.decide",
        json!({"decision": 3, "status": "approved"}),
    );
    assert_eq!(kind(&r), "conflict", "{r}");
    assert_eq!(
        r["error"]["message"],
        "Decision 3 is archived. Run agentsi restore 3 first, then decide."
    );
    let r = c.call(
        "archive.archive",
        json!({"product": "alpha", "kind": "decision", "id": "3"}),
    );
    assert_eq!(r["error"]["message"], "Decision 3 is already archived.");
    c.ok(
        "archive.restore",
        json!({"product": "alpha", "kind": "decision", "id": "3"}),
    );
    c.ok(
        "archive.archive",
        json!({"product": "alpha", "kind": "minutes", "id": "m1"}),
    );
    let r = c.call(
        "archive.restore",
        json!({"product": "alpha", "kind": "minutes", "id": "zz"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
    let r = c.call(
        "archive.archive",
        json!({"product": "alpha", "kind": "minutes"}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
}

#[test]
fn a_data_root_without_a_database_is_unavailable() {
    let dir = TempDir::new();
    let home = dir.0.join("root");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("agentsi.config.json"), REGISTRY).unwrap();
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    let r = c.call("mark.set", json!({"entries": []}));
    assert_eq!(kind(&r), "unavailable", "{r}");
    let r = c.call("mark.pending", json!({"product": "alpha"}));
    assert_eq!(kind(&r), "unavailable", "{r}");
}

/// B7 of the U27 review: the app reads its product list again at this event.
#[test]
fn archiving_and_restoring_a_product_each_send_data_changed_of_kind_product() {
    let dir = TempDir::new();
    let home = setup(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));
    for method in ["archive.archive", "archive.restore"] {
        c.ok(method, json!({"product": "alpha", "kind": "product"}));
        let event = c.next_event("data.changed");
        assert_eq!(
            (event["product"].as_str(), event["kind"].as_str()),
            (Some("alpha"), Some("product"))
        );
        let listed = c.ok("product.list", json!({}));
        assert_eq!(
            listed["products"][0]["archived"].as_bool().unwrap_or(false),
            method == "archive.archive"
        );
    }
}
