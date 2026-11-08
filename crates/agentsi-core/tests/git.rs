//! The `git` and `git.config` methods on a real core: the screen methods on a temporary repo,
//! the refusals, the events, and the git config of a product in a git screen commit and in a
//! new pane of each product. The core runs with a throwaway data root and config folder, and
//! with no global or system git config.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir, wait_until};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

struct Setup {
    root: PathBuf,
    config: PathBuf,
    home: PathBuf,
    alpha: PathBuf,
    beta: PathBuf,
}

fn setup(dir: &TempDir) -> Setup {
    let base = &dir.0;
    let (root, config, home) = (base.join("root"), base.join("config"), base.join("home"));
    for d in [&root, &config, &home] {
        std::fs::create_dir_all(d).unwrap();
    }
    let mut repos = Vec::new();
    for name in ["alpha", "beta"] {
        let repo = base.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        repos.push(repo);
    }
    let registry = json!({"products": [
        {"id": "alpha", "name": "Alpha", "paths": [repos[0]], "status": "active", "roles": ["pm"]},
        {"id": "beta", "name": "Beta", "paths": [repos[1]], "status": "active", "roles": ["pm"]}]});
    std::fs::write(root.join("agentsi.config.json"), registry.to_string()).unwrap();
    Setup {
        root,
        config,
        home,
        alpha: repos[0].clone(),
        beta: repos[1].clone(),
    }
}

fn start(dir: &TempDir, s: &Setup) -> CoreProc {
    CoreProc::start_with(
        dir,
        &[
            ("AGENTSI_HOME", &s.root),
            ("XDG_CONFIG_HOME", &s.config),
            ("HOME", &s.home),
            ("GIT_CONFIG_GLOBAL", Path::new("/dev/null")),
            ("GIT_CONFIG_SYSTEM", Path::new("/dev/null")),
        ],
    )
}

#[test]
fn the_screen_methods_stage_commit_and_log_and_send_an_event() {
    let dir = TempDir::new();
    let s = setup(&dir);
    git(&s.alpha, &["config", "user.name", "Test"]);
    git(&s.alpha, &["config", "user.email", "test@example.invalid"]);
    let core = start(&dir, &s);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    std::fs::write(s.alpha.join("a b.txt"), "one\n").unwrap();
    let st = c.ok("git.status", json!({"product": "alpha"}));
    assert_eq!(st["branch"], "main");
    assert_eq!(st["files"][0]["path"], "a b.txt");
    assert_eq!(st["files"][0]["unstaged"], "?");
    assert_eq!(st["files"][0]["staged"], Value::Null);

    c.ok(
        "git.stage",
        json!({"product": "alpha", "paths": ["a b.txt"]}),
    );
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "alpha", "kind": "git"})
    );
    let st = c.ok("git.status", json!({"product": "alpha", "repo": s.alpha}));
    assert_eq!(st["files"][0]["staged"], "A");
    let diff = c.ok(
        "git.diff",
        json!({"product": "alpha", "path": "a b.txt", "staged": true}),
    );
    assert!(diff["patch"].as_str().unwrap().contains("+one"));
    assert_eq!(diff["truncated"], false);

    let done = c.ok(
        "git.commit",
        json!({"product": "alpha", "message": "feat: a"}),
    );
    assert_eq!(done["committed"], true);
    assert_eq!(done["commit"], git(&s.alpha, &["rev-parse", "HEAD"]));
    assert_eq!(c.next_event("data.changed")["kind"], "git");
    let log = c.ok("git.log", json!({"product": "alpha", "limit": 50}));
    assert_eq!(log["commits"][0]["subject"], "feat: a");

    c.ok("git.unstage", json!({"product": "alpha", "paths": []}));
}

