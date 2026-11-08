//! The escape proof of the runner sandbox: inside bubblewrap, with the mask of
//! a story, a write outside the worktree, a read of a masked folder or file,
//! a connect to the docker socket and a network call each fail. A write in
//! the worktree works. The mask is the real one: the home dir of the owner of
//! the process. The test prints only counts and exit codes, no file text.
//! On a host with no bubblewrap it prints a line and passes.

use agentsi_agency::sandbox::{self, Mask, sandboxed, story_mask};
use std::path::Path;
use std::time::Duration;

fn run(wt: &Path, mask: &Mask, command: &str) -> (bool, String) {
    let r = sandboxed(wt, command, Duration::from_secs(60), mask, &|_| {});
    eprintln!(
        "$ {command}\n  ok={} {}",
        r.ok,
        format!("{}{}", r.stdout, r.stderr).trim()
    );
    (r.ok, format!("{}{}", r.stdout, r.stderr))
}

#[test]
fn a_command_in_the_sandbox_cannot_write_outside_read_secrets_or_reach_the_network() {
    if !sandbox::works() {
        eprintln!("skipped: bubblewrap cannot start a sandbox here");
        return;
    }
    // Not under /tmp: bubblewrap puts a private /tmp over it, which would hide the files for another reason.
    let exe = std::env::current_exe().unwrap();
    let dir = tempfile::tempdir_in(exe.parent().unwrap()).unwrap();
    let base = dir.path().canonicalize().unwrap();
    let repo = base.join("repo");
    let root = base.join("data");
    let wt = repo.join(".worktrees/S001-x");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), "{}").unwrap();
    std::fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
    std::fs::write(wt.join(".git"), "gitdir: /the/real/link\n").unwrap();
    let home = sandbox::home();
    let mask = story_mask(&repo, &home, &root);

    // A write in the worktree works.
    assert!(run(&wt, &mask, "echo ok > inside.txt && cat inside.txt").0);
    assert!(wt.join("inside.txt").exists());

    // A write outside the worktree fails: the repo, the data root, the home dir.
    let outside = repo.join("escaped.txt");
    assert!(!run(&wt, &mask, &format!("echo x > {}", outside.display())).0);
    assert!(!outside.exists());
    assert!(!run(&wt, &mask, &format!("echo x > {}/x", root.display())).0);
    let home_file = home.join(".agentsi-u23-escape-probe");
    assert!(!run(&wt, &mask, &format!("echo x > {}", home_file.display())).0);
    assert!(!home_file.exists());

    // The git link of the worktree cannot change: the runner runs git on the host.
    assert!(!run(&wt, &mask, "echo 'gitdir: /tmp/evil' > .git").0);
    assert!(!run(&wt, &mask, "mv .git moved").0);
    assert_eq!(
        std::fs::read_to_string(wt.join(".git")).unwrap(),
        "gitdir: /the/real/link\n"
    );

    // `~/.config` shows only the identity of git.
    let (ok, out) = run(
        &wt,
        &mask,
        &format!("ls -A {}", home.join(".config").display()),
    );
    assert!(ok);
    assert!(out.trim().is_empty() || out.trim() == "git", "{out}");

    // A masked folder reads as empty, and a masked file cannot be read.
    for d in [".ssh", ".aws", ".gnupg", ".claude"] {
        let path = home.join(d);
        if path.is_dir() {
            let (ok, out) = run(&wt, &mask, &format!("ls -A {} | wc -l", path.display()));
            assert!(ok);
            assert_eq!(out.trim(), "0", "{d} must read as empty");
        }
    }
    assert!(
        !run(
            &wt,
            &mask,
            &format!("ls -A {}", root.join("agentsi.config.json").display())
        )
        .0
    );
    assert!(!run(&wt, &mask, &format!("cat {}", repo.join(".env").display())).0);

    // /run is hidden, so the docker socket is not there.
    assert!(!run(&wt, &mask, "test -e /run/docker.sock").0);

    // No network: neither DNS nor a plain TCP connect.
    assert!(!run(&wt, &mask, "getent hosts example.com").0);
    assert!(!run(&wt, &mask, "curl -sS --max-time 5 http://1.1.1.1/").0);
}

#[test]
fn a_child_that_leaves_the_process_group_does_not_outlive_the_sandbox() {
    if !sandbox::works() {
        eprintln!("skipped: bubblewrap cannot start a sandbox here");
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let dir = tempfile::tempdir_in(exe.parent().unwrap()).unwrap();
    let wt = dir.path().canonicalize().unwrap();
    let beat = wt.join("beat");
    // `setsid` puts the child in a session of its own. It writes a line every 50 ms.
    let command =
        "setsid sh -c 'while :; do echo x >> beat; sleep 0.05; done' >/dev/null 2>&1 & sleep 0.5";
    let r = sandboxed(
        &wt,
        command,
        Duration::from_secs(30),
        &Mask::default(),
        &|_| {},
    );
    assert!(r.ok, "{}{}", r.stdout, r.stderr);
    let size = || std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0);
    let at_exit = size();
    assert!(at_exit > 0, "the child must have run");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        size(),
        at_exit,
        "the child kept running after the sandbox ended"
    );
}
