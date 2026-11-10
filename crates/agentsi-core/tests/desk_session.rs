//! The desk session on a real core with a fake `claude` on its PATH: two owner messages to one
//! process, a resume after a restart, a desk note, `busy` and the reply limit, a crash,
//! `signed_out`, `desk.show` and `desk.reset`. No test runs the real `claude`.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir, wait_until};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A fake `claude`. `auth status` prints `$dir/auth`. Else it counts its starts, keeps its
/// arguments and answers each stdin line. A line with `hang` never ends its reply, and a line
/// with `crash` stops the program.
const FAKE: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
if [ "$1" = auth ]; then cat "$dir/auth"; exit 0; fi
n=$(cat "$dir/starts" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$dir/starts"
printf '%s\n' "$@" > "$dir/args-$n"
echo "$$" >> "$dir/pids"
echo '{"type":"system","subtype":"init"}'
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$dir/stdin"
  case "$line" in
    *hang*) sleep 60;;
    *crash*) echo 'it broke' >&2; exit 3;;
  esac
  echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}}'
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Hi"},{"type":"tool_use","name":"mcp__agentsi__show_decisions","input":{"ids":[1]}}]}}'
  echo '{"type":"result","is_error":false,"total_cost_usd":0.01,"duration_ms":5}'
done
"#;

struct World {
    home: PathBuf,
    bin: PathBuf,
}

fn setup(dir: &TempDir, auth: &str) -> World {
    let home = dir.0.join("root");
    let repo = dir.0.join("repo");
    let bin = dir.0.join("bin");
    for d in [&home, &repo, &bin] {
        std::fs::create_dir_all(d).unwrap();
    }
    let registry = format!(
        r#"{{"owner":{{"name":"Ada","language":"English"}},"products":[
  {{"id":"alpha","name":"Alpha","paths":[{:?}],"status":"active","roles":["pm"]}}]}}"#,
        repo
    );
    std::fs::write(home.join("agentsi.config.json"), registry).unwrap();
    let conn = agentsi_store::db::open_writer(&home.join("agentsi.db")).unwrap();
    conn.execute_batch(
        "insert into products (id, name, paths, status) values ('alpha', 'Alpha', '[]', 'active');
         insert into decisions (id, product, minutes_id, text, why, status, kind, action)
           values (1, 'alpha', null, 'Ship it', 'It is ready', 'pending', 'product', 'Run the release'),
                  (2, 'alpha', null, 'Old one', 'w', 'pending', 'product', null);
         insert into archive_records values ('alpha', 'decision', '2');
         insert into products (id, name, paths, status) values ('beta', 'Beta', '[]', 'active');
         insert into minutes (id, product, question, status, created_at, data) values
           ('m1', 'alpha', 'What now?', 'valid', '2026-11-02T10:00:00.000Z',
            '{\"id\":\"m1\",\"question\":\"What now?\",\"priorities\":[{\"title\":\"P0\",\"why\":\"w\",\"sources\":[]}]}'),
           ('mb', 'beta', 'Other?', 'valid', '2026-11-02T10:00:00.000Z', '{\"id\":\"mb\"}');
         insert into priorities (minutes_id, idx, title, why) values ('m1', 0, 'P0', 'w');
         insert into stories (product, id, data) values ('alpha', 'S001', '{\"id\":\"S001\"}');",
    )
    .unwrap();
    let fake = bin.join("claude");
    std::fs::write(&fake, FAKE).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(bin.join("auth"), auth).unwrap();
    World { home, bin }
}

const READY: &str = r#"{"loggedIn": true, "authMethod": "claude.ai"}"#;

fn start(dir: &TempDir, w: &World, more: &[(&str, &str)]) -> CoreProc {
    let path = format!("{}:/usr/bin:/bin", w.bin.display());
    let mut env: Vec<(&str, &Path)> = vec![
        ("AGENTSI_HOME", w.home.as_path()),
        ("PATH", Path::new(&path)),
        ("AGENTSI_CLI_BIN", Path::new("/bin/true")),
    ];
    env.extend(more.iter().map(|(k, v)| (*k, Path::new(*v))));
    // `start_with` borrows the paths only while it spawns.
    CoreProc::start_with(dir, &env)
}

