//! `data.clear_plan` and `data.clear` on a real core, on a throwaway data root with two
//! products. Nothing here touches another folder.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir, wait_until};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const TABLES: [&str; 17] = [
    "priorities",
    "usage",
    "documents",
    "index_state",
    "minutes",
    "decisions",
    "stories",
    "story_logs",
    "memory",
    "agent_overlays",
    "briefs",
    "meeting_log",
    "snapshots",
    "listens",
    "reflections",
    "archive_records",
    "products",
];

struct World {
    home: PathBuf,
    repos: PathBuf,
}

fn put(root: &Path, rel: &str, bytes: usize) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}

fn seed(c: &Connection, p: &str) {
    let (m1, m2) = (format!("m1{p}"), format!("m2{p}"));
    c.execute_batch(&format!(
        "insert into products values ('{p}', '{p}', '[]', 'active');
         insert into documents (product, kind, ref, title, body) values
           ('{p}', 'minutes', '{m1}', 't', 'b'), ('{p}', 'minutes', '{m2}', 't', 'b'),
           ('{p}', 'memory', 'dev', 't', 'b'), ('{p}', 'doc', 'README', 't', 'b');
         insert into index_state values ('{p}', 'a.rs', 'abc', 'x');
         insert into minutes (id, product, created_at, data) values
           ('{m1}', '{p}', '2026-01-01T00:00:00.000Z', '{{}}'),
           ('{m2}', '{p}', '2026-10-01T00:00:00.000Z', '{{}}');
         insert into priorities (minutes_id, idx) values ('{m1}', 0), ('{m2}', 0);
         insert into decisions (product, minutes_id, text) values ('{p}', '{m1}', 'd1'), ('{p}', '{m2}', 'd2');
         insert into usage (minutes_id, role) values ('{m1}', 'pm'), ('{m2}', 'pm');
         insert into stories values
           ('{p}', 'S001', '{{\"status\":\"implemented\"}}'),
           ('{p}', 'S002', '{{\"status\":\"discarded\"}}'),
           ('{p}', 'S003', '{{\"status\":\"not_implemented\"}}');
         insert into story_logs values ('{p}', 'S001', 'l'), ('{p}', 'S003', 'l');
         insert into memory values ('{p}', 'dev', 'i1', 0, 't', 'fact', 0, 0, 's', 'x', null, null);
         insert into agent_overlays values ('{p}', 'dev', 'o');
         insert into briefs values ('{p}', '{{}}');
         insert into meeting_log values ('{p}', '{m1}', 0, '{{}}'), ('{p}', '{m2}', 1, '{{}}');
         insert into snapshots values ('{p}', '{m1}', 's'), ('{p}', '{m2}', 's');
         insert into listens values ('{p}', '{m1}', 'x'), ('{p}', '{m2}', 'x');
         insert into reflections values ('{p}', 'r1', 0, '{{}}');
         insert into archive_records values
           ('{p}', 'minutes', '{m1}'), ('{p}', 'story', 'S001'), ('{p}', 'story', 'S003');"
    ))
    .unwrap();
}

