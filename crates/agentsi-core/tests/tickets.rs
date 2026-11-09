//! `ticket.*` on a real core: each method, each move, an illegal move, the session rule,
//! `data.changed` and a failed write.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};

const REGISTRY: &str = r#"{"owner":{"name":"Ada","language":"English"},"products":[
  {"id":"alpha","name":"Alpha","paths":["/work/alpha"],"status":"active","roles":["pm","dev"]},
  {"id":"beta","name":"Beta","paths":["/work/beta"],"status":"active","roles":["pm"]}]}"#;

fn setup(dir: &TempDir) -> std::path::PathBuf {
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    conn.execute_batch(
        "insert into products (id, name, paths, status) values ('alpha', 'Alpha', '[]', 'active');
         insert into stories values ('alpha', 'S001', '{}');",
    )
    .unwrap();
    root
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

fn owner() -> Value {
    json!({"by": "owner"})
}

fn session(id: &str) -> Value {
    json!({"by": "session", "id": id})
}

fn ids(c: &mut Framed, params: Value) -> Vec<String> {
    c.ok("ticket.list", params)["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn each_ticket_method_on_a_real_core() {
    let dir = TempDir::new();
    let home = setup(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));
    let tickets_event = json!({"product": "alpha", "kind": "tickets"});

    // ticket.add: the owner gets the name of the owner block as its id.
    let r = c.ok(
        "ticket.add",
        json!({"product": "alpha", "kind": "todo", "title": " Ship it ", "author": owner(),
               "links": [{"type": "story", "ref": "S001"}]}),
    );
    let t = &r["ticket"];
    assert_eq!(
        (t["id"].as_str(), t["state"].as_str()),
        (Some("C001"), Some("open"))
    );
    assert_eq!(t["title"], "Ship it");
    assert_eq!(t["author"], json!({"by": "owner", "id": "Ada"}));
    assert_eq!(c.next_event("data.changed"), tickets_event);
    // A client cannot write as another owner.
    let r = c.ok(
        "ticket.add",
        json!({"product": "alpha", "kind": "note", "title": "From s1",
               "author": session("s1")}),
    );
    assert_eq!(r["ticket"]["id"], "C002");
    assert_eq!(r["ticket"]["author"], session("s1"));
    assert_eq!(c.next_event("data.changed"), tickets_event);

    // ticket.get
    let r = c.ok("ticket.get", json!({"product": "alpha", "id": "C001"}));
    assert_eq!(r["ticket"]["id"], "C001");
    let md = r["markdown"].as_str().unwrap();
    assert!(
        md.starts_with(
            "# C001 Ship it\n\n- Kind: todo · State: open · Assignee: none\n- Author: owner Ada"
        ),
        "{md}"
    );
    assert!(md.contains("- Links: story S001\n"), "{md}");

    // ticket.list: the newest write first, the highest id on a tie.
    let all = ids(&mut c, json!({"product": "alpha"}));
    assert_eq!(all.len(), 2);
    assert_eq!(
        ids(&mut c, json!({"product": "alpha", "author": "session"})),
        ["C002"]
    );
    assert!(ids(&mut c, json!({"product": "beta"})).is_empty());

    // The session rule: s1 cannot touch a ticket of the owner.
    let r = c.call(
        "ticket.move",
        json!({"product": "alpha", "id": "C001", "state": "dropped", "by": session("s1")}),
    );
    assert_eq!(kind(&r), "conflict", "{r}");
    assert_eq!(r["error"]["message"], "Ticket C001 belongs to the owner.");
    let r = c.call(
        "ticket.update",
        json!({"product": "alpha", "id": "C001", "title": "Mine", "by": session("s1")}),
    );
    assert_eq!(kind(&r), "conflict", "{r}");

    // ticket.update: s1 changes its own ticket.
    let r = c.ok(
        "ticket.update",
        json!({"product": "alpha", "id": "C002", "kind": "question", "assignee": "dev",
               "by": session("s1")}),
    );
    assert_eq!(
        (
            r["ticket"]["kind"].as_str(),
            r["ticket"]["assignee"].as_str()
        ),
        (Some("question"), Some("dev"))
    );
    assert_eq!(c.next_event("data.changed"), tickets_event);

    // ticket.move: each move of the table.
    let mv = |c: &mut Framed, id: &str, state: &str, waiting_on: Option<&str>| {
        let mut p = json!({"product": "alpha", "id": id, "state": state, "by": owner()});
        if let Some(w) = waiting_on {
            p["waiting_on"] = json!(w);
        }
        c.call("ticket.move", p)
    };
    assert_eq!(kind(&mv(&mut c, "C001", "waiting", None)), "invalid");
    let r = mv(&mut c, "C001", "waiting", Some("Ada"));
    assert_eq!(r["result"]["ticket"]["waiting_on"], "Ada", "{r}");
    assert_eq!(c.next_event("data.changed"), tickets_event);
    let r = mv(&mut c, "C001", "open", None);
    assert_eq!(r["result"]["ticket"]["waiting_on"], Value::Null, "{r}");
    assert_eq!(c.next_event("data.changed"), tickets_event);
    let r = mv(&mut c, "C001", "done", None);
    assert!(r["result"]["ticket"]["closed_at"].is_string(), "{r}");
    assert_eq!(c.next_event("data.changed"), tickets_event);
    // An illegal move.
    let r = mv(&mut c, "C001", "dropped", None);
    assert_eq!(kind(&r), "invalid", "{r}");
    assert_eq!(
        r["error"]["message"],
        "Ticket C001 cannot move from done to dropped."
    );
    let r = mv(&mut c, "C001", "open", None);
    assert_eq!(r["result"]["ticket"]["closed_at"], Value::Null, "{r}");
    assert_eq!(c.next_event("data.changed"), tickets_event);
    // A move to the same state writes nothing and sends no event: the next event is of beta.
    let r = mv(&mut c, "C001", "open", None);
    assert_eq!(r["result"]["ticket"]["state"], "open", "{r}");
    c.ok(
        "ticket.add",
        json!({"product": "beta", "kind": "note", "title": "b", "author": owner()}),
    );
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "beta", "kind": "tickets"})
    );

    // Errors: unknown product, unknown ticket, unknown link target, bad title.
    let r = c.call("ticket.list", json!({"product": "nope"}));
    assert_eq!(kind(&r), "not_found", "{r}");
    let r = c.call("ticket.get", json!({"product": "alpha", "id": "C404"}));
    assert_eq!(kind(&r), "not_found", "{r}");
    let r = c.call("ticket.list", json!({"product": "alpha", "limit": 0}));
    assert_eq!(kind(&r), "invalid", "{r}");

    // A failed write leaves no row and sends no event.
    let r = c.call(
        "ticket.add",
        json!({"product": "alpha", "kind": "todo", "title": "t", "author": owner(),
               "links": [{"type": "story", "ref": "S999"}]}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
    let r = c.call(
        "ticket.add",
        json!({"product": "alpha", "kind": "todo", "title": "  ", "author": owner()}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call(
        "ticket.add",
        json!({"product": "alpha", "kind": "task", "title": "t", "author": owner()}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    assert_eq!(
        ids(
            &mut c,
            json!({"product": "alpha", "states": ["open", "waiting", "done", "dropped"]})
        )
        .len(),
        2
    );
    c.ok(
        "ticket.add",
        json!({"product": "beta", "kind": "note", "title": "b2", "author": owner()}),
    );
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "beta", "kind": "tickets"})
    );
}

#[test]
fn a_promote_makes_a_decision_and_a_story_promote_needs_stories_mode() {
    let dir = TempDir::new();
    let home = setup(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    for title in ["Pick a name", "Ship it"] {
        c.ok(
            "ticket.add",
            json!({"product": "alpha", "kind": "question", "title": title, "author": owner()}),
        );
    }
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let r = c.ok(
        "ticket.promote",
        json!({"product": "alpha", "id": "C001", "to": "decision", "by": owner()}),
    );
    let decision = r["decision"].as_i64().unwrap();
    assert_eq!(r["ticket"]["state"], "done");
    assert_eq!(
        r["ticket"]["links"],
        json!([{"type": "decision", "ref": decision.to_string()}])
    );
    assert!(r.get("story").is_none(), "{r}");
    assert_eq!(c.next_event("data.changed")["kind"], "decision");
    assert_eq!(c.next_event("data.changed")["kind"], "tickets");
    let pending = c.ok("mark.pending", json!({"product": "alpha"}));
    let item = &pending["items"][0];
    assert_eq!(
        (item["id"].as_i64(), item["text"].as_str()),
        (Some(decision), Some("Pick a name"))
    );
    assert_eq!(item["why"], "From ticket C001.");

    // A done ticket cannot be promoted again.
    let r = c.call(
        "ticket.promote",
        json!({"product": "alpha", "id": "C001", "to": "decision", "by": owner()}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    // A session cannot promote a ticket of the owner.
    let r = c.call(
        "ticket.promote",
        json!({"product": "alpha", "id": "C002", "to": "decision", "by": session("s1")}),
    );
    assert_eq!(kind(&r), "conflict", "{r}");

    // Stories mode is off for alpha: no PM runs and the ticket stays open.
    let r = c.call(
        "ticket.promote",
        json!({"product": "alpha", "id": "C002", "to": "story", "by": owner()}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Stories mode is off for alpha"),
        "{r}"
    );
    let r = c.ok("ticket.get", json!({"product": "alpha", "id": "C002"}));
    assert_eq!(r["ticket"]["state"], "open");
    assert_eq!(r["ticket"]["links"], json!([]));
}
