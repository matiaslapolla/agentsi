//! The setup screen in the core: the inline stage of `roles.apply`, the `products` row
//! that it writes, `setup.scan`, `setup.draft` and `setup.outcome`.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const REGISTRY: &str = "{\n  \"owner\": { \"name\": \"Ada\", \"language\": \"English\" },\n  \"products\": [\n    { \"id\": \"alpha\", \"name\": \"Alpha\", \"paths\": [\"/work/alpha\"], \"status\": \"active\", \"roles\": [\"pm\"] }\n  ]\n}\n";
const BRIEF: &str = r#"{"version":1,"product":{"what":"w","audience":"a","stage":"idea"},"objectives":[{"id":"o1","text":"t","signal":"s","due":"2030-01-01"}],"north_stars":[],"roles":{"pm":"x","dev":"y","dev.api":"z"}}"#;
const ROSTER: &str = r#"["pm","dev","dev.api"]"#;
const OVERLAY: &str = "---\nextends: dev\n---\nAPI.";

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

struct Env {
    dir: TempDir,
    root: PathBuf,
    tmp: PathBuf,
}

fn env() -> Env {
    let dir = TempDir::new();
    let root = dir.0.join("root");
    let tmp = dir.0.join("tmp");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    drop(agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap());
    Env { dir, root, tmp }
}

fn start(e: &Env) -> CoreProc {
    CoreProc::start_with(
        &e.dir,
        &[
            ("AGENTSI_HOME", e.root.as_path()),
            ("TMPDIR", e.tmp.as_path()),
        ],
    )
}

fn inline() -> Value {
    json!({"brief.json": BRIEF, "roster.json": ROSTER, "agents/dev.api.md": OVERLAY})
}

fn folder(at: &Path) -> String {
    std::fs::create_dir_all(at.join("agents")).unwrap();
    std::fs::write(at.join("brief.json"), BRIEF).unwrap();
    std::fs::write(at.join("roster.json"), ROSTER).unwrap();
    std::fs::write(at.join("agents/dev.api.md"), OVERLAY).unwrap();
    at.display().to_string()
}

#[test]
fn inline_stage_gives_the_plan_of_the_same_files_in_a_folder() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    let stage_dir = folder(&e.dir.0.join("stage"));
    let from_folder = c.ok(
        "roles.apply",
        json!({"product": "alpha", "stage_dir": stage_dir}),
    );
    let from_text = c.ok(
        "roles.apply",
        json!({"product": "alpha", "stage": inline()}),
    );
    assert!(from_folder["plan"].is_object(), "{from_folder}");
    assert_eq!(from_text, from_folder);
    let left: Vec<_> = std::fs::read_dir(&e.tmp).unwrap().collect();
    assert!(left.is_empty(), "the inline stage stays: {left:?}");
}

#[test]
fn inline_stage_refuses_both_neither_and_a_key_outside_the_stage_names() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    let stage_dir = folder(&e.dir.0.join("stage"));
    let bad = [
        json!({"product": "alpha", "stage_dir": stage_dir, "stage": inline()}),
        json!({"product": "alpha"}),
        json!({"product": "alpha", "stage": {"../brief.json": BRIEF, "roster.json": ROSTER}}),
        json!({"product": "alpha", "stage": {"brief.json": BRIEF, "roster.json": ROSTER, "agents/a/b.md": "x"}}),
        json!({"product": "alpha", "stage": {"brief.json": BRIEF, "roster.json": ROSTER, "notes.txt": "x"}}),
    ];
    for params in bad {
        let r = c.call("roles.apply", params.clone());
        assert_eq!(kind(&r), "invalid", "{params} gave {r}");
    }
    let left: Vec<_> = std::fs::read_dir(&e.tmp).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn register_writes_the_products_row_and_sends_data_changed_of_kind_product() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));
    let path = e.dir.0.join("beta");
    std::fs::create_dir_all(&path).unwrap();
    let r = c.ok(
        "roles.apply",
        json!({"product": "beta", "stage": inline(), "yes": true,
               "register": {"name": "Beta", "path": path.display().to_string()}}),
    );
    assert_eq!(r["applied"], true, "{r}");
    let event = c.next_event("data.changed");
    assert_eq!(
        (event["product"].as_str(), event["kind"].as_str()),
        (Some("beta"), Some("product"))
    );
    let listed = c.ok("product.list", json!({}));
    let beta = listed["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "beta")
        .cloned()
        .unwrap_or_else(|| panic!("no beta in {listed}"));
    assert_eq!(beta["name"], "Beta");
    let registry = std::fs::read_to_string(e.root.join("agentsi.config.json")).unwrap();
    assert!(registry.contains("\"id\": \"beta\""), "{registry}");
}