fn connect(core: &CoreProc) -> Framed {
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["desk", "data"]}));
    c
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

/// Reads `desk.item` events until one of `kind`, and gives the kinds that came.
fn items_until(c: &mut Framed, kind: &str) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        let e = c.next_event("desk.item");
        let k = e["item"]["kind"].as_str().unwrap().to_string();
        seen.push(k.clone());
        if k == kind {
            return seen;
        }
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn history(c: &mut Framed) -> Value {
    c.ok("desk.history", json!({"product": "alpha"}))
}

fn kinds(history: &Value) -> Vec<String> {
    history["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item"]["kind"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_second_owner_message_goes_to_the_same_claude_process() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    assert_eq!(history(&mut c)["state"], "stopped");

    let r = c.ok(
        "desk.send",
        json!({"product": "alpha", "text": "What waits for me?"}),
    );
    assert_eq!(r, json!({"seq": 1}));
    assert_eq!(items_until(&mut c, "end"), ["owner", "reply", "end"]);
    let r = c.ok(
        "desk.send",
        json!({"product": "alpha", "text": "And then?"}),
    );
    assert_eq!(r, json!({"seq": 4}));
    assert_eq!(items_until(&mut c, "end"), ["owner", "reply", "end"]);

    assert_eq!(read(&w.bin.join("starts")).trim(), "1");
    assert_eq!(read(&w.bin.join("pids")).lines().count(), 1);
    let stdin = read(&w.bin.join("stdin"));
    assert_eq!(stdin.lines().count(), 2);
    assert!(stdin.contains("\"content\":\"And then?\""), "{stdin}");

    let h = history(&mut c);
    assert_eq!(h["state"], "idle");
    assert_eq!(
        kinds(&h),
        ["owner", "reply", "end", "owner", "reply", "end"]
    );
    assert_eq!(
        h["items"][1]["item"],
        json!({"kind": "reply", "text": "Hi"})
    );
    assert_eq!(h["items"][2]["item"]["cost_usd"], 0.01);
    let session = h["session"].as_str().unwrap().to_string();
    let args = read(&w.bin.join("args-1"));
    assert!(
        args.contains(&format!("--session-id\n{session}\n")),
        "{args}"
    );
    for banned in ["--bare", "Bash", "Edit", "Write"] {
        assert!(!args.contains(banned), "{banned}");
    }
    assert!(args.contains(&format!("\"AGENTSI_DESK_SESSION\":\"{session}\"")));

    let page = c.ok(
        "desk.history",
        json!({"product": "alpha", "before": 4, "limit": 2}),
    );
    let seqs: Vec<i64> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, [2, 3]);
    assert_eq!(
        c.ok("desk.stop", json!({"product": "alpha"})),
        json!({"stopped": true})
    );
    assert_eq!(
        c.ok("desk.stop", json!({"product": "alpha"})),
        json!({"stopped": false})
    );
}

#[test]
fn after_a_core_restart_the_process_resumes_the_same_session() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let session = {
        let core = start(&dir, &w, &[]);
        let mut c = connect(&core);
        c.ok("desk.send", json!({"product": "alpha", "text": "one"}));
        items_until(&mut c, "end");
        history(&mut c)["session"].as_str().unwrap().to_string()
    };
    wait_until("the first process stops with its core", || {
        read(&w.bin.join("pids"))
            .trim()
            .parse::<i32>()
            .is_ok_and(|pid| !common::alive(pid))
    });
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    assert_eq!(history(&mut c)["session"], session.as_str());
    c.ok("desk.send", json!({"product": "alpha", "text": "two"}));
    items_until(&mut c, "end");
    let args = read(&w.bin.join("args-2"));
    assert!(args.contains(&format!("--resume\n{session}\n")), "{args}");
    assert!(!args.contains("--session-id"));
}

