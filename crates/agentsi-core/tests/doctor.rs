//! `core.doctor` on a real core: the rows of `agentsi doctor` in the same order. The
//! `PATH` of the core holds only fake tools, so no real `claude` runs.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const NAMES: [&str; 10] = [
    "core",
    "data root",
    "claude",
    "git",
    "bwrap",
    "gh",
    "codebase-memory-mcp",
    "espeak-ng",
    "voice models",
    "audio devices",
];

fn tool(bin: &Path, name: &str, body: &str) {
    let path = bin.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn core_doctor_gives_the_rows_of_agentsi_doctor_in_order() {
    let dir = TempDir::new();
    let root = dir.0.join("root");
    let bin = dir.0.join("bin");
    let home = dir.0.join("home");
    for d in [&root, &bin, &home] {
        std::fs::create_dir_all(d).unwrap();
    }
    tool(
        &bin,
        "claude",
        r#"case "$1" in
  --version) echo "9.9.9 (Claude Code)" ;;
  --help) echo "--safe-mode --restricted --permission-prompts --max-budget-usd --strict-mcp-config --disable-slash-commands" ;;
  *) exit 1 ;;
esac"#,
    );
    tool(&bin, "git", "echo 'git version 2.99.0'");
    let core = CoreProc::start_with(
        &dir,
        &[
            ("AGENTSI_HOME", root.as_path()),
            ("PATH", bin.as_path()),
            ("HOME", home.as_path()),
        ],
    );
    let mut c = Framed::connect(&core.socket.path);
    let r = c.ok("core.doctor", json!({}));
    let checks = r["checks"].as_array().unwrap();
    let names: Vec<&str> = checks.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, NAMES);
    let row = |name: &str| -> &Value { checks.iter().find(|c| c["name"] == name).unwrap() };

    assert_eq!(row("core")["status"], "ok");
    assert_eq!(row("core")["level"], "required");
    assert!(
        row("core")["detail"]
            .as_str()
            .unwrap()
            .starts_with("agentsi-core ")
    );
    assert_eq!(row("data root")["status"], "ok");
    assert!(
        row("data root")["detail"]
            .as_str()
            .unwrap()
            .contains("no database yet")
    );
    assert_eq!(row("claude")["status"], "ok");
    assert_eq!(row("claude")["detail"], "9.9.9 (Claude Code)");
    assert_eq!(row("git")["detail"], "git version 2.99.0");
    assert_eq!(row("bwrap")["level"], "stories");
    assert_eq!(row("bwrap")["status"], "missing");
    assert_eq!(row("bwrap")["detail"], "");
    assert_eq!(row("gh")["level"], "optional");
    assert_eq!(
        row("gh")["blocks"],
        "open issues in the knowledge base and the issue check of the minutes"
    );
    assert_eq!(row("audio devices")["status"], "fail");
    let models = row("voice models");
    assert!(
        models["status"] == "ok" || models["detail"].as_str().unwrap().contains("downloaded"),
        "{models}"
    );
}
