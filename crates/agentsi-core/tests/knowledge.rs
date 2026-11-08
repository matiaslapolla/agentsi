//! The `knowledge` methods on a real core: the errors and the shape of the
//! results. The text of each MCP tool against the TypeScript server is in the
//! golden test of `agentsi-cli`.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

/// A data root with one product, one git repo with a doc, a pending decision and a memory item.
fn setup(dir: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = dir.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("README.md"),
        "# Tide tables\n\nThe harbour opens at dawn.\n",
    )
    .unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "-m",
            "feat: tides",
        ],
    ] {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let registry = json!({"products": [
        {"id": "alpha", "name": "Alpha", "paths": [repo], "status": "active", "roles": ["pm"]}]});
    std::fs::write(root.join("agentsi.config.json"), registry.to_string()).unwrap();
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    conn.execute_batch(
        "insert into decisions (id, product, text, status, kind) values (1, 'alpha', 'Pick a name', 'pending', 'product');
         insert into memory (product, agent, id, position, text, kind, helpful, harmful, source, created_at)
           values ('alpha', 'pm', 'm-1', 0, 'Prefers short answers.', 'fact', 0, 0, 's', '2026-10-01T00:00:00.000Z');",
    )
    .unwrap();
    (root, repo)
}

#[test]
fn the_index_feeds_the_search_and_the_other_reads() {
    let dir = TempDir::new();
    let (root, _repo) = setup(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &root)]);
    let mut c = Framed::connect(&core.socket.path);

    let r = c.ok("knowledge.index", json!({"product": "alpha"}));
    assert_eq!(r["repos"][0]["skipped"], false);
    assert_eq!(
        (
            r["repos"][0]["docs"].as_u64(),
            r["repos"][0]["commits"].as_u64()
        ),
        (Some(1), Some(1))
    );
    assert_eq!(r["memory"], 1);
    let r = c.ok("knowledge.index", json!({"product": "alpha"}));
    assert_eq!(r["repos"][0]["skipped"], true, "the second run is fresh");

    let r = c.ok(
        "knowledge.search",
        json!({"product": "alpha", "query": "harbour dawn"}),
    );
    assert_eq!(r["any_term"], false);
    assert_eq!(r["hits"][0]["ref"], "README.md");
    assert_eq!(r["hits"][0]["title"], "Tide tables");

    let r = c.ok(
        "knowledge.search",
        json!({"product": "alpha", "query": "harbour zzqq"}),
    );
    assert_eq!(r["any_term"], true, "no document has both words");
    assert_eq!(r["hits"].as_array().unwrap().len(), 1);

    let r = c.ok(
        "knowledge.search",
        json!({"product": "alpha", "query": "short", "kind": "memory"}),
    );
    assert_eq!(r["hits"][0]["ref"], "pm");
    let r = c.ok(
        "knowledge.search",
        json!({"product": "alpha", "query": "short"}),
    );
    assert!(
        r["hits"].as_array().unwrap().is_empty(),
        "memory is searched only when named"
    );

    let r = c.ok(
        "knowledge.get_doc",
        json!({"product": "alpha", "kind": "doc", "ref": "README.md"}),
    );
    assert!(r["body"].as_str().unwrap().contains("opens at dawn"));
    let r = c.ok(
        "knowledge.pending_decisions",
        json!({"product": "alpha", "limit": 10}),
    );
    assert_eq!(r["decisions"][0]["text"], "Pick a name");
    assert_eq!(r["hidden"], 0);
    let r = c.ok(
        "knowledge.role_memory",
        json!({"product": "alpha", "role": "pm"}),
    );
    assert_eq!(r["text"], "- [m-1] Prefers short answers.");
    let r = c.ok(
        "knowledge.recent_minutes",
        json!({"product": "alpha", "limit": 5}),
    );
    assert!(r["minutes"].as_array().unwrap().is_empty());
}

#[test]
fn the_errors_name_their_kind() {
    let dir = TempDir::new();
    let (root, _repo) = setup(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &root)]);
    let mut c = Framed::connect(&core.socket.path);

    let r = c.call("knowledge.index", json!({"product": "nope"}));
    assert_eq!(kind(&r), "not_found", "{r}");
    let r = c.call(
        "knowledge.get_doc",
        json!({"product": "alpha", "kind": "doc", "ref": "x.md"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
    assert_eq!(
        r["error"]["message"],
        "No doc \"x.md\" in alpha. Use search to find valid refs."
    );
    let r = c.call(
        "knowledge.search",
        json!({"product": "alpha", "query": "x", "kind": "nonsense"}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call(
        "knowledge.role_memory",
        json!({"product": "alpha", "role": "Not An Id"}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    assert_eq!(r["error"]["message"], "Invalid agent id \"Not An Id\"");
}
