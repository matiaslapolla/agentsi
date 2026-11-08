//! The flow of a meeting on a frozen snapshot, with a fake `claude`. The golden
//! cases of `agentsi-cli` compare the same flow with the TypeScript code.

use super::*;
use crate::registry::Registry;
use agentsi_proto::meeting_api::MeetingProgressParams;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

struct Recorder {
    events: Mutex<Vec<MeetingProgressParams>>,
    lines: Mutex<Vec<(String, String)>>,
}

impl Sink for Recorder {
    fn progress(&self, event: MeetingProgressParams) {
        self.events.lock().unwrap().push(event);
    }

    fn line(&self, role: &str, line: &str) {
        self.lines
            .lock()
            .unwrap()
            .push((role.to_string(), line.to_string()));
    }
}

struct World {
    dir: tempfile::TempDir,
    store: Store,
    registry: Registry,
    recorder: Recorder,
    program: String,
}

const REPLIES: &str = r##"#!/bin/sh
prompt=$(cat)
ok() { printf '{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}\n'
  printf '{"type":"result","subtype":"success","total_cost_usd":0.125,"duration_ms":10,"structured_output":%s}\n' "$1"; }
case "$prompt" in
*"You are the dev of"*)
  ok '{"answer":"Dev view.","priorities":[{"title":"Do it","why":"W","sources":[{"type":"path","ref":"docs/vision.md"},{"type":"path","ref":"docs/nothing.md"}]}],"concerns":[],"memory_notes":["The product ships in May."]}' ;;
*"You are the qa of"*) echo boom >&2; exit 1 ;;
*"# Repair"*)
  ok '{"priorities":[{"title":"Do it","why":"W","sources":[{"type":"path","ref":"docs/vision.md"}],"owner_role":"dev"}],"disagreements":[],"decisions":[{"text":"Go","why":"Y","priorities":[0]}],"script":[{"role":"pm","text":"Hello."}]}' ;;
*) ok '{"priorities":[{"title":"Do it","why":"W","sources":[{"type":"path","ref":"docs/invented.md"}],"owner_role":"dev"}],"disagreements":[],"decisions":[],"script":[]}' ;;
esac
"##;

fn world(reply: &str) -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(root.join("plugin/agents")).unwrap();
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/vision.md"), "v").unwrap();
    for role in ["pm", "dev", "qa"] {
        std::fs::write(
            root.join(format!("plugin/agents/{role}.md")),
            format!("---\nmodel: haiku\n---\nYou are {role}.\n"),
        )
        .unwrap();
    }
    let registry = Registry::parse(
        &serde_json::json!({
            "owner": {"name": "Ada", "language": "English"},
            "products": [{"id": "p", "name": "P", "paths": [repo], "status": "active", "roles": ["dev", "qa"]},
                         {"id": "old", "name": "Old", "paths": [repo], "status": "active", "roles": ["dev"]}],
        })
        .to_string(),
    )
    .unwrap();
    let store = Store::open(&root.join("agentsi.db")).unwrap();
    store
        .write(|tx| {
            tx.execute("insert into snapshots (product, minutes_id, body) values ('p', 'old-1', '# Snapshot\nfrozen')", [])?;
            tx.execute("insert into archive_records (product, kind, ref) values ('old', 'product', '')", [])?;
            Ok(())
        })
        .unwrap();
    let file = dir.path().join("fake-claude");
    std::fs::write(&file, reply).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    World {
        program: file.to_string_lossy().into_owned(),
        dir,
        store,
        registry,
        recorder: Recorder {
            events: Mutex::default(),
            lines: Mutex::default(),
        },
    }
}

fn options(product: &str) -> MeetingOptions {
    MeetingOptions {
        meeting: "t1".into(),
        product: product.into(),
        scope: None,
        question: "¿Qué sigue?".into(),
        roles: Vec::new(),
        frozen: Some("old-1".into()),
    }
}

fn run(w: &World, o: &MeetingOptions, cancel: &Cancel) -> Result<MeetingOutcome> {
    let root = w.dir.path().join("root");
    run_meeting(
        &Env {
            store: &w.store,
            root: &root,
            registry: &w.registry,
            sink: &w.recorder,
            cancel,
            claude: &w.program,
        },
        o,
    )
}

fn count(w: &World, table: &str) -> i64 {
    w.store
        .read(|c| Ok(c.query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))?))
        .unwrap()
}

