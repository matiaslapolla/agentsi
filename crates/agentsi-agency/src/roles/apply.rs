//! `roles apply`: `roster-apply.ts`. A stage is a folder with `brief.json`,
//! `roster.json`, `agents/<id>.md` and, optionally, `stories.json`. The plan
//! compares it with the database. `write_plan` stores it: the brief and the
//! overlays into the database, the roster (and the stories block) into the
//! registry. The registry write is the last step, so a failed edit rolls the
//! database back.

use super::files::RoleFiles;
use super::load::describe_roster;
use super::registry_edit::{Patch, add_product, update_product};
use crate::registry::{FILE, Product};
use crate::roster::{Brief, check_roster, js_text, read_overlays};
use agentsi_proto::roles_api::{ApplyPlan, ChangeStatus, FileChange};
use agentsi_store::read;
use rusqlite::{Connection, Transaction, params};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The stage in `stories.json`.
#[derive(Debug, Clone, PartialEq)]
enum StageStories {
    /// No file: leave the block as it is.
    Absent,
    /// `null`: turn Stories mode off.
    Off,
    Block(Value),
}

#[derive(Debug, Clone)]
pub struct Stage {
    roster: Vec<String>,
    /// Stage-relative path to file text: `brief.json` and `agents/<id>.md`.
    files: BTreeMap<String, String>,
    brief: Value,
    overlays: BTreeMap<String, String>,
    stories: StageStories,
}

/// A plan with the stage that it comes from.
#[derive(Debug, Clone)]
pub struct Planned {
    pub plan: ApplyPlan,
    stage: Stage,
    /// The product as the registry will hold it.
    product: Product,
}

pub type Outcome = std::result::Result<Planned, Vec<String>>;

/// The outcome of a plan, and the warnings that loading the roster gave.
pub type Planning = (Outcome, Vec<String>);

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn read_stage(dir: &Path) -> (Option<Stage>, Vec<String>) {
    let mut failures = Vec::new();
    let text = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    let brief_text = text("brief.json");
    let roster_text = text("roster.json");
    if brief_text.is_none() {
        failures.push("Stage has no brief.json.".to_string());
    }
    if roster_text.is_none() {
        failures.push("Stage has no roster.json.".to_string());
    }
    let mut roster = Vec::new();
    if let Some(t) = &roster_text {
        match serde_json::from_str::<Value>(t) {
            Ok(Value::Array(items)) if items.iter().all(Value::is_string) => {
                roster = items
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect();
            }
            Ok(_) => failures.push("roster.json must be an array of agent ids.".into()),
            Err(e) => failures.push(format!("roster.json is not valid JSON: {e}")),
        }
    }
    let mut brief = None;
    if let Some(t) = &brief_text {
        match serde_json::from_str::<Value>(t) {
            Ok(v) => brief = Some(v),
            Err(e) => failures.push(format!("brief.json is not valid JSON: {e}")),
        }
    }
    let mut stories = StageStories::Absent;
    if let Some(t) = text("stories.json") {
        match serde_json::from_str::<Value>(&t) {
            Ok(Value::Null) => stories = StageStories::Off,
            Ok(v @ Value::Object(_)) => stories = StageStories::Block(v),
            Ok(_) => failures.push("stories.json must be an object or null.".into()),
            Err(e) => failures.push(format!("stories.json is not valid JSON: {e}")),
        }
    }
    let mut files = BTreeMap::new();
    if let Some(t) = brief_text {
        files.insert("brief.json".to_string(), t);
    }
    let mut overlays = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("agents")) {
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".md"))
            .collect();
        names.sort();
        for name in names {
            let body = std::fs::read_to_string(dir.join("agents").join(&name)).unwrap_or_default();
            overlays.insert(name[..name.len() - 3].to_string(), body.clone());
            files.insert(format!("agents/{name}"), body);
        }
    }
    // A brief that is `null` (or another falsy value) gives no stage and no failure, as in TypeScript.
    match brief {
        Some(brief) if failures.is_empty() && truthy(&brief) => (
            Some(Stage {
                roster,
                files,
                brief,
                overlays,
                stories,
            }),
            failures,
        ),
        _ => (None, failures),
    }
}