#[test]
fn an_owner_decision_reaches_the_next_owner_message_as_a_desk_note() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    // No desk session yet: a write makes no note.
    c.ok(
        "decision.add",
        json!({"product": "alpha", "text": "Pick a name", "why": "w"}),
    );
    c.ok("desk.send", json!({"product": "alpha", "text": "one"}));
    items_until(&mut c, "end");

    c.ok(
        "decision.decide",
        json!({"decision": 1, "status": "approved"}),
    );
    let note = c.next_event("desk.item");
    assert_eq!(
        note["item"],
        json!({"kind": "note", "text": "The owner approved decision 1."})
    );
    c.ok("desk.send", json!({"product": "alpha", "text": "two"}));
    items_until(&mut c, "end");
    let stdin = read(&w.bin.join("stdin"));
    let last: Value = serde_json::from_str(stdin.lines().last().unwrap()).unwrap();
    assert_eq!(
        last["message"]["content"],
        "Desk notes:\n- The owner approved decision 1.\n\ntwo"
    );
    let h = history(&mut c);
    assert_eq!(
        kinds(&h),
        ["owner", "reply", "end", "note", "owner", "reply", "end"]
    );
    assert_eq!(h["items"][4]["item"]["text"], "two");
}

#[test]
fn a_reply_that_runs_is_busy_and_the_reply_limit_stops_it() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[("AGENTSI_DESK_REPLY_SECS", "2")]);
    let mut c = connect(&core);
    c.ok("desk.send", json!({"product": "alpha", "text": "hang now"}));
    let r = c.call("desk.send", json!({"product": "alpha", "text": "again"}));
    assert_eq!(kind(&r), "busy", "{r}");
    assert_eq!(items_until(&mut c, "error"), ["owner", "error"]);
    wait_until("the state is stopped", || {
        history(&mut c)["state"] == "stopped"
    });
    let pid: i32 = read(&w.bin.join("pids")).trim().parse().unwrap();
    wait_until("the process is gone", || !common::alive(pid));
}

#[test]
fn a_crash_gives_a_failed_state_and_the_next_message_starts_a_new_process() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    c.ok(
        "desk.send",
        json!({"product": "alpha", "text": "crash now"}),
    );
    items_until(&mut c, "error");
    wait_until("the state is failed", || {
        history(&mut c)["state"] == "failed"
    });
    let h = history(&mut c);
    assert!(h["message"].as_str().unwrap().contains("it broke"), "{h}");
    assert_eq!(read(&w.bin.join("starts")).trim(), "1");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        read(&w.bin.join("starts")).trim(),
        "1",
        "no restart by itself"
    );
    c.ok("desk.send", json!({"product": "alpha", "text": "hello"}));
    items_until(&mut c, "end");
    assert_eq!(read(&w.bin.join("starts")).trim(), "2");
}

