//! The git config of a product reaches each git call of the agency, each story session and
//! each meeting process of that product. The test process has a data root with two products
//! and a `git.toml` with a shared `user.email` and one override. No test writes a `.git/config`.

use agentsi_agency::git_env;
use agentsi_agency::gitpanel;
use agentsi_agency::meeting::claude::{Cancel, claude as meeting_claude};
use agentsi_agency::roles::commit::commit_files as role_commit;
use agentsi_agency::story::claude::{Call, run_claude};
use agentsi_agency::story::git::{commit_files, git, git_out};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

struct World {
    _dir: tempfile::TempDir,
    alpha: PathBuf,
    beta: PathBuf,
    outside: PathBuf,
    path: String,
}

fn raw(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// A repo with one commit and NO user.name or user.email of its own.
fn repo(base: &Path, name: &str) -> PathBuf {
    let path = base.join(name);
    std::fs::create_dir_all(&path).unwrap();
    raw(&path, &["init", "-q", "-b", "main"]);
    raw(&path, &["config", "commit.gpgsign", "false"]);
    std::fs::write(path.join(".gitignore"), ".worktrees/\n").unwrap();
    path
}

fn world() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let (home, config) = (base.join("home"), base.join("config"));
        std::fs::create_dir_all(home.join("x")).unwrap();
        std::fs::create_dir_all(config.join("agentsi")).unwrap();
        std::fs::create_dir_all(base.join("bin")).unwrap();
        // SAFETY: this is the only test binary function that sets the environment, and the
        // `OnceLock` runs it once, before any child process starts.
        unsafe {
            std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
            std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
            std::env::set_var("XDG_CONFIG_HOME", &config);
            std::env::set_var("AGENTSI_HOME", &home);
            std::env::remove_var("GIT_CONFIG_COUNT");
        }
        let (alpha, beta, outside) = (
            repo(&base, "alpha"),
            repo(&base, "beta"),
            repo(&base, "outside"),
        );
        let registry = serde_json::json!({"products": [
            {"id": "alpha", "name": "Alpha", "paths": [alpha], "status": "active", "roles": ["pm"]},
            {"id": "beta", "name": "Beta", "paths": [beta], "status": "active", "roles": ["pm"]}]});
        std::fs::write(home.join("agentsi.config.json"), registry.to_string()).unwrap();
        let file = git_env::path().unwrap();
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        git_env::set_table(
            &file,
            None,
            &pairs(&[
                ("user.name", "Shared Name"),
                ("user.email", "shared@example.invalid"),
                ("core.abbrev", "9"),
            ]),
        )
        .unwrap();
        git_env::set_table(
            &file,
            Some("beta"),
            &pairs(&[("user.email", "beta@example.invalid"), ("core.abbrev", "")]),
        )
        .unwrap();
        // A fake claude that tells which email git gives in its folder.
        let bin = base.join("bin").join("claude");
        std::fs::write(
            &bin,
            "#!/bin/sh\ncat >/dev/null\nprintf '{\"type\":\"result\",\"result\":\"%s\"}\\n' \"$(git config user.email)\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", base.join("bin").display());
        World {
            _dir: dir,
            alpha,
            beta,
            outside,
            path,
        }
    })
}

fn email(repo: &Path) -> String {
    git_out(repo, &["log", "-1", "--format=%ae"]).unwrap()
}

fn touch(repo: &Path, name: &str) {
    std::fs::write(repo.join(name), name).unwrap();
}

#[test]
fn a_story_commit_a_role_commit_and_a_screen_commit_use_the_config_of_their_product() {
    let w = world();
    // Story commit: the runner of a story.
    for (repo, want) in [
        (&w.alpha, "shared@example.invalid"),
        (&w.beta, "beta@example.invalid"),
    ] {
        touch(repo, "story.txt");
        assert!(commit_files(repo, &["story.txt"], "feat: story").unwrap());
        assert_eq!(email(repo), want, "story commit in {}", repo.display());
        assert_eq!(
            git_out(repo, &["log", "-1", "--format=%an"]).unwrap(),
            "Shared Name"
        );
        // Role commit: the memory of a role.
        touch(repo, "role.txt");
        assert!(role_commit(repo, &["role.txt"], "docs: role").committed);
        assert_eq!(email(repo), want, "role commit in {}", repo.display());
        // Screen commit.
        touch(repo, "screen.txt");
        gitpanel::stage(repo, &["screen.txt".into()]).unwrap();
        assert!(gitpanel::commit(repo, "feat: screen").unwrap().committed);
        assert_eq!(email(repo), want, "screen commit in {}", repo.display());
        // The values came by the environment: the config of the repo holds none of them.
        let local = std::fs::read_to_string(repo.join(".git/config")).unwrap();
        assert!(
            !local.contains("example.invalid") && !local.contains("Shared Name"),
            "{local}"
        );
    }
}

#[test]
fn a_folder_of_no_product_gets_no_variables_and_a_story_worktree_gets_its_product() {
    let w = world();
    assert!(git_env::env_for_cwd(&w.outside).is_empty());
    assert!(git_env::env_for_cwd(Path::new("/")).is_empty());
    let inside = w.alpha.join(".worktrees").join("story-x");
    assert_eq!(
        git_env::env_for_cwd(&inside),
        git_env::env_for_cwd(&w.alpha)
    );
    assert!(!git_env::env_for_cwd(&inside).is_empty());
}

#[test]
fn an_empty_override_removes_the_shared_key_for_that_product_only() {
    let w = world();
    assert_eq!(git_out(&w.alpha, &["config", "core.abbrev"]).unwrap(), "9");
    assert!(!git(&w.beta, &["config", "core.abbrev"]).ok);
    assert!(git_env::env_for_cwd(&w.beta).iter().all(|(_, v)| v != "9"));
}

#[test]
fn a_story_session_and_a_meeting_process_run_with_the_config_of_their_product() {
    let w = world();
    for (repo, want) in [
        (&w.alpha, "shared@example.invalid"),
        (&w.beta, "beta@example.invalid"),
    ] {
        let raw = run_claude(
            &["-p".to_string()],
            repo,
            "prompt",
            Duration::from_secs(20),
            &[("PATH", &w.path)],
            &|_| {},
        )
        .unwrap();
        assert_eq!(raw["result"], want, "story session in {}", repo.display());
    }
    // The meeting process takes the PATH of this process, so the fake program is a full path.
    for (repo, want) in [
        (&w.alpha, "shared@example.invalid"),
        (&w.beta, "beta@example.invalid"),
    ] {
        let program = w.path.split(':').next().unwrap().to_string() + "/claude";
        let call = Call {
            prompt: "p".into(),
            model: "haiku".into(),
            system: "s".into(),
            cwd: repo.to_string_lossy().into_owned(),
            timeout: Duration::from_secs(20),
            ..Call::default()
        };
        let r = meeting_claude(&program, &call, &|_| {}, &Cancel::default()).unwrap();
        assert_eq!(
            r.output,
            serde_json::json!(want),
            "meeting in {}",
            repo.display()
        );
    }
}