/// The block with its keys in registry order, so two blocks compare by value.
pub fn normal_stories(block: &Value) -> Value {
    let mut out = Map::new();
    for key in ["repo", "test", "gate", "budget_usd"] {
        if let Some(v) = block.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    Value::Object(out)
}

fn stories_problems(product: &Product, block: &Value) -> Vec<String> {
    let allowed = ["repo", "test", "gate", "budget_usd"];
    let extra: Vec<&str> = block
        .as_object()
        .into_iter()
        .flat_map(|m| m.keys().map(String::as_str))
        .filter(|k| !allowed.contains(k))
        .collect();
    if !extra.is_empty() {
        return vec![format!(
            "stories.json has unknown keys: {}.",
            extra.join(", ")
        )];
    }
    let repo = block.get("repo");
    if let Some(r) = repo {
        let ok = r
            .as_str()
            .is_some_and(|p| Path::new(p).is_absolute() && Path::new(p).is_dir());
        if !ok {
            return vec![format!(
                "stories.json repo must be an existing absolute path to a directory, got \"{}\".",
                js_text(Some(r))
            )];
        }
    }
    let id = &product.id;
    if repo.is_none() && product.paths.is_empty() {
        return vec![format!("Product {id} has no path for its stories repo.")];
    }
    if !block
        .get("test")
        .and_then(Value::as_str)
        .is_some_and(|t| t.contains("{files}"))
    {
        return vec![format!(
            "The stories block of {id} needs a test command with {{files}}."
        )];
    }
    let gate_ok = block
        .get("gate")
        .and_then(Value::as_array)
        .is_some_and(|g| {
            !g.is_empty()
                && g.iter()
                    .all(|c| c.as_str().is_some_and(|s| !s.trim().is_empty()))
        });
    if !gate_ok {
        return vec![format!(
            "The stories block of {id} needs gate as a list of commands."
        )];
    }
    let budget = block.get("budget_usd");
    let budget_ok = budget.is_none_or(|b| b.as_f64().is_some_and(|f| f.is_finite() && f > 0.0));
    if !budget_ok {
        return vec![format!(
            "The stories block of {id} needs budget_usd as a positive number of USD, got {}.",
            budget.map_or("undefined".into(), Value::to_string)
        )];
    }
    Vec::new()
}

fn unified_diff(before: &str, after: &str) -> String {
    let Ok(dir) = tempfile::tempdir() else {
        return String::new();
    };
    if std::fs::write(dir.path().join("before"), before).is_err()
        || std::fs::write(dir.path().join("after"), after).is_err()
    {
        return String::new();
    }
    let out = Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--no-color",
            "--unified=2",
            "--",
            "before",
            "after",
        ])
        .current_dir(dir.path())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    text.split('\n')
        .filter(|l| {
            !(l.starts_with("diff --git")
                || l.starts_with("index ")
                || l.starts_with("--- ")
                || l.starts_with("+++ "))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// What the database holds, as the stage files it would come from. A brief
/// equal to the stage's keeps the stage's text.
fn current_files(
    c: &Connection,
    product: &str,
    stage: &Stage,
) -> super::Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    if let Some(brief) = read::brief(c, product)? {
        let text = if brief == stage.brief {
            stage.files["brief.json"].clone()
        } else {
            serde_json::to_string_pretty(&brief).unwrap_or_default() + "\n"
        };
        files.insert("brief.json".to_string(), text);
    }
    for (id, text) in read_overlays(c, product)? {
        files.insert(format!("agents/{id}.md"), text);
    }
    Ok(files)
}

/// The stories block that the registry holds for a product, if any.
fn registry_stories(text: &str, id: &str) -> Option<Value> {
    let doc: Value = serde_json::from_str(text).ok()?;
    doc.get("products")?
        .as_array()?
        .iter()
        .find(|p| p.get("id").and_then(Value::as_str) == Some(id))?
        .get("stories")
        .cloned()
}

fn stories_changed(before: &Option<Value>, after: &Option<Value>) -> bool {
    let norm = |s: &Option<Value>| {
        s.as_ref()
            .map(|b| super::registry_edit::one_line(&normal_stories(b)))
    };
    norm(before) != norm(after)
}

fn edit_registry(text: &str, planned: &Planned) -> std::result::Result<String, String> {
    let plan = &planned.plan;
    if plan.register {
        let mut p = Map::new();
        p.insert("id".into(), planned.product.id.clone().into());
        p.insert("name".into(), planned.product.name.clone().into());
        p.insert("paths".into(), planned.product.paths.clone().into());
        p.insert("status".into(), planned.product.status.clone().into());
        p.insert("roles".into(), plan.roster_after.clone().into());
        if let Some(s) = &plan.stories_after {
            p.insert("stories".into(), s.clone());
        }
        return add_product(text, &Value::Object(p));
    }
    let patch = Patch {
        roles: Some(plan.roster_after.clone()),
        stories: plan.stories_changed.then(|| plan.stories_after.clone()),
    };
    update_product(text, &plan.product, &patch)
}

/// Validates the stage and compares it with the database. Nothing is written.
/// `product` is the registered product, or the product to register.
pub fn plan_apply(
    c: &Connection,
    root: &Path,
    stage_dir: &Path,
    product: &Product,
    register: bool,
) -> super::Result<Planning> {
    let (stage, failures) = read_stage(stage_dir);
    let Some(stage) = stage else {
        return Ok((Err(failures), Vec::new()));
    };
    let registry_file = root.join(FILE);
    let registry_text = std::fs::read_to_string(&registry_file).map_err(|e| {
        crate::error::Error::Unavailable(format!("cannot read {}: {e}", registry_file.display()))
    })?;
    let stories_before = if register {
        None
    } else {
        registry_stories(&registry_text, &product.id)
    };
    let stories_after = match &stage.stories {
        StageStories::Absent => stories_before.clone(),
        StageStories::Off => None,
        StageStories::Block(b) => Some(normal_stories(b)),
    };
    let next = Product {
        roles: stage.roster.clone(),
        ..product.clone()
    };

    let brief: Brief = serde_json::from_value(stage.brief.clone()).unwrap_or_default();
    let files = RoleFiles::at(root);
    let mut warnings = Vec::new();
    describe_roster(
        &next.roles,
        &next.id,
        &files,
        &stage.overlays,
        &mut warnings,
    );
    let mut problems = check_roster(&files, &next, &stage.overlays, Some(&brief));
    if let StageStories::Block(b) = &stage.stories {
        problems.extend(stories_problems(&next, b));
    }
    if !problems.is_empty() {
        return Ok((Err(problems), warnings));
    }

    let before = current_files(c, &product.id, &stage)?;
    let paths: BTreeSet<&String> = before.keys().chain(stage.files.keys()).collect();
    let mut changes = Vec::new();
    for path in paths {
        let (a, b) = (before.get(path), stage.files.get(path));
        if a == b {
            continue;
        }
        let (status, diff) = match (a, b) {
            (None, _) => (ChangeStatus::New, String::new()),
            (_, None) => (ChangeStatus::Removed, String::new()),
            (Some(a), Some(b)) => (ChangeStatus::Changed, unified_diff(a, b)),
        };
        changes.push(FileChange {
            path: path.clone(),
            status,
            diff,
        });
    }
    let plan = ApplyPlan {
        product: product.id.clone(),
        register,
        roster_before: if register {
            Vec::new()
        } else {
            product.roles.clone()
        },
        roster_after: stage.roster.clone(),
        changes,
        stories_changed: stories_changed(&stories_before, &stories_after),
        stories_before,
        stories_after,
    };
    let planned = Planned {
        plan,
        stage,
        product: next,
    };
    if let Err(e) = edit_registry(&registry_text, &planned) {
        return Ok((Err(vec![e]), warnings));
    }
    Ok((Ok(planned), warnings))
}

/// The message of the commit of the registry.
pub fn commit_message(plan: &ApplyPlan) -> String {
    format!(
        "roles({}): {}{}",
        plan.product,
        if plan.register {
            "register and set roster"
        } else {
            "set roster"
        },
        if plan.stories_changed {
            " and stories"
        } else {
            ""
        }
    )
}

/// Writes a plan in one transaction. The registry is the last step: a failed
/// edit returns an error, and the caller rolls the database back.
pub fn write_plan(tx: &Transaction, root: &Path, planned: &Planned) -> super::Result<()> {
    let id = &planned.plan.product;
    for change in &planned.plan.changes {
        if change.path == "brief.json" {
            if change.status == ChangeStatus::Removed {
                tx.execute("delete from briefs where product = ?1", [id])?;
            } else {
                let data = serde_json::to_string(&planned.stage.brief).unwrap_or_default();
                tx.execute(
                    "insert into briefs (product, data) values (?1, ?2) \
                     on conflict (product) do update set data = excluded.data",
                    params![id, data],
                )?;
            }
            continue;
        }
        let agent = &change.path["agents/".len()..change.path.len() - ".md".len()];
        if change.status == ChangeStatus::Removed {
            tx.execute(
                "delete from agent_overlays where product = ?1 and agent = ?2",
                params![id, agent],
            )?;
        } else {
            tx.execute(
                "insert into agent_overlays (product, agent, body) values (?1, ?2, ?3) \
                 on conflict (product, agent) do update set body = excluded.body",
                params![id, agent, planned.stage.overlays[agent]],
            )?;
        }
    }
    let file: PathBuf = root.join(FILE);
    let text = std::fs::read_to_string(&file)
        .map_err(|e| super::Error::Invalid(format!("cannot read {}: {e}", file.display())))?;
    let edited = edit_registry(&text, planned).map_err(super::Error::Invalid)?;
    std::fs::write(&file, edited)
        .map_err(|e| super::Error::Invalid(format!("cannot write {}: {e}", file.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = "{\n  \"products\": [\n    { \"id\": \"p\", \"name\": \"P\", \"paths\": [\"/x\"], \"status\": \"active\", \"roles\": [\"pm\"] }\n  ]\n}\n";
    const BRIEF: &str = r#"{"version":1,"product":{"what":"w","audience":"a","stage":"idea"},"objectives":[{"id":"o1","text":"t","signal":"s","due":"2030-01-01"}],"north_stars":[],"roles":{"pm":"x","dev":"y","dev.api":"z"}}"#;

    struct Env {
        _dir: tempfile::TempDir,
        root: PathBuf,
        stage: PathBuf,
        conn: Connection,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let stage = dir.path().join("stage");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(stage.join("agents")).unwrap();
        std::fs::write(root.join(FILE), REGISTRY).unwrap();
        std::fs::write(stage.join("brief.json"), BRIEF).unwrap();
        std::fs::write(stage.join("roster.json"), r#"["pm","dev","dev.api"]"#).unwrap();
        std::fs::write(
            stage.join("agents/dev.api.md"),
            "---\nextends: dev\n---\nAPI.",
        )
        .unwrap();
        let conn = agentsi_store::db::open_writer(&dir.path().join("t.db")).unwrap();
        Env {
            root: root.clone(),
            stage,
            conn,
            _dir: dir,
        }
    }

    fn product() -> Product {
        Product {
            id: "p".into(),
            name: "P".into(),
            paths: vec!["/x".into()],
            status: "active".into(),
            roles: vec!["pm".into()],
        }
    }

    fn plan(e: &Env) -> Outcome {
        plan_apply(&e.conn, &e.root, &e.stage, &product(), false)
            .unwrap()
            .0
    }

    #[test]
    fn a_valid_stage_plans_two_new_files_and_a_new_roster() {
        let e = env();
        let p = plan(&e).unwrap();
        let paths: Vec<_> = p
            .plan
            .changes
            .iter()
            .map(|c| (c.path.as_str(), c.status))
            .collect();
        assert_eq!(
            paths,
            [
                ("agents/dev.api.md", ChangeStatus::New),
                ("brief.json", ChangeStatus::New)
            ]
        );
        assert_eq!(p.plan.roster_after, ["pm", "dev", "dev.api"]);
        assert!(!p.plan.stories_changed);
    }

    #[test]
    fn a_stage_with_a_missing_file_or_a_bad_roster_fails() {
        let e = env();
        std::fs::remove_file(e.stage.join("brief.json")).unwrap();
        std::fs::write(e.stage.join("roster.json"), r#"[1]"#).unwrap();
        assert_eq!(
            plan(&e).unwrap_err(),
            [
                "Stage has no brief.json.",
                "roster.json must be an array of agent ids."
            ]
        );
    }

    #[test]
    fn a_stage_that_breaks_the_roster_rules_lists_each_failure() {
        let e = env();
        std::fs::write(e.stage.join("roster.json"), r#"["pm","pm","nobody"]"#).unwrap();
        let failures = plan(&e).unwrap_err();
        assert!(failures.contains(&"Agent \"pm\" is repeated in the roster.".to_string()));
        assert!(
            failures
                .iter()
                .any(|f| f.starts_with("Agent \"nobody\": Unknown role"))
        );
    }

    #[test]
    fn writing_a_plan_stores_the_files_and_edits_one_registry_line() {
        let mut e = env();
        let p = plan(&e).unwrap();
        let tx = e.conn.transaction().unwrap();
        write_plan(&tx, &e.root, &p).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            read::overlay(&e.conn, "p", "dev.api").unwrap().unwrap(),
            "---\nextends: dev\n---\nAPI."
        );
        assert_eq!(
            read::brief(&e.conn, "p").unwrap().unwrap()["product"]["stage"],
            "idea"
        );
        let text = std::fs::read_to_string(e.root.join(FILE)).unwrap();
        assert!(text.contains(r#""roles": ["pm", "dev", "dev.api"] }"#));
        // The same stage again changes nothing in the database.
        let again = plan(&e).unwrap();
        assert!(again.plan.changes.is_empty());
    }

    #[test]
    fn a_changed_overlay_has_a_diff_and_a_missing_one_is_removed() {
        let mut e = env();
        let p = plan(&e).unwrap();
        let tx = e.conn.transaction().unwrap();
        write_plan(&tx, &e.root, &p).unwrap();
        tx.commit().unwrap();
        std::fs::write(
            e.stage.join("agents/dev.api.md"),
            "---\nextends: dev\n---\nREST API.",
        )
        .unwrap();
        let p = plan(&e).unwrap();
        assert_eq!(p.plan.changes.len(), 1);
        assert_eq!(p.plan.changes[0].status, ChangeStatus::Changed);
        assert!(p.plan.changes[0].diff.contains("-API."));
        assert!(p.plan.changes[0].diff.contains("+REST API."));
        std::fs::remove_file(e.stage.join("agents/dev.api.md")).unwrap();
        std::fs::write(e.stage.join("roster.json"), r#"["pm","dev"]"#).unwrap();
        std::fs::write(
            e.stage.join("brief.json"),
            BRIEF.replace(r#","dev.api":"z""#, ""),
        )
        .unwrap();
        let p = plan(&e).unwrap();
        assert_eq!(p.plan.changes[0].status, ChangeStatus::Removed);
    }

    #[test]
    fn stories_are_checked_and_a_registered_product_is_added() {
        let e = env();
        std::fs::write(
            e.stage.join("stories.json"),
            r#"{"test":"run","gate":["g"]}"#,
        )
        .unwrap();
        assert_eq!(
            plan(&e).unwrap_err(),
            ["The stories block of p needs a test command with {files}."]
        );
        std::fs::write(
            e.stage.join("stories.json"),
            r#"{"test":"run {files}","gate":["g"],"x":1}"#,
        )
        .unwrap();
        assert_eq!(plan(&e).unwrap_err(), ["stories.json has unknown keys: x."]);
        std::fs::write(
            e.stage.join("stories.json"),
            r#"{"test":"run {files}","gate":["g"],"budget_usd":2}"#,
        )
        .unwrap();
        let p = plan(&e).unwrap();
        assert!(p.plan.stories_changed);
        assert!(commit_message(&p.plan).ends_with("set roster and stories"));

        let new = Product {
            id: "q".into(),
            name: "Q".into(),
            paths: vec!["/q".into()],
            status: "active".into(),
            roles: vec![],
        };
        let p = plan_apply(&e.conn, &e.root, &e.stage, &new, true)
            .unwrap()
            .0
            .unwrap();
        assert_eq!(p.plan.roster_before, Vec::<String>::new());
        let edited = edit_registry(REGISTRY, &p).unwrap();
        assert!(edited.contains(r#"{ "id": "q", "name": "Q", "paths": ["/q"], "status": "active", "roles": ["pm", "dev", "dev.api"], "stories": { "test": "run {files}", "gate": ["g"], "budget_usd": 2 } }"#));
    }
}
