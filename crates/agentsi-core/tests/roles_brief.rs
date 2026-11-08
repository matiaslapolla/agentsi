//! `roles.brief`: the brief of a product as the Roles screen shows it. It only reads.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};

const REGISTRY: &str = r#"{"owner":{"name":"Ada","language":"English"},"products":[
  {"id":"alpha","name":"Alpha","paths":["/work/alpha"],"status":"active","roles":["pm","dev"]},
  {"id":"beta","name":"Beta","paths":["/work/beta"],"status":"active","roles":["pm"]}]}"#;

const BRIEF: &str = r#"{"version":1,
  "product":{"what":"A mail filter","audience":"Small teams","stage":"building"},
  "objectives":[{"id":"o1","text":"Ship the filter","signal":"10 users","due":"2026-12-01"}],
  "north_stars":["Calm inbox"],
  "sources_of_truth":[{"kind":"docs","ref":"docs/","note":"the docs"}],
  "out_of_scope":["Calendar"],
  "roles":{"pm":"Ranks the work.","dev":"Builds it."},
  "updated_at":"2026-10-30T10:00:00.000Z"}"#;

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

#[test]
fn roles_brief_gives_the_brief_of_a_product_and_null_for_a_product_without_one() {
    let dir = TempDir::new();
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    conn.execute(
        "insert into briefs (product, data) values ('alpha', ?1)",
        [BRIEF.replace('\n', "")],
    )
    .unwrap();
    drop(conn);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", root.as_path())]);
    let mut c = Framed::connect(&core.socket.path);

    let got = c.ok("roles.brief", json!({"product": "alpha"}));
    assert_eq!(got["product"], "alpha");
    assert_eq!(got["brief"]["what"], "A mail filter");
    assert_eq!(got["brief"]["audience"], "Small teams");
    assert_eq!(got["brief"]["stage"], "building");
    assert_eq!(got["brief"]["objectives"][0]["id"], "o1");
    assert_eq!(got["brief"]["objectives"][0]["due"], "2026-12-01");
    assert_eq!(got["brief"]["north_stars"], json!(["Calm inbox"]));
    assert_eq!(got["brief"]["out_of_scope"], json!(["Calendar"]));
    assert_eq!(got["brief"]["roles"]["dev"], "Builds it.");
    assert_eq!(got["brief"]["updated_at"], "2026-10-30T10:00:00.000Z");

    let none = c.ok("roles.brief", json!({"product": "beta"}));
    assert_eq!(none, json!({"product": "beta", "brief": null}));

    let r = c.call("roles.brief", json!({"product": "nope"}));
    assert_eq!(kind(&r), "not_found", "{r}");
}

/// B4 of the U27 review: a brief that `roles.validate` accepts reads, with its lacking fields
/// empty, as `product.desk` reads it.
#[test]
fn roles_brief_reads_a_lacking_field_as_empty() {
    let dir = TempDir::new();
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    let sparse = r#"{"version":1,"product":{"what":"A mail filter"},
      "objectives":[{"id":"o1","text":"Ship the filter"},{"text":"No id"}],
      "north_stars":["Calm inbox", 3],"roles":{"pm":"Ranks the work.","dev":7}}"#;
    conn.execute(
        "insert into briefs (product, data) values ('alpha', ?1)",
        [sparse.replace('\n', "")],
    )
    .unwrap();
    conn.execute(
        "insert into briefs (product, data) values ('beta', '{}')",
        [],
    )
    .unwrap();
    drop(conn);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", root.as_path())]);
    let mut c = Framed::connect(&core.socket.path);

    let got = c.call("roles.brief", json!({"product": "alpha"}));
    assert!(got.get("error").is_none(), "{got}");
    let brief = &got["result"]["brief"];
    assert_eq!(brief["what"], "A mail filter");
    assert_eq!(brief["audience"], "");
    assert_eq!(brief["stage"], "");
    assert_eq!(brief["objectives"][0]["id"], "o1");
    assert_eq!(brief["objectives"][0]["signal"], "");
    assert_eq!(brief["objectives"][0]["due"], "");
    assert_eq!(brief["objectives"][1]["id"], "");
    assert_eq!(brief["north_stars"], json!(["Calm inbox"]));
    assert_eq!(brief["out_of_scope"], json!([]));
    assert_eq!(brief["roles"], json!({"pm": "Ranks the work."}));
    assert_eq!(brief["updated_at"], Value::Null);

    let empty = c.call("roles.brief", json!({"product": "beta"}));
    assert!(empty.get("error").is_none(), "{empty}");
    assert_eq!(empty["result"]["brief"]["what"], "");
}