#[test]
fn a_refused_path_a_foreign_repo_and_an_unknown_product_fail_with_a_kind() {
    let dir = TempDir::new();
    let s = setup(&dir);
    let core = start(&dir, &s);
    let mut c = Framed::connect(&core.socket.path);
    let r = c.call(
        "git.stage",
        json!({"product": "alpha", "paths": ["../beta/x.txt"]}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call(
        "git.diff",
        json!({"product": "alpha", "path": "/etc/hostname", "staged": false}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    // The repo of another product is not a repo of this product.
    let r = c.call("git.status", json!({"product": "alpha", "repo": s.beta}));
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call("git.status", json!({"product": "nobody"}));
    assert_eq!(kind(&r), "not_found", "{r}");
    let r = c.call("git.commit", json!({"product": "alpha", "message": "   "}));
    assert_eq!(kind(&r), "invalid", "{r}");
}

/// The email that `git config user.email` prints in a new pane of the product.
fn pane_email(c: &mut Framed, repo: &Path, product: &str, out: &Path) -> String {
    let _ = std::fs::remove_file(out);
    c.ok(
        "terminal.create",
        json!({
            "cwd": repo,
            "argv": ["sh", "-c", format!("git config user.email > {}", out.display())],
            "product": product,
        }),
    );
    // The shell makes the file empty first, and git writes the line after.
    wait_until("the pane wrote the email", || {
        std::fs::read_to_string(out).is_ok_and(|t| t.ends_with('\n'))
    });
    std::fs::read_to_string(out).unwrap().trim().to_string()
}

#[test]
fn the_git_config_of_a_product_reaches_a_screen_commit_and_a_new_pane() {
    let dir = TempDir::new();
    let s = setup(&dir);
    let core = start(&dir, &s);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let empty = c.ok("git.config.get", json!({"product": "beta"}));
    assert_eq!(empty["shared"], json!([]));
    assert!(
        empty["path"]
            .as_str()
            .unwrap()
            .ends_with("agentsi/git.toml")
    );

    let r = c.ok(
        "git.config.set",
        json!({"entries": [
            {"key": "user.name", "value": "Shared Name"},
            {"key": "user.email", "value": "shared@example.invalid"}]}),
    );
    assert_eq!(r["shared"][1]["value"], "shared@example.invalid");
    // The shared table changes both products.
    assert_eq!(c.next_event("data.changed")["kind"], "git");
    assert_eq!(c.next_event("data.changed")["kind"], "git");
    let r = c.ok(
        "git.config.set",
        json!({"product": "beta", "entries": [{"key": "user.email", "value": "beta@example.invalid"}]}),
    );
    assert_eq!(r["product"][0]["key"], "user.email");
    let src = |r: &Value, key: &str| -> (String, String) {
        let e = r["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == key)
            .unwrap();
        (
            e["value"].as_str().unwrap().into(),
            e["source"].as_str().unwrap().into(),
        )
    };
    assert_eq!(
        src(&r, "user.email"),
        ("beta@example.invalid".into(), "product".into())
    );
    assert_eq!(
        src(&r, "user.name"),
        ("Shared Name".into(), "shared".into())
    );
    let alpha = c.ok("git.config.get", json!({"product": "alpha"}));
    assert_eq!(
        src(&alpha, "user.email"),
        ("shared@example.invalid".into(), "shared".into())
    );
    assert_eq!(alpha["product"], json!([]));

    // A git screen commit of each product.
    for (repo, product, want) in [
        (&s.alpha, "alpha", "shared@example.invalid"),
        (&s.beta, "beta", "beta@example.invalid"),
    ] {
        std::fs::write(repo.join("f.txt"), product).unwrap();
        c.ok("git.stage", json!({"product": product, "paths": ["f.txt"]}));
        let done = c.ok(
            "git.commit",
            json!({"product": product, "message": "feat: f"}),
        );
        assert_eq!(done["committed"], true, "{done}");
        assert_eq!(git(repo, &["log", "-1", "--format=%ae"]), want);
        assert_eq!(git(repo, &["log", "-1", "--format=%an"]), "Shared Name");
    }
    // A new pane of each product.
    let out = dir.0.join("email.txt");
    assert_eq!(
        pane_email(&mut c, &s.alpha, "alpha", &out),
        "shared@example.invalid"
    );
    assert_eq!(
        pane_email(&mut c, &s.beta, "beta", &out),
        "beta@example.invalid"
    );
    // With no product named, the folder decides.
    c.ok(
        "terminal.create",
        json!({"cwd": s.beta, "argv": ["sh", "-c", format!("git config user.email > {}", out.display())]}),
    );
    wait_until("the pane without a product wrote", || {
        std::fs::read_to_string(&out).is_ok_and(|t| t.trim() == "beta@example.invalid")
    });

    // The file is the only place that holds the values.
    for repo in [&s.alpha, &s.beta] {
        let local = std::fs::read_to_string(repo.join(".git/config")).unwrap();
        assert!(!local.contains("example.invalid"), "{local}");
    }
    assert!(!s.home.join(".gitconfig").exists());
}

#[test]
fn the_config_set_keeps_the_comments_and_refuses_a_bad_key() {
    let dir = TempDir::new();
    let s = setup(&dir);
    std::fs::create_dir_all(s.config.join("agentsi")).unwrap();
    let file = s.config.join("agentsi").join("git.toml");
    std::fs::write(
        &file,
        "# my notes\n[shared]\n\"user.name\" = \"Ada\"\n\n# beta is for work\n[product.beta]\n\"user.email\" = \"w@x.example\"\n",
    )
    .unwrap();
    let core = start(&dir, &s);
    let mut c = Framed::connect(&core.socket.path);
    c.ok(
        "git.config.set",
        json!({"product": "alpha", "entries": [{"key": "user.email", "value": "a@x.example"}]}),
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("# my notes") && text.contains("# beta is for work"),
        "{text}"
    );
    assert!(
        text.contains("w@x.example") && text.contains("a@x.example"),
        "{text}"
    );
    for bad in ["name", "user name.x", ""] {
        let r = c.call(
            "git.config.set",
            json!({"entries": [{"key": bad, "value": "v"}]}),
        );
        assert_eq!(kind(&r), "invalid", "{bad}: {r}");
    }
    let r = c.call(
        "git.config.set",
        json!({"entries": [{"key": "user.name", "value": ""}]}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call("git.config.get", json!({"product": "nobody"}));
    assert_eq!(kind(&r), "not_found", "{r}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        text,
        "a refused write changes nothing"
    );
    // A broken file is unavailable, and the core keeps working.
    std::fs::write(&file, "[shared\n").unwrap();
    let r = c.call("git.config.get", json!({}));
    assert_eq!(kind(&r), "unavailable", "{r}");
}