#[test]
fn a_meeting_repairs_its_minutes_and_stores_all_its_parts() {
    let w = world(REPLIES);
    let out = run(&w, &options("p"), &Cancel::default()).unwrap();
    let m = &out.minutes;
    assert_eq!(m["status"], "invalid", "qa failed: {:?}", m["check"]);
    assert_eq!(m["check"]["repaired"], true);
    assert_eq!(
        m["check"]["failures"],
        serde_json::json!(["Role \"qa\" has no answer."])
    );
    let first = m["check"]["first_failures"].as_array().unwrap();
    assert!(first[0].as_str().unwrap().contains("docs/invented.md"));
    assert_eq!(
        m["roles"]["qa"]["concerns"][0]
            .as_str()
            .unwrap()
            .split(':')
            .next(),
        Some("Role call failed")
    );
    assert!(m["id"].as_str().unwrap().ends_with("-que-sigue"));

    let usage: Vec<&str> = out.usage.iter().map(|u| u.role.as_str()).collect();
    assert_eq!(usage, ["dev", "synthesis", "synthesis-repair"]);
    assert_eq!(out.total_cost_usd, 0.375);

    let id = m["id"].as_str().unwrap().to_string();
    let (entry, snapshot, stored, memory) = w
        .store
        .read(|c| {
            Ok((
                read::meeting_log(c, "p")?.remove(0),
                read::snapshot(c, "p", &id)?.unwrap(),
                read::minutes(c, &id)?.unwrap(),
                read::memory(c, "p", "dev")?,
            ))
        })
        .unwrap();
    assert_eq!(&stored, m);
    assert_eq!(snapshot, "# Snapshot\nfrozen");
    assert_eq!(entry["minutes_id"], m["id"]);
    assert_eq!(
        entry["agents"]["dev"]["unresolved_sources"],
        serde_json::json!([{"type": "path", "ref": "docs/nothing.md"}])
    );
    assert!(
        entry["agents"]["qa"]["error"]
            .as_str()
            .unwrap()
            .contains("boom")
    );
    assert_eq!(entry["synthesis"]["repaired"], true);
    assert_eq!(entry["synthesis"]["first_check"]["passed"], false);
    assert_eq!(entry["total_cost_usd"], 0.375);
    assert_eq!(memory[0].text, "The product ships in May.");
    assert_eq!(memory[0].source, id);
    assert_eq!(count(&w, "priorities"), 1);
    assert_eq!(count(&w, "decisions"), 1);
    assert_eq!(count(&w, "usage"), 3);
}

#[test]
fn the_events_tell_each_stage_in_order_and_end_with_done() {
    let w = world(REPLIES);
    run(&w, &options("p"), &Cancel::default()).unwrap();
    let events = w.recorder.events.lock().unwrap();
    let stages: Vec<String> = events
        .iter()
        .map(|e| format!("{}:{}", e.stage, e.phase))
        .collect();
    assert_eq!(stages[..2], ["start:start", "role:start"]);
    assert_eq!(stages.last().unwrap(), "done:end");
    assert_eq!(events[0].note.as_deref(), Some("¿Qué sigue?"));
    assert!(events.iter().all(|e| e.meeting == "t1" && e.product == "p"));
    let failed = events
        .iter()
        .find(|e| e.stage == "role" && e.phase == "end" && e.role.as_deref() == Some("qa"))
        .unwrap();
    assert!(failed.error.as_deref().unwrap().contains("boom"));
    let repair = events
        .iter()
        .find(|e| e.stage == "synthesis" && e.phase == "start" && e.note.is_some())
        .unwrap();
    assert_eq!(repair.role.as_deref(), Some("synthesis-repair"));
    let lines = w.recorder.lines.lock().unwrap();
    assert!(lines.iter().any(|(role, _)| role == "dev"));
    assert!(lines.iter().any(|(role, _)| role == "synthesis-repair"));
}

#[test]
fn an_archived_product_and_an_unknown_role_stop_before_any_call() {
    let w = world("#!/bin/sh\necho called >> \"$0.log\"\nexit 1\n");
    let e = run(&w, &options("old"), &Cancel::default()).err().unwrap();
    assert_eq!(
        e.to_string(),
        "Product old is archived. Run agentsi restore --product old to bring it back."
    );
    let mut o = options("p");
    o.roles = vec!["dev.api".into()];
    let e = run(&w, &o, &Cancel::default()).err().unwrap();
    assert_eq!(e.to_string(), "Specialist \"dev.api\" has no overlay in p");
    assert!(!std::path::Path::new(&format!("{}.log", w.program)).exists());
    let events = w.recorder.events.lock().unwrap();
    assert_eq!(events.last().unwrap().stage, "done");
    assert!(events.last().unwrap().error.is_some());
}

