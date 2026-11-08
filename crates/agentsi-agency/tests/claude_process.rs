//! The process layer of a Claude call with a fake `claude` on the PATH of the
//! child: the prompt goes through stdin, the JSON envelope is parsed, a budget
//! stop is a result, output that is not JSON is an error, and a timeout kills
//! the process group.

use agentsi_agency::story::claude::{run_claude, session_result};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

fn fake(dir: &Path, body: &str) -> String {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let file = bin.join("claude");
    std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("{}:/usr/bin:/bin", bin.display())
}

#[test]
fn the_prompt_goes_through_stdin_and_the_envelope_is_parsed() {
    let dir = tempfile::tempdir().unwrap();
    let path = fake(
        dir.path(),
        r#"cat > "$(dirname "$0")/prompt.txt"; echo "$@" > "$(dirname "$0")/args.txt"
echo '{"is_error":false,"result":"done","total_cost_usd":0.12,"duration_ms":3400}'"#,
    );
    let args = vec!["-p".to_string(), "--model".into(), "sonnet".into()];
    let raw = run_claude(
        &args,
        dir.path(),
        "write the tests",
        Duration::from_secs(20),
        &[("PATH", &path)],
        &|_| {},
    )
    .unwrap();
    let r = session_result(&raw, "sonnet").unwrap();
    assert_eq!(
        (r.text.as_str(), r.cost_usd, r.duration_ms, r.budget_stop),
        ("done", 0.12, 3400, false)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("bin/prompt.txt")).unwrap(),
        "write the tests"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("bin/args.txt"))
            .unwrap()
            .trim(),
        "-p --model sonnet"
    );
}

#[test]
fn a_budget_stop_is_a_result_and_text_that_is_not_json_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = fake(
        dir.path(),
        r#"cat >/dev/null; echo '{"is_error":true,"subtype":"error_max_budget_usd","total_cost_usd":0.51}'; exit 1"#,
    );
    let raw = run_claude(
        &[],
        dir.path(),
        "",
        Duration::from_secs(20),
        &[("PATH", &path)],
        &|_| {},
    )
    .unwrap();
    assert!(session_result(&raw, "m").unwrap().budget_stop);

    let other = tempfile::tempdir().unwrap();
    let path = fake(other.path(), "cat >/dev/null; echo 'not json' >&2; exit 3");
    let err = run_claude(
        &[],
        other.path(),
        "",
        Duration::from_secs(20),
        &[("PATH", &path)],
        &|_| {},
    )
    .unwrap_err();
    assert_eq!(err, "claude exited 3 without JSON: not json");
}

#[test]
fn a_timeout_kills_the_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("child-alive");
    let path = fake(
        dir.path(),
        &format!("(sleep 3; touch {}) & sleep 30", marker.display()),
    );
    let start = Instant::now();
    let err = run_claude(
        &[],
        dir.path(),
        "",
        Duration::from_millis(500),
        &[("PATH", &path)],
        &|_| {},
    )
    .unwrap_err();
    assert_eq!(err, "claude timed out after 0s");
    assert!(start.elapsed() < Duration::from_secs(10));
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        !marker.exists(),
        "the child of claude was killed with its group"
    );
}