fn setup(dir: &TempDir) -> World {
    let home = dir.0.join("root");
    let repos = dir.0.join("repos");
    std::fs::create_dir_all(&home).unwrap();
    for p in ["alpha", "beta"] {
        std::fs::create_dir_all(repos.join(p)).unwrap();
    }
    let registry = format!(
        "{{\n  \"owner\": {{ \"name\": \"Ada\", \"language\": \"English\" }},\n  \"products\": [\n    {{ \"id\": \"alpha\", \"name\": \"Alpha\", \"paths\": [{:?}], \"status\": \"active\", \"roles\": [\"pm\"] }},\n    {{ \"id\": \"beta\", \"name\": \"Beta\", \"paths\": [{:?}], \"status\": \"active\", \"roles\": [\"pm\"] }}\n  ]\n}}\n",
        repos.join("alpha"),
        repos.join("beta")
    );
    std::fs::write(home.join("agentsi.config.json"), registry).unwrap();
    let conn = agentsi_store::db::open_writer(&home.join("agentsi.db")).unwrap();
    for p in ["alpha", "beta"] {
        seed(&conn, p);
        for m in ["m1", "m2"] {
            put(&home, &format!("workspace/{p}/audio/{m}{p}/call.opus"), 100);
        }
        put(&home, &format!("workspace/{p}/notes.txt"), 7);
        put(&repos, &format!("{p}/src.rs"), 9);
    }
    put(&home, "voice/transcripts/old.md", 10);
    put(&home, "voice/transcripts/new.md", 10);
    let f = std::fs::File::options()
        .write(true)
        .open(home.join("voice/transcripts/old.md"))
        .unwrap();
    f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
        .unwrap();
    put(&home, "logs/hook.log", 50);
    for d in ["2020-01-01", "2020-01-02"] {
        put(&home, &format!("backups/agentsi-{d}.db"), 20);
        put(&home, &format!("backups/agentsi.config-{d}.json"), 5);
    }
    World { home, repos }
}

fn start(dir: &TempDir, w: &World) -> CoreProc {
    CoreProc::start_with(dir, &[("AGENTSI_HOME", w.home.as_path())])
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

fn item(item: &str, product: Option<&str>, before: Option<&str>) -> Value {
    let mut v = json!({"item": item});
    if let Some(p) = product {
        v["product"] = json!(p);
    }
    if let Some(b) = before {
        v["before"] = json!(b);
    }
    v
}

/// The rows of one product, per table.
fn counts(home: &Path, p: &str) -> BTreeMap<&'static str, i64> {
    let c = Connection::open(home.join("agentsi.db")).unwrap();
    let by_minutes = "minutes_id in (select id from minutes where product = ?1)";
    TABLES
        .iter()
        .map(|t| {
            let pred = match *t {
                "priorities" | "usage" => by_minutes,
                "products" => "id = ?1",
                _ => "product = ?1",
            };
            let n = c
                .query_row(
                    &format!("select count(*) from {t} where {pred}"),
                    [p],
                    |r| r.get(0),
                )
                .unwrap();
            (*t, n)
        })
        .collect()
}

