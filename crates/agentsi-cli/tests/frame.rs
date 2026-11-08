//! The frame of the CLI: what it does before it reaches the core.

use std::process::{Command, Output};

fn agentsi(args: &[&str], env: &[(&str, &str)]) -> Output {
    let dir = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_agentsi"))
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .env("AGENTSI_CORE_SOCKET", dir.path().join("none/core.sock"))
        // Without this, the CLI starts the `agentsi-core` next to it, and nothing stops that core.
        .env("AGENTSI_CORE_BIN", "/nonexistent/agentsi-core")
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(String::from)
        .collect()
}

#[test]
fn no_command_of_the_help_says_it_is_not_ported() {
    let help = String::from_utf8_lossy(&agentsi(&["help"], &[]).stdout).into_owned();
    for line in help.lines().filter(|l| l.starts_with("  ")) {
        let name = line.split_whitespace().next().unwrap();
        if name == "mcp" {
            continue;
        }
        let out = agentsi(&[name, "--anything"], &[]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("not ported"), "{name}: {err}");
    }
}

#[test]
fn data_import_says_where_it_went_in_one_line_and_fails() {
    let out = agentsi(&["data", "import", "x"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        lines(&out.stderr),
        [
            "agentsi data: Removed. Use `agentsi-core import-postgres` to move data of an old data root."
        ]
    );
}

#[test]
fn every_command_of_the_typescript_cli_is_in_the_help() {
    let out = agentsi(&["help"], &[]);
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    for name in [
        "up", "doctor", "migrate", "data", "backup", "whoami", "desk", "index", "status",
        "snapshot", "check", "audit", "meet", "ask", "decide", "decision", "story", "mark",
        "archive", "restore", "roles", "reset", "reflect", "render", "voice", "web", "mcp",
    ] {
        assert!(
            help.lines()
                .any(|l| l.trim_start().starts_with(&format!("{name} "))),
            "{name}"
        );
    }
}

#[test]
fn a_bad_option_fails_before_the_core_starts() {
    for (args, text) in [
        (
            &["status", "--nope"][..],
            "agentsi status: Unknown option '--nope'",
        ),
        (
            &["audit", "--product"],
            "agentsi audit: Option '--product <value>' argument missing",
        ),
        (
            &["snapshot", "extra"],
            "agentsi snapshot: Unexpected argument 'extra'",
        ),
    ] {
        let out = agentsi(args, &[]);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).starts_with(text),
            "{args:?}"
        );
    }
}

#[test]
fn without_a_core_program_a_command_says_what_to_set() {
    let out = agentsi(
        &["status", "--product", "x"],
        &[("AGENTSI_CORE_BIN", "/does/not/exist")],
    );
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(err.starts_with("agentsi status: "), "{err}");
    assert!(err.contains("/does/not/exist"), "{err}");
}