#[test]
fn a_signed_out_login_shows_its_message_and_starts_no_process() {
    let dir = TempDir::new();
    let w = setup(&dir, r#"{"loggedIn": false}"#);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    let r = c.ok("desk.auth", json!({}));
    assert_eq!(r["state"], "signed_out");
    assert!(r["message"].as_str().unwrap().contains("claude auth login"));
    let r = c.call("desk.send", json!({"product": "alpha", "text": "hi"}));
    assert_eq!(kind(&r), "unavailable", "{r}");
    let state = c.next_event("desk.state");
    assert_eq!(state["state"], "auth");
    assert!(!w.bin.join("starts").exists());
    assert_eq!(history(&mut c)["items"], json!([]));

    std::fs::write(w.bin.join("auth"), READY).unwrap();
    assert_eq!(c.ok("desk.auth", json!({}))["state"], "signed_out");
    assert_eq!(
        c.ok("desk.auth", json!({"recheck": true}))["state"],
        "ready"
    );
    c.ok("desk.send", json!({"product": "alpha", "text": "hi"}));
    items_until(&mut c, "end");
}

#[test]
fn a_view_names_decisions_of_the_product_and_decision_get_reads_them() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    let r = c.call(
        "desk.show",
        json!({"session": "nope", "view": {"kind": "decisions", "ids": [1]}}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");

    c.ok("desk.send", json!({"product": "alpha", "text": "one"}));
    items_until(&mut c, "end");
    let session = history(&mut c)["session"].as_str().unwrap().to_string();
    let r = c.call(
        "desk.show",
        json!({"session": session, "view": {"kind": "decisions", "ids": [1, 77]}}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
    assert!(r["error"]["message"].as_str().unwrap().contains("77"));
    let r = c.call(
        "desk.show",
        json!({"session": session, "view": {"kind": "decisions", "ids": ["x"]}}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call(
        "desk.show",
        json!({"session": session, "view": {"kind": "decisions", "ids": []}}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    assert_eq!(kinds(&history(&mut c)), ["owner", "reply", "end"]);

    let r = c.ok(
        "desk.show",
        json!({"session": session, "view": {"kind": "decisions", "ids": [1, 2]}}),
    );
    assert_eq!(r, json!({"view_id": 4}));
    let e = c.next_event("desk.item");
    assert_eq!(
        e["item"],
        json!({"kind": "view", "view": {"kind": "decisions", "ids": [1, 2]}})
    );

    let r = c.ok(
        "decision.get",
        json!({"product": "alpha", "ids": [2, 1, 99]}),
    );
    assert_eq!(
        r["decisions"],
        json!([
            {"id": 2, "kind": "product", "minutes_id": null, "text": "Old one", "why": "w",
             "action": null, "status": "pending", "archived": true},
            {"id": 1, "kind": "product", "minutes_id": null, "text": "Ship it",
             "why": "It is ready", "action": "Run the release", "status": "pending",
             "archived": false},
        ])
    );
    let r = c.call("decision.get", json!({"product": "nope", "ids": [1]}));
    assert_eq!(kind(&r), "not_found", "{r}");
}

#[test]
fn object_views_name_objects_of_the_product_and_accept_on_a_priority_writes_no_owner_message() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    c.ok("desk.send", json!({"product": "alpha", "text": "one"}));
    items_until(&mut c, "end");
    let session = history(&mut c)["session"].as_str().unwrap().to_string();
    let show = |c: &mut Framed, view: Value| {
        c.call("desk.show", json!({"session": session, "view": view}))
    };

    // An unknown id, or an object of another product, stores no view item.
    for (view, word) in [
        (json!({"kind": "story", "id": "S999"}), "S999"),
        (json!({"kind": "diff", "story_id": "S404"}), "S404"),
        (json!({"kind": "priorities", "minutes_id": "nope"}), "nope"),
        (json!({"kind": "priorities", "minutes_id": "mb"}), "mb"),
    ] {
        let r = show(&mut c, view);
        assert_eq!(kind(&r), "not_found", "{r}");
        assert!(r["error"]["message"].as_str().unwrap().contains(word), "{r}");
    }
    let r = show(&mut c, json!({"kind": "story"}));
    assert_eq!(kind(&r), "invalid", "{r}");
    assert_eq!(kinds(&history(&mut c)), ["owner", "reply", "end"]);

    for view in [
        json!({"kind": "story", "id": "S001"}),
        json!({"kind": "diff", "story_id": "S001"}),
        json!({"kind": "priorities", "minutes_id": "m1"}),
    ] {
        let r = show(&mut c, view.clone());
        assert!(r.get("error").is_none(), "{r}");
        assert_eq!(c.next_event("desk.item")["item"]["view"], view);
    }

    // The priorities view sets a mark with mark.set. The core stores it and a desk note, and
    // no owner message.
    c.ok(
        "mark.set",
        json!({"entries": [{"minutes_id": "m1", "idx": 0, "mark": "accepted"}], "unmarked_only": true}),
    );
    let r = c.ok("minutes.get", json!({"id": "m1"}));
    assert_eq!(r["priorities"][0]["mark"], "accepted");
    let h = history(&mut c);
    assert_eq!(
        kinds(&h),
        ["owner", "reply", "end", "view", "view", "view", "note"]
    );
    let n = &h["items"].as_array().unwrap()[6]["item"]["text"];
    assert!(n.as_str().unwrap().contains("accepted"), "{n}");
}

#[test]
fn generic_views_keep_their_limits_a_check_becomes_a_desk_note_and_a_page_gives_its_length() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    c.ok("desk.send", json!({"product": "alpha", "text": "one"}));
    items_until(&mut c, "end");
    let session = history(&mut c)["session"].as_str().unwrap().to_string();
    let show = |c: &mut Framed, view: Value| {
        c.call("desk.show", json!({"session": session, "view": view}))
    };

    // A bad input is `invalid` with a text for Claude, and no item is stored.
    for (view, word) in [
        (
            json!({"kind": "table", "title": "T", "columns": ["a", "b"],
                   "rows": [["1", "2"], ["1", "2", "3"]]}),
            "Row 2 has 3 cells",
        ),
        (
            json!({"kind": "table", "title": "T", "columns": [], "rows": []}),
            "columns has 0",
        ),
        (
            json!({"kind": "chart", "chart": "bar", "title": "C", "labels": ["a"], "values": [1, 2]}),
            "one label for each value",
        ),
        (
            json!({"kind": "chart", "chart": "pie", "title": "C", "labels": ["a"], "values": [-1]}),
            "negative",
        ),
        (
            json!({"kind": "checklist", "title": "L", "items": ["x".repeat(201)]}),
            "Item 1 has 201 characters",
        ),
        (
            json!({"kind": "page", "title": "P", "html": "x".repeat(512_001)}),
            "html has 512001",
        ),
    ] {
        let r = show(&mut c, view);
        assert_eq!(kind(&r), "invalid", "{r}");
        let m = r["error"]["message"].as_str().unwrap();
        assert!(m.contains(word), "{m}");
    }
    let r = show(&mut c, json!({"kind": "chart", "chart": "donut", "title": "C", "labels": [], "values": []}));
    assert_eq!(kind(&r), "invalid", "{r}");
    assert_eq!(kinds(&history(&mut c)), ["owner", "reply", "end"]);

    let table = json!({"kind": "table", "title": "T", "columns": ["a"], "rows": [["1"]]});
    assert_eq!(show(&mut c, table.clone())["result"]["view_id"], 4);
    assert_eq!(c.next_event("desk.item")["item"]["view"], table);

    // A checklist: the check is stored in the item and becomes a desk note.
    let list = json!({"kind": "checklist", "title": "Launch", "items": ["tag", "post", "mail"]});
    assert_eq!(show(&mut c, list)["result"]["view_id"], 5);
    c.next_event("desk.item");
    let r = c.ok(
        "desk.check",
        json!({"product": "alpha", "seq": 5, "checked": [1, 0, 1]}),
    );
    assert_eq!(r, json!({"seq": 5}));
    let note = c.next_event("desk.item");
    assert_eq!(
        note["item"],
        json!({"kind": "note", "text": "The owner checked 2 of 3 items of Launch: tag, post."})
    );
    for (params, want) in [
        (json!({"product": "alpha", "seq": 5, "checked": [3]}), "invalid"),
        (json!({"product": "alpha", "seq": 4, "checked": [0]}), "invalid"),
        (json!({"product": "alpha", "seq": 99, "checked": []}), "not_found"),
        (json!({"product": "nope", "seq": 5, "checked": []}), "not_found"),
    ] {
        let r = c.call("desk.check", params);
        assert_eq!(kind(&r), want, "{r}");
    }

    // A page: the events and the history give its length. desk.page gives the HTML.
    let html = "<h1>Report</h1>";
    let r = show(&mut c, json!({"kind": "page", "title": "Report", "html": html}));
    assert_eq!(r["result"]["view_id"], 7);
    let e = c.next_event("desk.item");
    assert_eq!(
        e["item"],
        json!({"kind": "view", "view": {"kind": "page", "title": "Report", "html": ""},
               "html_len": 15})
    );
    let h = history(&mut c);
    let items = h["items"].as_array().unwrap();
    assert_eq!(items[4]["item"]["checked"], json!([0, 1]));
    assert_eq!(items[6]["item"], e["item"]);
    let r = c.ok("desk.page", json!({"product": "alpha", "seq": 7}));
    assert_eq!(r, json!({"title": "Report", "html": html}));
    let r = c.call("desk.page", json!({"product": "alpha", "seq": 5}));
    assert_eq!(kind(&r), "invalid", "{r}");

    // The next owner message carries the note of the check.
    c.ok("desk.send", json!({"product": "alpha", "text": "two"}));
    items_until(&mut c, "end");
    let stdin = read(&w.bin.join("stdin"));
    let last: Value = serde_json::from_str(stdin.lines().last().unwrap()).unwrap();
    assert_eq!(
        last["message"]["content"],
        "Desk notes:\n- The owner checked 2 of 3 items of Launch: tag, post.\n\ntwo"
    );
}

#[test]
fn a_tickets_view_names_tickets_of_the_product() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    let t = c.ok(
        "ticket.add",
        json!({"product": "alpha", "kind": "todo", "title": "Tag the release",
               "author": {"by": "owner", "id": ""}}),
    );
    let id = t["ticket"]["id"].as_str().unwrap().to_string();
    c.ok("desk.send", json!({"product": "alpha", "text": "one"}));
    items_until(&mut c, "end");
    let session = history(&mut c)["session"].as_str().unwrap().to_string();
    let show = |c: &mut Framed, ids: Value| {
        c.call(
            "desk.show",
            json!({"session": session, "view": {"kind": "tickets", "ids": ids}}),
        )
    };
    let r = show(&mut c, json!([id, "C999"]));
    assert_eq!(kind(&r), "not_found", "{r}");
    assert!(r["error"]["message"].as_str().unwrap().contains("C999"), "{r}");
    let r = show(&mut c, json!([]));
    assert_eq!(kind(&r), "invalid", "{r}");
    assert_eq!(kinds(&history(&mut c)), ["owner", "reply", "end"]);
    let r = show(&mut c, json!([id]));
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(
        c.next_event("desk.item")["item"]["view"],
        json!({"kind": "tickets", "ids": [id]})
    );
}

#[test]
fn a_reset_starts_a_new_conversation_with_a_new_session() {
    let dir = TempDir::new();
    let w = setup(&dir, READY);
    let core = start(&dir, &w, &[]);
    let mut c = connect(&core);
    c.ok("desk.send", json!({"product": "alpha", "text": "one"}));
    items_until(&mut c, "end");
    let old = history(&mut c)["session"].as_str().unwrap().to_string();
    let r = c.ok("desk.reset", json!({"product": "alpha"}));
    let new = r["session"].as_str().unwrap().to_string();
    assert_ne!(old, new);
    let h = history(&mut c);
    assert_eq!(
        (h["session"].as_str(), h["items"].clone()),
        (Some(new.as_str()), json!([]))
    );
    assert_eq!(h["state"], "stopped");
    c.ok("desk.send", json!({"product": "alpha", "text": "two"}));
    items_until(&mut c, "end");
    let args = read(&w.bin.join("args-2"));
    assert!(args.contains(&format!("--session-id\n{new}\n")), "{args}");
    let r = c.call("desk.send", json!({"product": "nope", "text": "x"}));
    assert_eq!(kind(&r), "not_found", "{r}");
}
