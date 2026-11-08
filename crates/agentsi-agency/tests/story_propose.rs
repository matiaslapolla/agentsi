//! `story new`, `story suggest` and `story changes` with a fake PM, as the
//! TypeScript tests do it: one repair round, and nothing written after a
//! second failure.

use agentsi_agency::registry::Product;
use agentsi_agency::story::claude::Call;
use agentsi_agency::story::propose::{self, ProposeCtx};
use agentsi_agency::story::store;
use agentsi_proto::agency_api::Owner;
use agentsi_proto::story_api::{StoryConfig, StoryUsage};
use agentsi_store::Store;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    store: Arc<Store>,
    product: Product,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("data");
    let repo = base.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/app.ts"), "export {}\n").unwrap();
    let store = Arc::new(Store::open(&root.join("agentsi.db")).unwrap());
    let product = Product {
        id: "p".into(),
        name: "P".into(),
        paths: vec![repo.to_string_lossy().into()],
        status: "active".into(),
        roles: vec!["pm".into(), "dev".into(), "qa".into()],
    };
    Fixture {
        _dir: dir,
        root,
        repo,
        store,
        product,
    }
}

fn content(title: &str, path: &str) -> Value {
    let criterion =
        json!({"given": "g", "when": "w", "then": "t", "test": "test/a.test.ts", "checks": "c"});
    json!({"type": "story", "title": title, "as_a": "entrenador", "i_want": "ver", "so_that": "decida",
        "criteria": [criterion, criterion], "owner_role": "dev", "objective": null,
        "sources": [{"type": "path", "ref": path}]})
}

fn usage(role: &str) -> StoryUsage {
    StoryUsage {
        role: role.into(),
        model: "opus".into(),
        input_tokens: 1,
        output_tokens: 1,
        cache_read: 0,
        cache_write: 0,
        cost_usd: 0.25,
        duration_ms: 10,
    }
}

/// A PM that answers with `outputs` in order, and keeps each prompt.
fn ctx(f: &Fixture, outputs: Vec<Value>, prompts: Arc<Mutex<Vec<String>>>) -> ProposeCtx {
    let config = StoryConfig {
        repo: f.repo.to_string_lossy().into(),
        test: "node --test {files}".into(),
        gate: vec!["true".into()],
        budget_usd: 5.0,
    };
    let owner = Owner {
        name: "Ada".into(),
        language: "Spanish".into(),
    };
    let mut ctx = ProposeCtx::new(
        f.store.clone(),
        f.root.clone(),
        f.product.clone(),
        config,
        owner,
    );
    ctx.snapshot = Some("# Snapshot of p\n\nNothing yet.".into());
    let queue = Mutex::new(outputs);
    ctx.caller = Arc::new(move |c: &Call, role: &str| {
        prompts.lock().unwrap().push(c.prompt.clone());
        Ok((queue.lock().unwrap().remove(0), usage(role)))
    });
    ctx
}

fn stories(f: &Fixture) -> Vec<String> {
    f.store
        .read(|c| Ok(store::list(c, "p", true).unwrap()))
        .unwrap()
        .into_iter()
        .map(|s| format!("{} {} {}", s.id, s.approval, s.title))
        .collect()
}

#[test]
fn a_new_story_is_saved_pending_with_the_next_id() {
    let f = fixture();
    let prompts = Arc::default();
    let out = json!({"stories": [content("Ver el resultado", "src/app.ts")]});
    let r = propose::propose_story(
        &ctx(&f, vec![out], prompts),
        "quiero ver el resultado",
        None,
        None,
    )
    .unwrap();
    assert_eq!(r.stories[0].slug, "ver-el-resultado");
    assert_eq!(r.stories[0].proposed_by, "owner");
    assert_eq!(r.total_cost_usd, 0.25);
    assert_eq!(stories(&f), vec!["S001 pending Ver el resultado"]);
}

#[test]
fn a_source_that_does_not_exist_gets_one_repair_with_the_failure() {
    let f = fixture();
    let prompts: Arc<Mutex<Vec<String>>> = Arc::default();
    let bad = json!({"stories": [content("Ver", "src/missing.ts")]});
    let good = json!({"stories": [content("Ver", "src/app.ts")]});
    let r = propose::propose_story(&ctx(&f, vec![bad, good], prompts.clone()), "x", None, None)
        .unwrap();
    let roles: Vec<&str> = r.usage.iter().map(|u| u.role.as_str()).collect();
    assert_eq!(roles, vec!["pm", "pm-repair"]);
    let repair = &prompts.lock().unwrap()[1];
    assert!(
        repair.contains("- The path source \"src/missing.ts\" does not exist in the repo."),
        "{repair}"
    );
    assert_eq!(stories(&f), vec!["S001 pending Ver"]);
}

#[test]
fn a_second_bad_output_writes_nothing() {
    let f = fixture();
    let bad = json!({"stories": []});
    let err = propose::propose_story(
        &ctx(&f, vec![bad.clone(), bad], Arc::default()),
        "x",
        Some("task"),
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.starts_with("The PM output is invalid after one repair"),
        "{err}"
    );
    assert!(err.contains("\"stories\" is empty; return exactly 1 story."));
    assert!(stories(&f).is_empty());
}

#[test]
fn changes_rewrite_the_story_and_keep_its_id_and_slug() {
    let f = fixture();
    let first = json!({"stories": [content("Ver el resultado", "src/app.ts")]});
    propose::propose_story(&ctx(&f, vec![first], Arc::default()), "x", None, None).unwrap();
    let prompts: Arc<Mutex<Vec<String>>> = Arc::default();
    let rewritten = json!({"stories": [content("Ver el marcador", "src/app.ts")]});
    let r = propose::revise_story(
        &ctx(&f, vec![rewritten], prompts.clone()),
        "S001",
        "Usar marcador",
    )
    .unwrap();
    let s = &r.stories[0];
    assert_eq!(
        (s.id.as_str(), s.slug.as_str(), s.title.as_str()),
        ("S001", "ver-el-resultado", "Ver el marcador")
    );
    assert_eq!(
        (s.approval.as_str(), s.changes_note.as_deref()),
        ("pending", Some("Usar marcador"))
    );
    assert!(prompts.lock().unwrap()[0].contains("Note: Usar marcador"));
}

#[test]
fn suggest_may_propose_nothing_and_an_unknown_minutes_is_refused() {
    let f = fixture();
    let r =
        propose::suggest_stories(&ctx(&f, vec![json!({"stories": []})], Arc::default())).unwrap();
    assert!(r.stories.is_empty());
    let err = propose::propose_story(&ctx(&f, vec![], Arc::default()), "x", None, Some("m-1"))
        .unwrap_err();
    assert_eq!(err.to_string(), "Unknown minutes \"m-1\" in p");
}