#[test]
fn a_frozen_snapshot_that_does_not_exist_is_not_found() {
    let w = world(REPLIES);
    let mut o = options("p");
    o.frozen = Some("nope".into());
    let e = run(&w, &o, &Cancel::default()).err().unwrap();
    assert!(matches!(e, Error::Agency(crate::Error::NotFound(_))), "{e}");
}

#[test]
fn a_canceled_meeting_writes_nothing() {
    let w = world("#!/bin/sh\ncat >/dev/null\nsleep 30\n");
    let cancel = Cancel::default();
    let other = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        other.cancel();
    });
    let e = run(&w, &options("p"), &cancel).err().unwrap();
    assert_eq!(e.to_string(), CANCELED);
    assert_eq!(count(&w, "minutes"), 0);
    assert_eq!(count(&w, "meeting_log"), 0);
}

#[test]
fn a_meeting_of_a_failed_synthesis_has_no_repair_and_stays_invalid() {
    let w = world(
        r#"#!/bin/sh
prompt=$(cat)
case "$prompt" in
*"You are the"*) printf '{"type":"result","structured_output":{"answer":"A","priorities":[],"concerns":[],"memory_notes":[]}}\n' ;;
*) exit 2 ;;
esac
"#,
    );
    let out = run(&w, &options("p"), &Cancel::default()).unwrap();
    assert_eq!(out.minutes["status"], "invalid");
    assert_eq!(out.minutes["check"]["repaired"], false);
    let failures: Vec<&Value> = out.minutes["check"]["failures"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert_eq!(failures[0], "No priorities.");
    // The roles run at the same time, so they end in any order. A failed synthesis has no row.
    let mut ended: Vec<&str> = out.usage.iter().map(|u| u.role.as_str()).collect();
    ended.sort();
    assert_eq!(ended, ["dev", "qa"]);
}

/// Cancels the meeting when its check starts, as the owner can during a slow `gh` call.
struct CancelAtCheck {
    cancel: Cancel,
}

impl Sink for CancelAtCheck {
    fn progress(&self, event: MeetingProgressParams) {
        if event.stage == "check" && event.phase == "start" {
            self.cancel.cancel();
        }
    }

    fn line(&self, _: &str, _: &str) {}
}

#[test]
fn a_cancel_during_the_check_writes_nothing() {
    let w = world(
        r#"#!/bin/sh
prompt=$(cat)
case "$prompt" in
*"You are the"*) printf '{"type":"result","structured_output":{"answer":"A","priorities":[],"concerns":[],"memory_notes":["Keep it."]}}\n' ;;
*) printf '{"type":"result","structured_output":{"priorities":[{"title":"Do it","why":"W","sources":[{"type":"path","ref":"docs/vision.md"}],"owner_role":"dev"}],"disagreements":[],"decisions":[],"script":[{"role":"pm","text":"Hello."}]}}\n' ;;
esac
"#,
    );
    let cancel = Cancel::default();
    let sink = CancelAtCheck {
        cancel: cancel.clone(),
    };
    let root = w.dir.path().join("root");
    let env = Env {
        store: &w.store,
        root: &root,
        registry: &w.registry,
        sink: &sink,
        cancel: &cancel,
        claude: &w.program,
    };
    let e = run_meeting(&env, &options("p"))
        .err()
        .expect("the meeting is canceled");
    assert!(matches!(e, Error::Conflict(_)), "{e}");
    assert_eq!((count(&w, "minutes"), count(&w, "memory")), (0, 0));
}

#[test]
fn a_frozen_meeting_reads_its_snapshot_and_runs_no_index() {
    let w = world(REPLIES);
    let out = run(&w, &options("p"), &Cancel::default()).unwrap();
    let id = out.minutes["id"].as_str().unwrap().to_string();
    let events = w.recorder.events.lock().unwrap();
    assert!(!events.iter().any(|e| e.stage == "index"));
    assert_eq!(events.last().unwrap().note.as_deref(), Some(id.as_str()));
    let body = w
        .store
        .read(|c| {
            Ok(c.query_row(
                "select body from snapshots where product = 'p' and minutes_id = ?1",
                [&id],
                |r| r.get::<_, String>(0),
            )?)
        })
        .unwrap();
    assert_eq!(body, "# Snapshot\nfrozen");
    assert_eq!(count(&w, "documents"), 0);
}
