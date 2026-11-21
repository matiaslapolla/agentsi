//! The login shell PATH on a real core: `agentsi-core serve` reads `$SHELL -l -i` once at
//! start. The shells and `claude` are fakes. Spec: `docs/specs/agentsi-path.md`.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A shell that prints noise, then runs the command of the core with `extra` on its `PATH`.
fn noisy_shell(dir: &Path, extra: &Path) -> PathBuf {
    let shell = dir.join("noisy-shell");
    script(
        &shell,
        &format!(
            "echo 'Last login: today'\nPATH=\"$PATH:{}\"\nexport PATH\neval \"$4\"",
            extra.display()
        ),
    );
    shell
}

/// The `PATH` of the started programs: the fake tools, then the system for `sh` and `git`.
fn base_path(bin: &Path) -> String {
    format!("{}:/usr/bin:/bin", bin.display())
}

/// A core on the data root `root` that reads the login shell `shell`.
fn core_with_shell(dir: &TempDir, shell: &Path, path: &str, root: &Path) -> CoreProc {
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(root).unwrap();
    let env: [(&str, &Path); 5] = [
        ("AGENTSI_NO_SHELL_PATH", Path::new("0")),
        ("SHELL", shell),
        ("PATH", Path::new(path)),
        ("HOME", &home),
        ("AGENTSI_HOME", root),
    ];
    // The read of a slow shell holds the start for its whole limit.
    CoreProc::start_within(dir, &env, Duration::from_secs(10))
}

fn core_path(core: &CoreProc) -> Value {
    Framed::connect(&core.socket.path).ok("core.status", json!({}))["path"].clone()
}

#[test]
fn the_core_adds_the_folders_of_the_login_shell_after_its_own() {
    let dir = TempDir::new();
    let (bin, extra) = (dir.0.join("bin"), dir.0.join("extra"));
    std::fs::create_dir_all(&bin).unwrap();
    let path = base_path(&bin);
    let shell = noisy_shell(&dir.0, &extra);
    let core = core_with_shell(&dir, &shell, &path, &dir.0.join("root"));
    assert_eq!(
        core_path(&core),
        json!({"source": "merged", "value": format!("{path}:{}", extra.display())})
    );
}

#[test]
fn a_slow_shell_keeps_the_process_path() {
    let dir = TempDir::new();
    let bin = dir.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let shell = dir.0.join("slow-shell");
    script(&shell, "sleep 30");
    let path = base_path(&bin);
    let start = Instant::now();
    let core = core_with_shell(&dir, &shell, &path, &dir.0.join("root"));
    assert!(
        start.elapsed() < Duration::from_secs(6),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        core_path(&core),
        json!({"source": "shell_read_failed", "value": path})
    );
    assert!(
        core.stderr().contains("gave no PATH in 5s"),
        "{}",
        core.stderr()
    );
}

/// A product with one role, as `meeting.ask` needs it.
fn product(dir: &TempDir) -> PathBuf {
    let (root, repo) = (dir.0.join("root"), dir.0.join("repo"));
    std::fs::create_dir_all(root.join("plugin/agents")).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        root.join("plugin/agents/dev.md"),
        "---\nmodel: haiku\n---\nYou are dev.\n",
    )
    .unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    ] {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
    std::fs::write(
        root.join("agentsi.config.json"),
        json!({
            "owner": {"name": "Ada", "language": "English"},
            "products": [{"id": "p", "name": "P", "paths": [repo], "status": "active", "roles": ["dev"]}],
        })
        .to_string(),
    )
    .unwrap();
    root
}

fn ask(core: &CoreProc) -> Value {
    Framed::connect(&core.socket.path).call(
        "meeting.ask",
        json!({"product": "p", "role": "dev", "question": "Is it ready?"}),
    )
}

#[test]
fn a_meeting_starts_the_claude_that_only_the_login_shell_finds() {
    let dir = TempDir::new();
    let root = product(&dir);
    let (bin, extra) = (dir.0.join("bin"), dir.0.join("extra"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&extra).unwrap();
    let started = dir.0.join("claude-started");
    script(
        &extra.join("claude"),
        &format!("touch {}\nexit 1", started.display()),
    );
    let path = base_path(&bin);

    // With no shell read, the core does not find it and says where it looked.
    let off = TempDir::new();
    let blind = CoreProc::start_with(
        &off,
        &[
            ("AGENTSI_HOME", root.as_path()),
            ("PATH", Path::new(&path)),
            ("HOME", &dir.0),
        ],
    );
    let reply = ask(&blind);
    assert_eq!(
        reply["error"]["message"],
        format!("cannot find `claude` on the PATH of the core: {path}"),
        "{reply}"
    );
    assert!(!started.exists());

    let shell = noisy_shell(&dir.0, &extra);
    let core = core_with_shell(&dir, &shell, &path, &root);
    let reply = ask(&core);
    assert!(started.exists(), "{reply}");
}