fn total(home: &Path) -> i64 {
    let c = Connection::open(home.join("agentsi.db")).unwrap();
    TABLES
        .iter()
        .map(|t| {
            c.query_row(&format!("select count(*) from {t}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
        .sum()
}

/// Each file under `dir`, relative, with its size.
fn tree(dir: &Path) -> Vec<(String, u64)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            let rel = path.strip_prefix(base).unwrap().display().to_string();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.push((rel, std::fs::metadata(&path).unwrap().len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn plan_and_clear(c: &mut Framed, items: Value) -> (Value, Value) {
    let plan = c.ok("data.clear_plan", json!({"items": items}));
    let done = c.ok(
        "data.clear",
        json!({"items": items, "plan_id": plan["plan_id"]}),
    );
    (plan, done)
}

#[test]
fn each_item_deletes_its_data_and_the_other_product_keeps_every_row_and_file() {
    let cases: Vec<(&str, Vec<Value>)> = vec![
        ("product", vec![item("product", Some("alpha"), None)]),
        ("meetings", vec![item("meetings", Some("alpha"), None)]),
        (
            "meetings old",
            vec![item("meetings", Some("alpha"), Some("2026-06-01"))],
        ),
        ("stories", vec![item("stories", Some("alpha"), None)]),
        (
            "role learning",
            vec![item("role_learning", Some("alpha"), None)],
        ),
        ("call audio", vec![item("call_audio", Some("alpha"), None)]),
    ];
    for (name, items) in cases {
        let dir = TempDir::new();
        let w = setup(&dir);
        let core = start(&dir, &w);
        let mut c = Framed::connect(&core.socket.path);
        let beta_rows = counts(&w.home, "beta");
        let beta_files = tree(&w.home.join("workspace/beta"));
        let alpha_rows = counts(&w.home, "alpha");
        let before = total(&w.home);

        let (plan, done) = plan_and_clear(&mut c, json!(items));
        assert_ne!(
            plan["rows"].as_u64().unwrap() + plan["files"].as_u64().unwrap(),
            0,
            "{name}"
        );
        assert_eq!(
            done["rows"], plan["rows"],
            "{name}: the plan equals the delete"
        );
        assert_eq!(
            before - total(&w.home),
            plan["rows"].as_i64().unwrap(),
            "{name}: the plan equals the deleted rows"
        );
        assert_ne!(
            counts(&w.home, "alpha"),
            alpha_rows,
            "{name}: alpha changed"
        );
        assert_eq!(counts(&w.home, "beta"), beta_rows, "{name}: beta rows");
        assert_eq!(
            tree(&w.home.join("workspace/beta")),
            beta_files,
            "{name}: beta files"
        );
        assert_eq!(tree(&w.repos.join("beta")), [("src.rs".to_string(), 9)]);
        assert_eq!(
            tree(&w.repos.join("alpha")),
            [("src.rs".to_string(), 9)],
            "{name}: no repo file goes"
        );
        assert_eq!(done["failed"], json!([]), "{name}");
        let backup = PathBuf::from(done["backup"].as_str().unwrap());
        assert!(
            backup.starts_with(w.home.join("backups")) && backup.exists(),
            "{name}"
        );
    }
}

#[test]
fn a_product_leaves_the_registry_and_the_backup_holds_the_old_rows() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));
    let items = json!([item("product", Some("alpha"), None)]);
    let (plan, done) = plan_and_clear(&mut c, items);
    assert_eq!(plan["items"][0]["registry"], json!(true));
    assert_eq!(done["registry_removed"], json!(["alpha"]));
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "alpha", "kind": "clear"})
    );
    let registry = std::fs::read_to_string(w.home.join("agentsi.config.json")).unwrap();
    assert!(!registry.contains("alpha") && registry.contains("beta") && registry.contains("owner"));
    let copy = done["registry_backup"].as_str().unwrap();
    assert!(std::fs::read_to_string(copy).unwrap().contains("alpha"));
    // The backup is the way back: it has the rows that the delete removed.
    let b = Connection::open(done["backup"].as_str().unwrap()).unwrap();
    let n: i64 = b
        .query_row(
            "select count(*) from minutes where product = 'alpha'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
    // The file of the database shrinks off the request thread.
    wait_until("the checkpoint", || {
        std::fs::metadata(w.home.join("agentsi.db-wal")).map_or(true, |m| m.len() == 0)
    });
}

#[test]
fn files_items_go_by_date_and_the_newest_backup_stays() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    let items = json!([
        item("voice_transcripts", None, Some("2000-01-01")),
        item("logs", None, None),
        item("backups", None, None),
    ]);
    let plan = c.ok("data.clear_plan", json!({"items": items}));
    assert_eq!(
        plan["items"][0]["files"],
        json!([]),
        "nothing is older than 2000"
    );
    let items = json!([
        item("voice_transcripts", None, Some("2026-01-01")),
        item("logs", None, None),
        item("backups", None, None),
    ]);
    let (plan, done) = plan_and_clear(&mut c, items);
    assert!(plan["root_bytes"].as_u64().unwrap() > 0);
    assert_eq!(done["failed"], json!([]));
    let left: Vec<String> = tree(&w.home).into_iter().map(|t| t.0).collect();
    assert!(left.contains(&"voice/transcripts/new.md".to_string()));
    assert!(!left.contains(&"voice/transcripts/old.md".to_string()));
    assert!(!left.contains(&"logs/hook.log".to_string()));
    // The backup that the delete made is the newest. The 2020 backups are the old ones.
    assert!(!left.contains(&"backups/agentsi-2020-01-01.db".to_string()));
    assert!(!left.contains(&"backups/agentsi-2020-01-02.db".to_string()));
    assert!(PathBuf::from(done["backup"].as_str().unwrap()).exists());
}

