//! `product.whoami`, `product.status`, `data.snapshot` and `data.audit` on a
//! real core. The output against the TypeScript CLI is in the golden test of
//! `agentsi-cli`; these tests cover the errors and the shape of the results.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::path::Path;

const REGISTRY: &str = r#"{"owner":{"name":"Ada","language":"English"},"products":[
  {"id":"alpha","name":"Alpha","paths":["/work/alpha"],"status":"active","roles":["pm"]},
  {"id":"beta","name":"Beta","paths":["/work/beta"],"status":"planned","roles":["pm","dev"]}]}"#;

fn root(dir: &TempDir, with_db: bool) -> std::path::PathBuf {
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    if with_db {
        let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
        conn.execute_batch(
            "insert into products (id, name, paths, status) values ('alpha', 'Alpha', '[\"/work/alpha\"]', 'active');
             insert into decisions (id, product, text, status, kind) values (1, 'alpha', 'Pick a name', 'pending', 'product');
             insert into archive_records (product, kind, ref) values ('beta', 'product', '');",
        )
        .unwrap();
    }
    root
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

fn start(dir: &TempDir, home: &Path) -> (CoreProc, Framed) {
    let core = CoreProc::start_with(dir, &[("AGENTSI_HOME", home)]);
    let client = Framed::connect(&core.socket.path);
    (core, client)
}

#[test]
fn whoami_finds_a_product_by_folder_or_id_and_lists_the_registry() {
    let dir = TempDir::new();
    let (_core, mut c) = start(&dir, &root(&dir, true));

    let r = c.ok("product.whoami", json!({"cwd": "/work/alpha/src/../lib"}));
    assert_eq!(r["product"]["product"], "alpha");
    assert_eq!(
        r["product"]["owner"],
        json!({"name": "Ada", "language": "English"})
    );
    assert_eq!(r["product"]["scope"], Value::Null);

    let r = c.ok("product.whoami", json!({"cwd": "/elsewhere"}));
    assert_eq!(r["product"], Value::Null);

    let r = c.ok(
        "product.whoami",
        json!({"cwd": "/elsewhere", "product": "beta"}),
    );
    assert_eq!(r["product"]["name"], "Beta");

    let r = c.call("product.whoami", json!({"product": "nope"}));
    assert_eq!(kind(&r), "not_found", "{r}");
    assert_eq!(r["error"]["message"], "Unknown product \"nope\"");

    let r = c.ok("product.whoami", json!({"list": true}));
    let archived: Vec<_> = r["products"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["archived"].as_bool().unwrap()))
        .collect();
    assert_eq!(archived, [("alpha", false), ("beta", true)]);

    let r = c.ok("product.whoami", json!({"home": true}));
    assert_eq!(r["home"]["from"], "AGENTSI_HOME");
}

#[test]
fn status_snapshot_and_audit_read_the_store() {
    let dir = TempDir::new();
    let (_core, mut c) = start(&dir, &root(&dir, true));

    let r = c.ok("product.status", json!({"product": "alpha"}));
    let text = r["markdown"].as_str().unwrap();
    assert!(text.starts_with("# Alpha — AgentSI status\n"), "{text}");
    assert!(text.contains("- [1] Pick a name\n"), "{text}");
    assert_eq!(r["index"]["minutes"], 0);

    let r = c.ok("data.snapshot", json!({"product": "alpha"}));
    let text = r["markdown"].as_str().unwrap();
    assert!(text.starts_with("# Snapshot: Alpha\n"), "{text}");
    assert!(text.ends_with('\n'));

    let r = c.ok("data.audit", json!({"product": "alpha"}));
    assert_eq!(r["product"], "alpha");
    assert_eq!(r["meetings"]["count"], 0);
    assert_eq!(r["last_repairs"], Value::Null);
    // The role `pm` has no file in the data root, so the program uses the role file that it holds.
    assert_eq!(r["config"]["valid"], true);
}

#[test]
fn an_unknown_product_is_not_found_and_a_missing_database_is_unavailable() {
    let dir = TempDir::new();
    let (_core, mut c) = start(&dir, &root(&dir, true));
    for method in ["product.status", "data.snapshot", "data.audit"] {
        let r = c.call(method, json!({"product": "nope"}));
        assert_eq!(kind(&r), "not_found", "{method}: {r}");
    }

    let dir = TempDir::new();
    let (_core, mut c) = start(&dir, &root(&dir, false));
    for method in ["product.status", "data.snapshot", "data.audit"] {
        let r = c.call(method, json!({"product": "alpha"}));
        assert_eq!(kind(&r), "unavailable", "{method}: {r}");
    }
    // The registry alone answers `whoami`.
    assert_eq!(
        c.ok("product.whoami", json!({"cwd": "/work/beta"}))["product"]["product"],
        "beta"
    );
}

#[test]
fn whoami_without_a_registry_is_unavailable() {
    let dir = TempDir::new();
    let home = dir.0.join("empty");
    std::fs::create_dir_all(&home).unwrap();
    let (_core, mut c) = start(&dir, &home);
    let r = c.call("product.whoami", json!({"cwd": "/work/alpha"}));
    assert_eq!(kind(&r), "unavailable", "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("agentsi.config.json")
    );
}
