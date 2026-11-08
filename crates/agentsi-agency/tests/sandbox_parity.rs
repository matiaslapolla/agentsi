//! The bwrap arguments of the Rust sandbox equal the ones of
//! `packages/core/src/sandbox.ts` for the same worktree and mask. The test
//! needs `node` and the TypeScript checkout with its `node_modules`. Without
//! them, as in CI and after the switch-over, it prints a line and passes.
//! One difference is allowed and named: the Rust list adds `--unshare-pid` after
//! `--unshare-net` (owner decision, 2026-11-07).

use agentsi_agency::sandbox::{bwrap_args, node_modules_dirs, story_mask};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_bwrap_arguments_equal_the_typescript_ones() {
    if !repo_root().join("packages/core/node_modules").exists() {
        eprintln!("skipped: no TypeScript checkout with node_modules");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let home = base.join("home");
    let root = base.join("data");
    let repo = base.join("repo");
    for d in [".ssh", ".aws", ".config/git", ".claude"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::create_dir_all(&root).unwrap();
    write(&repo.join(".env"), "A=1");
    write(&repo.join("apps/web/.env.local"), "B=2");
    write(&repo.join("node_modules/x/index.js"), "");
    write(&repo.join("node_modules/x/.env"), "skipped");
    write(&repo.join("secrets/real.env"), "C=3");
    std::os::unix::fs::symlink(repo.join("secrets/real.env"), repo.join(".env.link")).unwrap();
    let wt = repo.join(".worktrees/S001-x");
    write(&wt.join("packages/a/node_modules/y/i.js"), "");
    write(&wt.join(".env"), "worktree env stays visible");

    let mask = story_mask(&repo, &home, &root);
    let rust = bwrap_args(
        &wt.to_string_lossy(),
        "pnpm test 'a b'",
        &node_modules_dirs(&wt),
        &mask,
    );

    let script = format!(
        r#"import {{ bwrapArgs, nodeModulesDirs, storyMask }} from "{}";
const [wt, repo, home, root] = process.argv.slice(2);
console.log(JSON.stringify(bwrapArgs(wt, "pnpm test 'a b'", nodeModulesDirs(wt), storyMask(repo, home, root))));"#,
        repo_root().join("packages/core/src/sandbox.ts").display()
    );
    let file = base.join("parity.mts");
    std::fs::write(&file, script).unwrap();
    let out = Command::new("node")
        .args(["--import", "tsx/esm"])
        .arg(&file)
        .args([&wt, &repo, &home, &root])
        .current_dir(repo_root().join("packages/core"))
        .env("AGENTSI_HOME", &root)
        .output()
        .expect("node runs");
    assert!(
        out.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ts: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
    let mut without_pid_ns = rust.clone();
    let at = rust.iter().position(|a| a == "--unshare-pid").unwrap();
    assert_eq!(rust.iter().filter(|a| *a == "--unshare-pid").count(), 1);
    assert_eq!(rust[at - 1], "--unshare-net");
    without_pid_ns.remove(at);
    assert_eq!(without_pid_ns, ts);
    // The mask holds the secret folders, the data root and both .env files of the repo.
    assert!(rust.iter().any(|a| a.ends_with("/home/.ssh")));
    assert!(rust.iter().any(|a| a.ends_with("/secrets/real.env")));
    assert!(!rust.iter().any(|a| a.ends_with("node_modules/x/.env")));
}
