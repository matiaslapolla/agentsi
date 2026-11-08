//! The story session runner with a fake session caller, as the TypeScript
//! tests do it. The tests and gates run in the real bubblewrap sandbox; on a
//! host with no bubblewrap the tests print a line and pass.

use agentsi_agency::registry::Product;
use agentsi_agency::sandbox;
use agentsi_agency::story::claude::{SessionCall, SessionResult};
use agentsi_agency::story::run::{self, RunCtx};
use agentsi_agency::story::{lifecycle, store};
use agentsi_proto::story_api::{Criterion, Story, StoryConfig};
use agentsi_store::Store;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    store: Arc<Store>,
    product: Product,
    config: StoryConfig,
}

fn fixture(id: &str) -> Option<Fixture> {
    if !sandbox::works() {
        eprintln!("skipped: bubblewrap cannot start a sandbox here");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("data");
    let repo = base.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(repo.join("test")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".gitignore"), ".worktrees/\n").unwrap();
    std::fs::write(repo.join("README.md"), "app\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Arc::new(Store::open(&root.join("agentsi.db")).unwrap());
    let product = Product {
        id: id.into(),
        name: "P".into(),
        paths: vec![repo.to_string_lossy().into()],
        status: "active".into(),
        roles: vec!["pm".into(), "dev".into(), "qa".into()],
    };
    let config = StoryConfig {
        repo: repo.to_string_lossy().into(),
        test: "sh {files}".into(),
        gate: vec!["test -f done".into()],
        budget_usd: 1.0,
    };
    let criterion = |t: &str| Criterion {
        given: "g".into(),
        when: "w".into(),
        then: "t".into(),
        test: Some(t.into()),
        checks: "c".into(),
    };
    let story = Story {
        id: "S001".into(),
        kind: "story".into(),
        slug: "make-it-done".into(),
        title: "Make it done".into(),
        as_a: Some("user".into()),
        i_want: "it done".into(),
        so_that: "it is done".into(),
        criteria: vec![criterion("test/a.sh"), criterion("test/b.sh")],
        owner_role: "dev".into(),
        objective: None,
        sources: vec![],
        status: "not_implemented".into(),
        approval: "approved".into(),
        changes_note: None,
        proposed_by: "owner".into(),
        minutes: None,
        session: None,
        results: vec![],
        created_at: "2026-10-01T10:00:00.000Z".into(),
        updated_at: "2026-10-01T10:00:00.000Z".into(),
        archived: None,
    };
    store::save(&store, &product, story).unwrap();
    Some(Fixture {
        _dir: dir,
        root,
        repo,
        store,
        product,
        config,
    })
}

type Script = dyn Fn(&SessionCall) -> SessionResult + Send + Sync;

fn ctx(f: &Fixture, script: Arc<Script>, calls: Arc<Mutex<Vec<String>>>) -> RunCtx {
    let mut ctx = RunCtx::new(
        f.store.clone(),
        f.root.clone(),
        f.product.clone(),
        f.config.clone(),
    );
    ctx.code_graph = false;
    ctx.caller = Arc::new(move |c: &SessionCall, _on: &dyn Fn(Option<u32>)| {
        calls.lock().unwrap().push(c.agent.clone());
        Ok(script(c))
    });
    ctx
}

fn ok(text: &str) -> SessionResult {
    SessionResult {
        text: text.into(),
        cost_usd: 0.01,
        duration_ms: 1000,
        budget_stop: false,
    }
}

/// QA writes the two tests, which pass only once the file `done` exists.
fn qa_writes_tests(c: &SessionCall) {
    let wt = Path::new(&c.cwd);
    std::fs::create_dir_all(wt.join("test")).unwrap();
    for t in ["test/a.sh", "test/b.sh"] {
        std::fs::write(wt.join(t), "test -f done\n").unwrap();
    }
}

fn log(f: &Fixture) -> String {
    f.store
        .read(|c| Ok(store::read_log(c, &f.product.id, "S001").unwrap()))
        .unwrap()
}

#[test]
fn a_session_writes_the_tests_then_the_work_and_ends_ready() {
    let Some(f) = fixture("p0") else { return };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let script: Arc<Script> = Arc::new(|c: &SessionCall| {
        if c.agent == "qa" {
            qa_writes_tests(c);
        } else {
            std::fs::write(Path::new(&c.cwd).join("done"), "").unwrap();
        }
        ok("done")
    });
    let ctx = ctx(&f, script, calls.clone());
    let queued = run::queue(&ctx, "S001").unwrap();
    let s = queued.session.unwrap();
    assert_eq!(
        (s.state.as_str(), s.branch.as_str()),
        ("queued", "story/S001-make-it-done")
    );
    let story = run::execute(&ctx, "S001").unwrap();
    let s = story.session.unwrap();
    assert_eq!(s.state, "ready", "{:?}\n{}", s.failure, log(&f));
    assert!(s.tests_commit.is_some() && s.final_commit.is_some());
    assert_eq!(s.pid, Some(Some(i64::from(std::process::id()))));
    assert_eq!(*calls.lock().unwrap(), vec!["qa", "dev"]);
    let text = log(&f);
    assert!(
        text.contains("Each criterion test fails on tests_commit."),
        "{text}"
    );
    assert!(text.contains("Every check passes."), "{text}");

    let accepted = lifecycle::accept(&f.store, &f.product, "S001", &f.config).unwrap();
    assert_eq!(accepted.result, "merged_unchanged");
    assert!(
        f.repo.join("done").exists(),
        "the merge brings the work to main"
    );
    assert!(!f.repo.join(".worktrees/S001-make-it-done").exists());
}

#[test]
fn a_qa_session_that_writes_outside_the_tests_fails_the_run() {
    let Some(f) = fixture("p1") else { return };
    let script: Arc<Script> = Arc::new(|c: &SessionCall| {
        qa_writes_tests(c);
        std::fs::write(Path::new(&c.cwd).join("README.md"), "changed\n").unwrap();
        ok("")
    });
    let ctx = ctx(&f, script, Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert_eq!(s.state, "failed");
    assert_eq!(
        s.failure.as_deref(),
        Some("QA changed files outside the criterion tests: README.md")
    );
}

#[test]
fn a_session_that_leaves_claude_config_fails_the_run() {
    let Some(f) = fixture("p2") else { return };
    let script: Arc<Script> = Arc::new(|c: &SessionCall| {
        qa_writes_tests(c);
        let dir = Path::new(&c.cwd).join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        ok("")
    });
    let ctx = ctx(&f, script, Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert_eq!(
        s.failure.as_deref(),
        Some("QA left files in .claude/ or .worktrees/: .claude/settings.json")
    );
}

#[test]
fn a_budget_stop_fails_the_run_with_the_cap() {
    let Some(f) = fixture("p3") else { return };
    let script: Arc<Script> = Arc::new(|c: &SessionCall| {
        qa_writes_tests(c);
        SessionResult {
            budget_stop: true,
            ..ok("")
        }
    });
    let ctx = ctx(&f, script, Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert_eq!(s.state, "failed");
    let stop = s.stop.flatten().unwrap();
    assert_eq!((stop.reason.as_str(), stop.cap_usd), ("budget", 1.0));
}

#[test]
fn a_test_that_passes_before_the_work_fails_the_run() {
    let Some(f) = fixture("p4") else { return };
    let script: Arc<Script> = Arc::new(|c: &SessionCall| {
        std::fs::create_dir_all(Path::new(&c.cwd).join("test")).unwrap();
        for t in ["test/a.sh", "test/b.sh"] {
            std::fs::write(Path::new(&c.cwd).join(t), "true\n").unwrap();
        }
        ok("")
    });
    let ctx = ctx(&f, script, Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert!(
        s.failure
            .unwrap()
            .starts_with("The test test/a.sh passes on tests_commit"),
    );
}

#[test]
fn a_rejected_session_stops_and_writes_no_failure() {
    let Some(f) = fixture("p5") else { return };
    let script: Arc<Script> = Arc::new(|c: &SessionCall| {
        qa_writes_tests(c);
        run::cancel("p5", "S001");
        ok("")
    });
    let ctx = ctx(&f, script, Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert_eq!((s.state.as_str(), s.failure), ("tests", None));
    assert!(log(&f).contains("was rejected while it ran"));
    let rejected = lifecycle::reject(&f.store, &f.product, "S001", false, &f.config).unwrap();
    assert!(rejected.session.is_none());
    assert_eq!(rejected.results[0].result, "discarded");
    assert!(!f.repo.join(".worktrees/S001-make-it-done").exists());
}

/// A QA that writes the tests, and an owner that writes `done`.
fn good_session(c: &SessionCall) -> SessionResult {
    if c.agent == "qa" {
        qa_writes_tests(c);
    } else {
        std::fs::write(Path::new(&c.cwd).join("done"), "").unwrap();
    }
    ok("done")
}

#[test]
fn a_gate_that_leaves_claude_config_fails_the_run() {
    let Some(mut f) = fixture("p6") else { return };
    f.config.gate = vec!["mkdir -p .claude && touch .claude/evil.json".into()];
    let ctx = ctx(&f, Arc::new(good_session), Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert_eq!(s.state, "failed");
    assert_eq!(
        s.failure.as_deref(),
        Some("A test or gate command left files in .claude/ or .worktrees/: .claude/evil.json")
    );
}

#[test]
fn a_repo_hook_that_a_session_wrote_does_not_run_on_the_host() {
    let Some(f) = fixture("p7") else { return };
    git(&f.repo, &["config", "core.hooksPath", ".hooks"]);
    let marker = f.root.join("hook-ran");
    let hook = format!("#!/bin/sh\ntouch {}\n", marker.display());
    let script: Arc<Script> = Arc::new(move |c: &SessionCall| {
        use std::os::unix::fs::PermissionsExt;
        if c.agent != "qa" {
            let dir = Path::new(&c.cwd).join(".hooks");
            std::fs::create_dir_all(&dir).unwrap();
            for name in ["pre-commit", "post-merge"] {
                let file = dir.join(name);
                std::fs::write(&file, &hook).unwrap();
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        good_session(c)
    });
    let ctx = ctx(&f, script, Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert_eq!(s.state, "ready", "{:?}", s.failure);
    lifecycle::accept(&f.store, &f.product, "S001", &f.config).unwrap();
    assert!(
        f.repo.join(".hooks/post-merge").exists(),
        "the merge brought the hook"
    );
    assert!(!marker.exists(), "no hook ran on the host");
}

#[test]
fn a_test_path_with_an_accent_is_a_criterion_test() {
    let Some(f) = fixture("p8") else { return };
    let mut story = f
        .store
        .read(|c| Ok(store::load(c, "p8", "S001").unwrap()))
        .unwrap();
    story.criteria[0].test = Some("test/año.sh".into());
    store::save(&f.store, &f.product, story).unwrap();
    let script: Arc<Script> = Arc::new(|c: &SessionCall| {
        if c.agent == "qa" {
            let wt = Path::new(&c.cwd);
            std::fs::create_dir_all(wt.join("test")).unwrap();
            for t in ["test/año.sh", "test/b.sh"] {
                std::fs::write(wt.join(t), "test -f done\n").unwrap();
            }
        } else {
            std::fs::write(Path::new(&c.cwd).join("done"), "").unwrap();
        }
        ok("done")
    });
    let ctx = ctx(&f, script, Arc::default());
    run::queue(&ctx, "S001").unwrap();
    let s = run::execute(&ctx, "S001").unwrap().session.unwrap();
    assert_eq!(s.state, "ready", "{:?}", s.failure);
}

#[test]
fn reject_force_never_kills_a_process_that_is_not_the_session() {
    let Some(f) = fixture("p9") else { return };
    let ctx = ctx(&f, Arc::new(good_session), Arc::default());
    let mut story = run::queue(&ctx, "S001").unwrap();
    let mut other = Command::new("sleep").arg("30").spawn().unwrap();
    let session = story.session.as_mut().unwrap();
    session.state = "work".into();
    session.pid = Some(Some(i64::from(other.id())));
    store::save(&f.store, &f.product, story).unwrap();
    let rejected = lifecycle::reject(&f.store, &f.product, "S001", true, &f.config).unwrap();
    assert!(rejected.session.is_none());
    assert!(
        other.try_wait().unwrap().is_none(),
        "the reused pid lives on"
    );
    other.kill().unwrap();
    let _ = other.wait();
}

#[test]
fn a_story_that_cannot_be_saved_stops_reject_and_accept_before_git() {
    let Some(f) = fixture("p10") else { return };
    let ctx = ctx(&f, Arc::new(good_session), Arc::default());
    run::queue(&ctx, "S001").unwrap();
    run::execute(&ctx, "S001").unwrap();
    let without_owner = Product {
        roles: vec!["pm".into(), "qa".into()],
        ..f.product.clone()
    };
    let wt = f.repo.join(".worktrees/S001-make-it-done");
    assert!(lifecycle::reject(&f.store, &without_owner, "S001", false, &f.config).is_err());
    assert!(lifecycle::accept(&f.store, &without_owner, "S001", &f.config).is_err());
    assert!(wt.exists(), "the work stays");
    assert!(!f.repo.join("done").exists(), "nothing was merged");
}
