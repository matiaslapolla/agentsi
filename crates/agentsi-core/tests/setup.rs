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

#[test]
fn scan_reads_the_id_the_manifest_commands_and_the_north_stars_of_a_folder() {
    let e = env();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    let app = e.dir.0.join("My App");
    std::fs::create_dir_all(app.join("docs")).unwrap();
    std::fs::write(
        app.join("package.json"),
        r#"{"scripts":{"test":"node --test","typecheck":"tsc","lint":"eslint ."}}"#,
    )
    .unwrap();
    std::fs::write(app.join("pnpm-lock.yaml"), "").unwrap();
    std::fs::write(app.join("docs/product.md"), "# Product").unwrap();
    std::fs::write(app.join("docs/vision-2027.md"), "# Vision").unwrap();
    std::fs::write(app.join("ROADMAP.md"), "# Roadmap").unwrap();
    std::fs::write(app.join("docs/notes.md"), "# Notes").unwrap();

    let r = c.ok("setup.scan", json!({"path": app.display().to_string()}));
    assert_eq!(r["id"], "my-app");
    assert_eq!(r["name"], "My App");
    assert_eq!(r["registered"], Value::Null);
    assert_eq!(
        r["manifest"],
        json!({"file": "package.json", "test": "pnpm test {files}",
               "gate": ["pnpm typecheck", "pnpm lint"]})
    );
    assert_eq!(
        r["north_stars"],
        json!(["ROADMAP.md", "docs/product.md", "docs/vision-2027.md"])
    );
    assert!(r["tools"]["bwrap"].is_boolean());
    let left = std::fs::read_dir(&app).unwrap().count();
    assert_eq!(left, 4, "the scan writes nothing");
}

#[test]
fn scan_finds_a_registered_folder_and_refuses_a_bad_path() {
    let e = env();
    let work = e.dir.0.join("alpha");
    std::fs::create_dir_all(work.join("src")).unwrap();
    let registry = REGISTRY.replace("/work/alpha", &work.display().to_string());
    std::fs::write(e.root.join("agentsi.config.json"), registry).unwrap();
    let core = start(&e);
    let mut c = Framed::connect(&core.socket.path);
    let r = c.ok(
        "setup.scan",
        json!({"path": work.join("src").display().to_string()}),
    );
    assert_eq!(r["registered"], "alpha");
    assert_eq!(r["manifest"], Value::Null);
    assert_eq!(r["north_stars"], json!([]));
    for bad in ["relative/path", "/no/such/folder"] {
        let r = c.call("setup.scan", json!({"path": bad}));
        assert_eq!(kind(&r), "invalid", "{bad} gave {r}");
    }
}

fn fake_claude(e: &Env, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let bin = e.dir.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let file = bin.join("claude");
    std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("{}:/usr/bin:/bin", bin.display())
}

fn log_lines(e: &Env) -> Vec<Value> {
    std::fs::read_to_string(e.root.join("logs/setup.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn draft_runs_one_capped_call_with_no_tools_and_logs_one_line() {
    let e = env();
    let path = fake_claude(
        &e,
        r#"cat > "$(dirname "$0")/prompt.txt"; printf '%s\n' "$@" > "$(dirname "$0")/args.txt"
echo '{"is_error":false,"total_cost_usd":0.07,"duration_ms":2100,"structured_output":{"fields":{"product":{"id":"app","name":"App","what":"A mail filter","audience":"","stage":"","out_of_scope":[]},"objectives":{"objectives":[],"north_stars":[],"sources_of_truth":[]},"roster":{"roles":["pm"],"specialists":[],"why":[]},"models":[],"stories":{"on":false,"test":"","gate":[]}},"no_data":["product.audience","product.stage"]}}'"#,
    );
    let core = CoreProc::start_with(
        &e.dir,
        &[
            ("AGENTSI_HOME", e.root.as_path()),
            ("TMPDIR", e.tmp.as_path()),
            ("PATH", Path::new(&path)),
        ],
    );
    let mut c = Framed::connect(&core.socket.path);
    let app = e.dir.0.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("README.md"), "# App\nA mail filter.").unwrap();

    let r = c.ok("setup.draft", json!({"path": app.display().to_string()}));
    assert_eq!(r["fields"]["product"]["what"], "A mail filter");
    assert_eq!(r["no_data"], json!(["product.audience", "product.stage"]));
    assert_eq!(r["cost_usd"], 0.07);
    let draft = r["draft"].as_str().unwrap().to_string();

    let args = std::fs::read_to_string(e.dir.0.join("bin/args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    let after = |flag: &str| args[args.iter().position(|a| *a == flag).unwrap() + 1];
    assert_eq!(after("--max-budget-usd"), "0.5");
    assert_eq!(after("--model"), "sonnet");
    assert!(args.contains(&"--tools="), "{args:?}");
    assert!(args.contains(&"--json-schema"), "{args:?}");
    let prompt = std::fs::read_to_string(e.dir.0.join("bin/prompt.txt")).unwrap();
    assert!(prompt.contains("## File README.md"), "{prompt}");

    let lines = log_lines(&e);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["kind"], "draft");
    assert_eq!(lines[0]["draft"], draft.as_str());
    assert_eq!(lines[0]["cost_usd"], 0.07);
    assert_eq!(lines[0]["files"], 1);

    c.ok(
        "setup.outcome",
        json!({"draft": draft, "product": "app",
               "fields": {"product.what": "kept", "product.name": "edited", "roster.roles": "cleared"}}),
    );
    let lines = log_lines(&e);
    assert_eq!(lines.len(), 2);
    assert_eq!(
        (
            &lines[1]["kind"],
            &lines[1]["kept"],
            &lines[1]["edited"],
            &lines[1]["cleared"]
        ),
        (&json!("outcome"), &json!(1), &json!(1), &json!(1))
    );
}

#[test]
fn a_failed_draft_call_is_internal_and_logs_nothing() {
    let e = env();
    let path = fake_claude(
        &e,
        r#"cat >/dev/null; echo '{"is_error":true,"subtype":"error_max_budget_usd","total_cost_usd":0.51}'; exit 1"#,
    );
    let core = CoreProc::start_with(
        &e.dir,
        &[
            ("AGENTSI_HOME", e.root.as_path()),
            ("PATH", Path::new(&path)),
        ],
    );
    let mut c = Framed::connect(&core.socket.path);
    let r = c.call(
        "setup.draft",
        json!({"path": e.dir.0.display().to_string()}),
    );
    assert_eq!(kind(&r), "internal", "{r}");
    assert!(log_lines(&e).is_empty());
    let r = c.call("setup.draft", json!({"path": "not/absolute"}));
    assert_eq!(kind(&r), "invalid", "{r}");
}
