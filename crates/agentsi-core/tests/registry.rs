//! `registry.get` and `registry.update` on a real core: unknown keys stay, bad paths are
//! refused, a failed write leaves the file as it was, a concurrent `roles.apply` loses no
//! edit, and no commit is made.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn brief(roles: &Value) -> String {
    json!({"version": 1, "product": {"what": "w", "audience": "a", "stage": "idea"},
           "objectives": [{"id": "o1", "text": "t", "signal": "s", "due": "2030-01-01"}],
           "north_stars": [], "roles": roles})
    .to_string()
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

fn registry(work: &Path) -> String {
    format!(
        "{{\n  \"owner\": {{ \"name\": \"Ada\", \"language\": \"English\", \"tz\": \"UTC\" }},\n  \"products\": [\n    {{ \"id\": \"alpha\", \"name\": \"Alpha\", \"paths\": [\"{}\"], \"status\": \"active\", \"roles\": [\"pm\"], \"custom\": true }}\n  ],\n  \"extra\": {{ \"keep\": 1 }}\n}}\n",
        work.display()
    )
}

struct Env {
    dir: TempDir,
    root: PathBuf,
    work: PathBuf,
}

fn env() -> Env {
    let dir = TempDir::new();
    let root = dir.0.join("root");
    let work = dir.0.join("alpha");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(root.join("agentsi.config.json"), registry(&work)).unwrap();
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    conn.execute(
        "insert into products (id, name, paths, status) values ('alpha', 'Alpha', '[]', 'active')",
        [],
    )
    .unwrap();
    drop(conn);
    Env { dir, root, work }
}

fn start(e: &Env) -> CoreProc {
    CoreProc::start_with(&e.dir, &[("AGENTSI_HOME", e.root.as_path())])
}

fn file(e: &Env) -> String {
    std::fs::read_to_string(e.root.join("agentsi.config.json")).unwrap()
}

#[test]
fn registry_get_gives_the_owner_and_the_products() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    let r = c.ok("registry.get", json!({}));
    assert_eq!(r["owner"], json!({"name": "Ada", "language": "English"}));
    assert_eq!(
        r["products"],
        json!([{"id": "alpha", "name": "Alpha", "paths": [e.work.display().to_string()], "status": "active"}])
    );
}

#[test]
fn registry_update_keeps_each_unknown_key_and_writes_the_products_row() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));
    let second = e.dir.0.join("alpha-api");
    std::fs::create_dir_all(&second).unwrap();
    let paths = [e.work.display().to_string(), second.display().to_string()];
    let r = c.ok(
        "registry.update",
        json!({"owner": {"name": "Bea"}, "product": {"id": "alpha", "name": "Alpha 2", "paths": paths}}),
    );
    assert_eq!(r["owner"], json!({"name": "Bea", "language": "English"}));
    assert_eq!(r["products"][0]["name"], "Alpha 2");
    assert_eq!(r["products"][0]["paths"], json!(paths));
    let event = c.next_event("data.changed");
    assert_eq!(
        (event["product"].as_str(), event["kind"].as_str()),
        (Some("alpha"), Some("product"))
    );

    let doc: Value = serde_json::from_str(&file(&e)).unwrap();
    assert_eq!(doc["owner"]["tz"], "UTC");
    assert_eq!(doc["extra"], json!({"keep": 1}));
    assert_eq!(doc["products"][0]["custom"], true);
    assert_eq!(doc["products"][0]["roles"], json!(["pm"]));
    let text = file(&e);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        registry(&e.work).lines().count(),
        "one product on each line"
    );

    let listed = c.ok("product.list", json!({}));
    assert_eq!(listed["products"][0]["name"], "Alpha 2");
}

#[test]
fn registry_update_refuses_a_missing_path_an_empty_list_and_an_unknown_product() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    let before = file(&e);
    let cases = [
        (
            json!({"product": {"id": "alpha", "paths": ["/no/such/folder"]}}),
            "invalid",
        ),
        (
            json!({"product": {"id": "alpha", "paths": ["relative"]}}),
            "invalid",
        ),
        (json!({"product": {"id": "alpha", "paths": []}}), "invalid"),
        (json!({"product": {"id": "alpha", "name": "  "}}), "invalid"),
        (json!({"owner": {"language": ""}}), "invalid"),
        (json!({}), "invalid"),
        (json!({"product": {"id": "nope", "name": "N"}}), "not_found"),
    ];
    for (params, want) in cases {
        let r = c.call("registry.update", params.clone());
        assert_eq!(kind(&r), want, "{params} gave {r}");
    }
    assert_eq!(file(&e), before);
}

#[test]
fn a_failed_write_leaves_the_file_and_the_products_row_as_they_were() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    let before = file(&e);
    // A folder in the place of the temporary file makes the write fail.
    std::fs::create_dir_all(e.root.join("agentsi.config.json.tmp")).unwrap();
    let r = c.call(
        "registry.update",
        json!({"owner": {"name": "Bea"}, "product": {"id": "alpha", "name": "Changed"}}),
    );
    assert_eq!(kind(&r), "internal", "{r}");
    assert_eq!(file(&e), before);
    let listed = c.ok("product.list", json!({}));
    assert_eq!(listed["products"][0]["name"], "Alpha");
}

#[test]
fn a_concurrent_roles_apply_loses_no_edit() {
    let e = env();
    let core = start(&e);
    let socket = core.socket.path.clone();
    for round in 0..5 {
        let socket2 = socket.clone();
        let (roster, roles) = if round % 2 == 0 {
            (json!(["pm", "dev"]), json!({"pm": "x", "dev": "y"}))
        } else {
            (json!(["pm"]), json!({"pm": "x"}))
        };
        let stage = json!({"brief.json": brief(&roles), "roster.json": roster.to_string()});
        let apply = std::thread::spawn(move || {
            let mut c = Framed::connect(&socket2);
            c.ok(
                "roles.apply",
                json!({"product": "alpha", "yes": true,
                       "stage": stage}),
            )
        });
        let mut c = Framed::connect(&socket);
        let name = format!("Alpha {round}");
        c.ok(
            "registry.update",
            json!({"product": {"id": "alpha", "name": name}}),
        );
        let applied = apply.join().unwrap();
        assert_eq!(applied["applied"], true, "{applied}");
        let doc: Value = serde_json::from_str(&file(&e)).unwrap();
        assert_eq!(doc["products"][0]["name"], name);
        assert_eq!(doc["products"][0]["roles"], roster);
    }
}

#[test]
fn registry_update_makes_no_commit_in_a_data_root_that_is_a_git_repo() {
    let e = env();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .current_dir(&e.root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    git(&["init", "-q"]);
    git(&["add", "agentsi.config.json"]);
    git(&["commit", "-q", "-m", "registry"]);
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("registry.update", json!({"owner": {"language": "Spanish"}}));
    assert_eq!(git(&["rev-list", "--count", "HEAD"]).trim(), "1");
    assert!(git(&["status", "--porcelain"]).contains("agentsi.config.json"));
}