#[test]
fn everything_empties_the_database_and_keeps_the_root_and_the_newest_backup() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    let (_, done) = plan_and_clear(&mut c, json!([item("everything", None, None)]));
    assert_eq!(total(&w.home), 0);
    assert_eq!(done["registry_removed"], json!(["alpha", "beta"]));
    let left: Vec<String> = tree(&w.home)
        .into_iter()
        .map(|t| t.0)
        .filter(|p| p != "agentsi.db" && !p.starts_with("agentsi.db-"))
        .collect();
    let backup = PathBuf::from(done["backup"].as_str().unwrap());
    let registry_copy = done["registry_backup"].as_str().unwrap();
    let mut want: Vec<String> = vec![
        "agentsi.config.json".into(),
        format!("backups/{}", backup.file_name().unwrap().to_str().unwrap()),
        format!(
            "backups/{}",
            Path::new(registry_copy)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
        ),
    ];
    want.sort();
    assert_eq!(left, want);
    assert_eq!(
        tree(&w.repos.join("alpha")).len() + tree(&w.repos.join("beta")).len(),
        2,
        "no repo file goes"
    );
}

#[test]
fn a_changed_store_between_plan_and_delete_gives_plan_changed_and_deletes_nothing() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    let items = json!([item("meetings", Some("alpha"), None)]);
    let plan = c.ok("data.clear_plan", json!({"items": items}));
    {
        let db = Connection::open(w.home.join("agentsi.db")).unwrap();
        db.execute(
            "insert into minutes (id, product, created_at, data) values ('m3alpha', 'alpha', '2026-10-02T00:00:00.000Z', '{}')",
            [],
        )
        .unwrap();
    }
    let before = (total(&w.home), tree(&w.home));
    let reply = c.call(
        "data.clear",
        json!({"items": items, "plan_id": plan["plan_id"]}),
    );
    assert_eq!(kind(&reply), "plan_changed", "{reply}");
    assert_eq!(
        (total(&w.home), tree(&w.home)),
        before,
        "nothing was deleted and no backup was made"
    );

    // A plan serves one delete, and a changed file list also changes the plan.
    let plan = c.ok("data.clear_plan", json!({"items": items}));
    put(&w.home, "workspace/alpha/audio/m3alpha/call.opus", 1);
    let reply = c.call(
        "data.clear",
        json!({"items": items, "plan_id": plan["plan_id"]}),
    );
    assert_eq!(kind(&reply), "plan_changed", "{reply}");
    let again = c.call(
        "data.clear",
        json!({"items": items, "plan_id": plan["plan_id"]}),
    );
    assert_eq!(kind(&again), "plan_changed", "a used plan is gone");
    let other = c.ok("data.clear_plan", json!({"items": items}));
    let different = json!([item("meetings", Some("beta"), None)]);
    let reply = c.call(
        "data.clear",
        json!({"items": different, "plan_id": other["plan_id"]}),
    );
    assert_eq!(
        kind(&reply),
        "plan_changed",
        "the items differ from the plan"
    );
    let reply = c.call("data.clear", json!({"items": items, "plan_id": "plan-0-0"}));
    assert_eq!(kind(&reply), "plan_changed", "an unknown plan");
}

#[test]
fn a_failed_backup_deletes_nothing() {
    let dir = TempDir::new();
    let w = setup(&dir);
    // A file where the folder of the backups should be: the backup cannot write.
    std::fs::remove_dir_all(w.home.join("backups")).unwrap();
    std::fs::write(w.home.join("backups"), "not a folder").unwrap();
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    let items = json!([item("product", Some("alpha"), None)]);
    let plan = c.ok("data.clear_plan", json!({"items": items}));
    let before = (total(&w.home), tree(&w.home));
    let reply = c.call(
        "data.clear",
        json!({"items": items, "plan_id": plan["plan_id"]}),
    );
    assert_eq!(kind(&reply), "unavailable", "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("backup")
    );
    assert_eq!((total(&w.home), tree(&w.home)), before);
    assert!(
        std::fs::read_to_string(w.home.join("agentsi.config.json"))
            .unwrap()
            .contains("alpha")
    );
}

