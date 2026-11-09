//! The push guard, `agentsi hook pre-tool-use`. It must give the verdict of
//! `plugin/hooks/push-guard.mjs` for each command: a verdict that differs is a
//! push that goes through.
//!
//! `push_guard_cases.json` holds the table. The `node-test` rows are the cases
//! of `push-guard.test.mjs`. The `added` rows are new, and the Node guard gave
//! their verdicts (`node-verdicts.mjs` in the U24 report checks the whole file
//! against the Node guard).

use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn hook(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentsi"))
        .arg("hook")
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A hook that refuses its arguments exits before it reads stdin.
    match child.stdin.take().unwrap().write_all(stdin) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
        other => other.unwrap(),
    }
    child.wait_with_output().unwrap()
}

fn pre_tool_use(payload: &Value) -> Output {
    hook(&["pre-tool-use"], payload.to_string().as_bytes())
}

fn cases() -> Vec<Value> {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo"));
    let text = std::fs::read_to_string(dir.join("tests/push_guard_cases.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn each_command_of_the_table_gets_the_verdict_of_the_node_guard() {
    let cases = cases();
    assert!(cases.len() >= 60, "the Node table has 50 cases");
    let added = cases.iter().filter(|c| c["src"] == "added").count();
    assert!(added >= 5, "the table needs 5 added cases, has {added}");
    for case in &cases {
        let command = case["command"].as_str().unwrap();
        let out = pre_tool_use(&json!({"tool_name": "Bash", "tool_input": {"command": command}}));
        assert_eq!(out.status.code(), Some(0), "{command:?}");
        let guarded = case["guarded"].as_bool().unwrap();
        if guarded {
            let answer: Value = serde_json::from_slice(&out.stdout)
                .unwrap_or_else(|_| panic!("{command:?} printed no decision"));
            let out = &answer["hookSpecificOutput"];
            assert_eq!(out["hookEventName"], "PreToolUse", "{command:?}");
            assert_eq!(out["permissionDecision"], "ask", "{command:?}");
            assert!(
                out["permissionDecisionReason"]
                    .as_str()
                    .unwrap()
                    .contains("push"),
                "{command:?}"
            );
        } else {
            assert!(out.stdout.is_empty(), "{command:?} was guarded");
        }
    }
}

#[test]
fn the_decision_has_the_text_of_the_node_guard() {
    let out = pre_tool_use(&json!({"tool_input": {"command": "git push origin main"}}));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        concat!(
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","#,
            r#""permissionDecision":"ask","#,
            r#""permissionDecisionReason":"AgentSI: this command publishes code (push or pull request). Confirm only if the owner asked for it explicitly."}}"#
        )
    );
}

#[test]
fn a_payload_that_is_not_a_bash_command_prints_nothing_and_exits_with_0() {
    let bad: [&[u8]; 6] = [
        b"not json",
        b"",
        b"[]",
        br#"{"tool_input": {"command": 5}}"#,
        br#"{"tool_input": "git status"}"#,
        br#"{"tool_name": "Bash"}"#,
    ];
    for payload in bad {
        let out = hook(&["pre-tool-use"], payload);
        assert_eq!(out.status.code(), Some(0), "{payload:?}");
        assert!(out.stdout.is_empty(), "{payload:?}");
    }
}

#[test]
fn an_unknown_event_is_a_usage_error() {
    let out = hook(&["post-tool-use"], b"{}");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage: agentsi hook pre-tool-use"));
}

/// The Node guard reads the whole payload and decodes bad bytes to U+FFFD.
#[test]
fn a_big_payload_and_bad_utf8_are_guarded_as_the_node_guard_does() {
    let big = format!("echo {}; git push", "a".repeat(9 << 20));
    let out = pre_tool_use(&json!({"tool_input": {"command": big}}));
    assert!(!out.stdout.is_empty(), "a payload over 8 MiB went through");

    let mut bad = br#"{"tool_input":{"command":"echo "#.to_vec();
    bad.push(0xff);
    bad.extend_from_slice(br#"; git push"}}"#);
    assert!(!hook(&["pre-tool-use"], &bad).stdout.is_empty());
}

/// Verdicts of the Node guard (`\s` of JavaScript): U+0085 is no space.
#[test]
fn the_space_characters_are_those_of_javascript() {
    for (sep, guarded) in [("\u{85}", false), ("\u{a0}", true), ("\u{feff}", true)] {
        let command = format!("git{sep}push");
        let out = pre_tool_use(&json!({"tool_input": {"command": command}}));
        assert_eq!(!out.stdout.is_empty(), guarded, "{sep:?}");
    }
}