#[test]
fn a_product_with_a_running_pane_is_busy_and_the_other_product_is_not() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    let term = c.ok(
        "terminal.create",
        json!({"cwd": w.home, "argv": ["sh", "-c", "sleep 60"], "product": "alpha"}),
    );
    let id = term["id"].as_str().unwrap().to_string();
    let items = json!([item("meetings", Some("alpha"), None)]);
    let plan = c.ok("data.clear_plan", json!({"items": items}));
    let before = (total(&w.home), tree(&w.home));
    let reply = c.call(
        "data.clear",
        json!({"items": items, "plan_id": plan["plan_id"]}),
    );
    assert_eq!(kind(&reply), "busy", "{reply}");
    assert!(reply["error"]["message"].as_str().unwrap().contains(&id));
    assert_eq!((total(&w.home), tree(&w.home)), before);

    let beta = json!([
        item("logs", None, None),
        item("meetings", Some("beta"), None)
    ]);
    plan_and_clear(&mut c, beta);
    // Everything waits for each pane.
    let all = json!([item("everything", None, None)]);
    let plan = c.ok("data.clear_plan", json!({"items": all}));
    let reply = c.call(
        "data.clear",
        json!({"items": all, "plan_id": plan["plan_id"]}),
    );
    assert_eq!(kind(&reply), "busy");

    c.ok("terminal.kill", json!({"id": id}));
    let items = json!([item("meetings", Some("alpha"), None)]);
    let (_, done) = plan_and_clear(&mut c, items);
    assert!(done["rows"].as_u64().unwrap() > 0);
}

#[test]
fn a_file_that_cannot_go_is_listed_and_the_rows_stay_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    let locked = w.home.join("workspace/alpha/audio/m1alpha");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let (_, done) = plan_and_clear(&mut c, json!([item("call_audio", Some("alpha"), None)]));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let failed: Vec<&str> = done["failed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(failed, ["workspace/alpha/audio/m1alpha"]);
    assert_eq!(counts(&w.home, "alpha")["listens"], 0);
    assert!(!w.home.join("workspace/alpha/audio/m2alpha").exists());
}

#[test]
fn a_bad_item_and_a_repo_inside_the_data_root_are_refused() {
    let dir = TempDir::new();
    let w = setup(&dir);
    let core = start(&dir, &w);
    let mut c = Framed::connect(&core.socket.path);
    let root = c.ok("data.clear_plan", json!({"items": []}));
    assert_eq!(
        (root["rows"].clone(), root["files"].clone()),
        (json!(0), json!(0))
    );
    assert!(root["data_root"].as_str().unwrap().ends_with("root"));
    let reply = c.call(
        "data.clear",
        json!({"items": [], "plan_id": root["plan_id"]}),
    );
    assert_eq!(kind(&reply), "invalid", "{reply}");
    for bad in [
        json!([item("product", Some("../repos/alpha"), None)]),
        json!([item("product", None, None)]),
        json!([item("meetings", Some("alpha"), Some("last week"))]),
        json!([item("logs", Some("alpha"), None)]),
    ] {
        let reply = c.call("data.clear_plan", json!({"items": bad}));
        assert_eq!(kind(&reply), "invalid", "{bad}: {reply}");
    }
    let reply = c.call(
        "data.clear_plan",
        json!({"items": [item("product", Some("ghost"), None)]}),
    );
    assert_eq!(kind(&reply), "not_found", "{reply}");

    // A product repo inside its workspace folder is never deleted.
    let inside = w.home.join("workspace/alpha/repo");
    std::fs::create_dir_all(&inside).unwrap();
    let text = std::fs::read_to_string(w.home.join("agentsi.config.json")).unwrap();
    let text = text.replace(
        w.repos.join("alpha").to_str().unwrap(),
        inside.to_str().unwrap(),
    );
    std::fs::write(w.home.join("agentsi.config.json"), text).unwrap();
    let reply = c.call(
        "data.clear_plan",
        json!({"items": [item("product", Some("alpha"), None)]}),
    );
    assert_eq!(kind(&reply), "invalid", "{reply}");
    assert!(reply["error"]["message"].as_str().unwrap().contains("repo"));
}
